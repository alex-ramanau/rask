//! ack's `MAIN` block: from the command line to the exit code.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::sync::Arc;

use crate::bytes::{Bytes, from_os, lossy, to_path};
use crate::cli::getopt::{self, Config, Spec};
use crate::config::{Opt, Processed};
use crate::filter::{Collection, File, FilterContext};
use crate::matcher::PcreMatcher;
use crate::matcher::build::{RegexOpts, build_all_regexes, build_regex};
use crate::output::color::Color;
use crate::output::die;
use crate::search::{Mode, OutputTemplate, Searcher, Settings};

fn compile(qr: &[u8]) -> PcreMatcher {
    PcreMatcher::from_bytes(qr)
        .unwrap_or_else(|e| die(&format!("Invalid regex '{}': {e}", lossy(qr))))
}

/// `/^--th[pt]+t+$/`: two or more of `p` and `t`, ending in `t`.
fn is_thpppt(arg: &[u8]) -> bool {
    arg.strip_prefix(b"--th").is_some_and(|rest| {
        rest.len() >= 2 && rest.iter().all(|c| matches!(c, b'p' | b't')) && rest.ends_with(b"t")
    })
}

/// A (fairly) unique identifier for a file (`get_file_id`).
fn get_file_id(name: &[u8]) -> Bytes {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(m) = std::fs::metadata(to_path(name)) {
            return format!("{}:{}", m.dev(), m.ino()).into_bytes();
        }
        name.to_vec()
    }
    #[cfg(not(unix))]
    {
        crate::walk::reslash(name)
    }
}

/// `-p STDIN`: is stdin a pipe?
fn stdin_is_pipe() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata("/dev/stdin")
            .map(|m| m.file_type().is_fifo())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        !std::io::stdin().is_terminal()
    }
}

fn color_for(opt_value: &Option<Bytes>, var: &str, default: &str) -> Color {
    let spec = opt_value
        .clone()
        .or_else(|| crate::env::var(var).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| default.as_bytes().to_vec());
    Color::new(&spec).unwrap_or_else(|e| die(&e))
}

/// `File::Spec->splitdir` on Unix: split on `/`, keeping empty fields.
fn splitdir(dir: &[u8]) -> Vec<&[u8]> {
    dir.split(|&c| c == b'/').collect()
}

/// Runs ack with the process's arguments, and exits.
pub fn run() -> ! {
    let mut argv: Vec<Bytes> = std::env::args_os().skip(1).map(|a| from_os(&a)).collect();

    // Do preliminary arg checking.
    let mut env_is_usable = true;
    for arg in &argv {
        if arg == b"--" {
            break;
        }
        // Get the --thpppt, --bar, --cathy checking out of the way.
        if is_thpppt(arg) {
            crate::help::thpppt(arg);
        }
        match arg.as_slice() {
            b"--bar" => crate::help::ackbar(),
            b"--cathy" => crate::help::cathy(),
            b"--env" => env_is_usable = true,
            b"--noenv" => env_is_usable = false,
            _ => {}
        }
    }
    if env_is_usable {
        if crate::env::var("ACK_OPTIONS").is_some_and(|v| !v.is_empty() && v != b"0") {
            crate::output::warn(
                "WARNING: ack no longer uses the ACK_OPTIONS environment variable.  Use an ackrc file instead.",
            );
        }
    } else {
        crate::env::ignore_ack_variables();
    }

    #[derive(Clone)]
    enum Early {
        Help,
        Version,
        Man,
    }
    let mut early = None;
    getopt::get_options(
        &mut argv,
        &[
            Spec::new("help", Early::Help),
            Spec::new("version", Early::Version),
            Spec::new("man", Early::Man),
        ],
        Config::ack(true, true),
        |act, _, _| {
            if early.is_none() {
                early = Some(act.clone());
            }
            Ok(())
        },
    );
    match early {
        Some(Early::Help) => {
            crate::help::show_help();
            crate::output::exit(0);
        }
        Some(Early::Version) => {
            crate::output::print(&[crate::help::version_text().as_bytes()]);
            crate::output::exit(0);
        }
        Some(Early::Man) => {
            crate::help::show_man();
            crate::output::exit(0);
        }
        None => {}
    }

    if argv.is_empty() {
        crate::help::show_help();
        crate::output::exit(1);
    }

    let sources = crate::config::retrieve_arg_sources(&mut argv);
    let Processed {
        mut opt,
        types,
        argv: mut args,
    } = crate::config::process_args(sources);

    if opt.show_types && !(opt.f || opt.g) {
        die("--show-types can only be used with -f or -g.");
    }

    let range = |r: &Option<Bytes>| -> Option<PcreMatcher> {
        let r = r
            .as_ref()
            .filter(|r| !r.is_empty() && r.as_slice() != b"0")?;
        match build_regex(r, &RegexOpts::default()) {
            Ok((qr, _)) => Some(compile(&qr)),
            Err(e) => die(&e),
        }
    };
    let range_start = range(&opt.range_start);
    let range_end = range(&opt.range_end);

    let report_bad_filenames = !opt.s;
    let output_to_pipe = !std::io::stdout().is_terminal();

    let color = match opt.color {
        Some(c) => c,
        None => !opt.g && !output_to_pipe,
    };
    let heading = opt.heading.unwrap_or(!output_to_pipe);
    let break_ = opt.break_.unwrap_or(!output_to_pipe);

    let is_filter_mode = opt.filter_mode.unwrap_or_else(stdin_is_pipe);

    let build = |regex: &[u8], opt: &Opt| match build_all_regexes(
        regex,
        &opt.and,
        &opt.or,
        &opt.not,
        &opt.regex_opts,
    ) {
        Ok(all) => all,
        Err(e) => die(&e),
    };

    let mut show_filename: Option<bool> = None;
    if opt.big_h.is_some() || opt.h.is_some() {
        show_filename = Some(opt.big_h == Some(true) && opt.h != Some(true));
    }

    let mut regexes = None;
    let mut start: Vec<Bytes> = Vec::new();
    let use_stdin = is_filter_mode && opt.files_from.is_none();
    if use_stdin || !opt.f {
        let regex = opt
            .regex
            .clone()
            .or_else(|| (!args.is_empty()).then(|| args.remove(0)));
        let Some(regex) = regex else {
            die("No regular expression found.")
        };
        regexes = Some(build(&regex, &opt));
    }
    if !use_stdin {
        if opt.files_from.is_none() {
            start = args.clone();
        }
        // Show filenames unless searching exactly one file.
        if show_filename.is_none() && (start.len() != 1 || to_path(&start[0]).is_dir()) {
            show_filename = Some(true);
        }
    }

    let settings = Settings {
        re_match: regexes.as_ref().map(|r| compile(&r.re_match)),
        re_not: regexes
            .as_ref()
            .and_then(|r| r.re_not.as_ref())
            .map(|q| compile(q)),
        re_hilite: regexes.as_ref().map(|r| compile(&r.re_hilite)),
        re_scan: regexes
            .as_ref()
            .and_then(|r| r.re_scan.as_ref())
            .map(|q| compile(q)),
        range_start,
        range_end,
        one: opt.one,
        a: opt.a.unwrap_or(0).max(0) as usize,
        b: opt.b.unwrap_or(0).max(0) as usize,
        break_,
        c: opt.c,
        color,
        column: opt.column,
        f: opt.f,
        g: opt.g,
        heading,
        l: opt.l,
        big_l: opt.big_l,
        m: opt.m,
        output: opt.output.as_ref().map(|o| OutputTemplate::new(o)),
        passthru: opt.passthru,
        p: opt.p,
        show_filename: show_filename.unwrap_or(false),
        show_types: opt.show_types.then_some(types),
        underline: opt.underline,
        v: opt.v,
        report_bad_filenames,
        ors: if opt.print0 { b"\0" } else { b"\n" },
        color_match: color_for(&opt.colors.match_, "ACK_COLOR_MATCH", "black on_yellow"),
        color_filename: color_for(&opt.colors.filename, "ACK_COLOR_FILENAME", "bold green"),
        color_lineno: color_for(&opt.colors.lineno, "ACK_COLOR_LINENO", "bold yellow"),
        color_colno: color_for(&opt.colors.colno, "ACK_COLOR_COLNO", "bold yellow"),
    };
    let g_match = if opt.g {
        settings
            .re_match
            .as_ref()
            .map(|_| regexes.as_ref().unwrap().re_match.clone())
    } else {
        None
    };
    let g_not = if opt.g {
        regexes.as_ref().and_then(|r| r.re_not.clone())
    } else {
        None
    };
    let debug_regexes = regexes.clone();
    let debug = opt.debug;
    let opt_pager = opt.pager.clone();

    let warn_handler = move |msg: &str| {
        if report_bad_filenames {
            crate::output::warn(msg);
        }
    };

    // Set up the file stream: a list of files, or a directory walk.
    enum Input {
        Files(Box<dyn Iterator<Item = File>>),
        Walk {
            make_walker: Box<dyn FnOnce() -> crate::walk::Files<'static> + Send>,
            selector: Selector,
        },
    }
    let input = if use_stdin {
        Input::Files(Box::new(std::iter::once(File::new(b"-".to_vec()))))
    } else if let Some(from) = &opt.files_from {
        match crate::walk::from_file(from, Box::new(warn_handler)) {
            Ok(f) => Input::Files(Box::new(f)),
            Err(msg) => {
                warn_handler(&msg);
                crate::output::exit(1);
            }
        }
    } else {
        if start.is_empty() {
            start.push(b".".to_vec());
        }
        for target in &start {
            if report_bad_filenames
                && std::fs::symlink_metadata(to_path(target)).is_err()
                && !to_path(target).exists()
            {
                crate::output::warn(&format!("{}: No such file or directory", lossy(target)));
            }
        }
        let opt = std::mem::take(&mut opt);
        let idirs = opt.idirs.clone();
        let selector = file_selector(
            FilterOpts {
                filters: opt.filters,
                ifiles: opt.ifiles,
                idirs: opt.idirs,
            },
            &start,
            g_match,
            g_not,
            opt.v,
            report_bad_filenames,
        );
        let (no_recurse, sort_files, follow_symlinks) = (opt.n, opt.sort_files, opt.follow);
        let starts = start.clone();
        let make_walker = move || {
            let descend_filter = if no_recurse {
                Some(Box::new(|_: &[u8]| false) as crate::walk::DescendFilter)
            } else {
                compile_descend_filter(&idirs, report_bad_filenames)
            };
            let warn = move |msg: &str| {
                if report_bad_filenames {
                    crate::output::warn(msg);
                }
            };
            crate::walk::Files::new(
                crate::walk::WalkOptions {
                    file_filter: None,
                    descend_filter,
                    error_handler: Box::new(warn),
                    sort_files,
                    follow_symlinks,
                },
                &starts,
            )
        };
        Input::Walk {
            make_walker: Box::new(make_walker),
            selector,
        }
    };

    if let Some(pager) = &opt_pager {
        crate::output::set_up_pager(pager);
    }

    let settings = Arc::new(settings);
    let mode = if settings.f || settings.g {
        Mode::List
    } else if settings.c {
        Mode::Count { bail: false }
    } else if settings.l || settings.big_l {
        Mode::Count { bail: true }
    } else {
        Mode::Search
    };
    let threads = crate::parallel::threads();
    let mut prepared: Box<dyn Iterator<Item = crate::search::Prepared>> = match input {
        Input::Walk {
            make_walker,
            selector,
        } if threads > 1 => Box::new(crate::parallel::run(
            make_walker,
            selector,
            settings.clone(),
            mode,
            threads,
        )),
        Input::Walk {
            make_walker,
            mut selector,
        } => {
            use crate::parallel::Select;
            let mut walker = make_walker();
            let settings = settings.clone();
            Box::new(std::iter::from_fn(move || {
                loop {
                    let entry = walker.next_entry()?;
                    if let Some(file) = selector.select(&entry) {
                        return Some(settings.prepare(file.regular(entry.is_file), mode));
                    }
                }
            }))
        }
        Input::Files(files) => {
            let settings = settings.clone();
            Box::new(files.map(move |f| settings.prepare(f, mode)))
        }
    };
    let mut searcher = Searcher::new(settings);
    let nmatches = match mode {
        Mode::List => searcher.file_loop_fg(&mut prepared),
        Mode::Count { bail: false } => searcher.file_loop_c(&mut prepared),
        Mode::Count { bail: true } => searcher.file_loop_l(&mut prepared),
        Mode::Search => searcher.file_loop_normal(&mut prepared),
    };
    drop(prepared);

    if debug {
        let show = |q: Option<&Bytes>| q.map(|q| lossy(q)).unwrap_or_else(|| "undef".into());
        let r = debug_regexes.as_ref();
        let st = &searcher.stats;
        let rows = [
            ("re_match", show(r.map(|r| &r.re_match))),
            ("re_scan", show(r.and_then(|r| r.re_scan.as_ref()))),
            ("re_not", show(r.and_then(|r| r.re_not.as_ref()))),
            ("prescans", st.prescans.to_string()),
            ("linescans", st.linescans.to_string()),
            ("filematches", st.filematches.to_string()),
            ("linematches", st.linematches.to_string()),
        ];
        for (name, value) in rows {
            crate::output::warn(&format!("{name:<11} = {value}"));
        }
    }

    crate::output::exit(if nmatches > 0 { 0 } else { 1 });
}

/// The pieces of `$opt` that the file filter needs.
struct FilterOpts {
    filters: Vec<crate::filter::FilterRef>,
    ifiles: Option<Collection>,
    idirs: Vec<crate::config::IgnoreDir>,
}

/// `_compile_descend_filter`
fn compile_descend_filter(
    idirs: &[crate::config::IgnoreDir],
    report_bad_filenames: bool,
) -> Option<crate::walk::DescendFilter<'static>> {
    // With any --noignore-dir, whole hierarchies can't be skipped; the file
    // filter looks at each file's directories instead.
    if idirs.iter().any(|d| d.inverted) || idirs.is_empty() {
        return None;
    }
    let collections: Vec<_> = idirs.iter().map(|d| d.collection.clone()).collect();
    let ctx = FilterContext {
        report_bad_filenames,
    };
    Some(Box::new(move |dir: &[u8]| {
        let file = File::new(dir.to_vec());
        !collections
            .iter()
            .any(|c| c.lock().unwrap().matches(&file, &ctx))
    }))
}

/// The file filter (`_compile_file_filter`): which walked files are searched.
struct SelectorData {
    ctx: FilterContext,
    direct: Collection,
    inverse: Collection,
    has_inverse: bool,
    starting_set: HashSet<Bytes>,
    starts_include_files: bool,
    dont_ignore_dir_filter: bool,
    idirs: Vec<crate::config::IgnoreDir>,
    ifiles: Option<Collection>,
    g_match: Option<PcreMatcher>,
    g_not: Option<PcreMatcher>,
    opt_v: bool,
}

/// The file filter, with the per-thread cache of the last directory's
/// `--noignore-dir` result.
#[derive(Clone)]
struct Selector {
    data: Arc<SelectorData>,
    previous_dir: Bytes,
    previous_dir_ignore_result: bool,
}

impl crate::parallel::Select for Selector {
    fn select(&mut self, entry: &crate::walk::Entry) -> Option<File> {
        let d = self.data.clone();
        use crate::matcher::Matcher;
        let name = &entry.fullpath;
        if let Some(g) = &d.g_match {
            let mut is_match = g.is_match(name);
            if is_match && let Some(not) = &d.g_not {
                is_match = !not.is_match(name);
            }
            if is_match == d.opt_v {
                return None;
            }
        }
        // ack always selects files that are specified on the command line,
        // regardless of filetype.
        if d.starts_include_files && d.starting_set.contains(&get_file_id(name)) {
            return Some(File::new(name.clone()));
        }

        if d.dont_ignore_dir_filter {
            let dir = entry.dirname.clone().unwrap_or_default();
            if self.previous_dir == dir {
                if self.previous_dir_ignore_result {
                    return None;
                }
            } else {
                let dirs = splitdir(&dir);
                let mut is_ignoring = false;
                for i in 0..dirs.len() {
                    let prefix = dirs[..=i].join(&b"/"[..]);
                    let dir_file = File::new(crate::walk::canonpath(&prefix));
                    for idir in &d.idirs {
                        if idir.collection.lock().unwrap().matches(&dir_file, &d.ctx) {
                            is_ignoring = !idir.inverted;
                        }
                    }
                }
                self.previous_dir = dir;
                self.previous_dir_ignore_result = is_ignoring;
                if is_ignoring {
                    return None;
                }
            }
        }

        // Ignore named pipes found in directory searching.
        if entry.is_named_pipe {
            return None;
        }
        // We can't handle unreadable filenames; report them. Perl checks the
        // mode bits (-r _) and then access(2) (-R); a file that opens is
        // readable by both, so they're only consulted when the open fails.
        // The open handle is kept for the -T test and the search.
        let path = to_path(name);
        let file = match std::fs::File::open(&path) {
            Ok(handle) => File::with_handle(name.clone(), handle),
            Err(_) => {
                let meta = std::fs::metadata(&path).ok();
                if !crate::filetest::readable(meta.as_ref())
                    && !crate::filetest::access_readable(&path)
                {
                    if d.ctx.report_bad_filenames {
                        crate::output::warn(&format!(
                            "{}: cannot open file for reading",
                            lossy(name)
                        ));
                    }
                    return None;
                }
                File::new(name.clone())
            }
        };

        if let Some(ifiles) = &d.ifiles
            && ifiles.matches(&file, &d.ctx)
        {
            return None;
        }
        let mut match_found = d.direct.matches(&file, &d.ctx);
        // Don't bother invoking inverse filters unless we consider the current file a match.
        if match_found && d.has_inverse && d.inverse.matches(&file, &d.ctx) {
            match_found = false;
        }
        match_found.then_some(file)
    }
}

/// `_compile_file_filter`
fn file_selector(
    fo: FilterOpts,
    start: &[Bytes],
    g_match: Option<Bytes>,
    g_not: Option<Bytes>,
    opt_v: bool,
    report_bad_filenames: bool,
) -> Selector {
    let ctx = FilterContext {
        report_bad_filenames,
    };
    let mut direct = Collection::default();
    let mut inverse = Collection::default();
    let mut has_inverse = false;
    for f in &fo.filters {
        if f.inverted {
            // We want to check if files match the uninverted filters.
            inverse.add(f.filter.clone());
            has_inverse = true;
        } else {
            direct.add(f.filter.clone());
        }
    }
    let starting_set: HashSet<Bytes> = start.iter().map(|s| get_file_id(s)).collect();
    // A walked file can only be in the starting set if a start isn't a
    // directory; otherwise the stat for its id can be skipped.
    let starts_include_files = start.iter().any(|s| !to_path(s).is_dir());
    let dont_ignore_dir_filter = fo.idirs.iter().any(|d| d.inverted);
    let idirs = fo.idirs;
    let ifiles = fo.ifiles;
    let g_match = g_match.map(|q| compile(&q));
    let g_not = g_not.map(|q| compile(&q));
    Selector {
        data: Arc::new(SelectorData {
            ctx,
            direct,
            inverse,
            has_inverse,
            starting_set,
            starts_include_files,
            dont_ignore_dir_filter,
            idirs,
            ifiles,
            g_match,
            g_not,
            opt_v,
        }),
        previous_dir: Vec::new(),
        previous_dir_ignore_result: false,
    }
}
