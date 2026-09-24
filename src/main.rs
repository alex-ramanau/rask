//! rask: a Rust reimplementation of ack 3.
//!
//! Perl ack 3.10.0 is the specification: its test suite is run against this
//! binary by `scripts/ack-suite`. `main` follows the `MAIN` block of ack.

mod bytes;
mod cli;
mod config;
mod env;
mod filter;
mod help;
mod matcher;
mod output;
mod search;
mod walk;

use std::collections::HashSet;
use std::io::IsTerminal;

use bytes::{Bytes, from_os, lossy, to_path};
use cli::getopt::{self, Config, Spec};
use config::{Opt, Processed};
use filter::{Collection, File, FilterContext};
use matcher::PcreMatcher;
use matcher::build::{RegexOpts, build_all_regexes, build_regex};
use output::color::Color;
use output::die;
use search::{OutputTemplate, Searcher, Settings};

/// Version of Perl ack whose behaviour we reproduce.
pub const ACK_VERSION: &str = "3.10.0";

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
        walk::reslash(name)
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
        .or_else(|| env::var(var).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| default.as_bytes().to_vec());
    Color::new(&spec).unwrap_or_else(|e| die(&e))
}

/// `File::Spec->splitdir` on Unix: split on `/`, keeping empty fields.
fn splitdir(dir: &[u8]) -> Vec<&[u8]> {
    dir.split(|&c| c == b'/').collect()
}

fn main() {
    let mut argv: Vec<Bytes> = std::env::args_os().skip(1).map(|a| from_os(&a)).collect();

    // Do preliminary arg checking.
    let mut env_is_usable = true;
    for arg in &argv {
        if arg == b"--" {
            break;
        }
        // Get the --thpppt, --bar, --cathy checking out of the way.
        if is_thpppt(arg) {
            help::thpppt(arg);
        }
        match arg.as_slice() {
            b"--bar" => help::ackbar(),
            b"--cathy" => help::cathy(),
            b"--env" => env_is_usable = true,
            b"--noenv" => env_is_usable = false,
            _ => {}
        }
    }
    if env_is_usable {
        if env::var("ACK_OPTIONS").is_some_and(|v| !v.is_empty() && v != b"0") {
            output::warn(
                "WARNING: ack no longer uses the ACK_OPTIONS environment variable.  Use an ackrc file instead.",
            );
        }
    } else {
        env::ignore_ack_variables();
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
            help::show_help();
            output::exit(0);
        }
        Some(Early::Version) => {
            output::print(&[help::version_text().as_bytes()]);
            output::exit(0);
        }
        Some(Early::Man) => {
            help::show_man();
            output::exit(0);
        }
        None => {}
    }

    if argv.is_empty() {
        help::show_help();
        output::exit(1);
    }

    let sources = config::retrieve_arg_sources(&mut argv);
    let Processed {
        mut opt,
        types,
        argv: mut args,
    } = config::process_args(sources);

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

    let warn_handler = |msg: &str| {
        if report_bad_filenames {
            output::warn(msg);
        }
    };

    // Set up the file stream.
    let mut files: Box<dyn Iterator<Item = Bytes>> = if use_stdin {
        Box::new(std::iter::once(b"-".to_vec()))
    } else if let Some(from) = &opt.files_from {
        match walk::from_file(from, Box::new(warn_handler)) {
            Ok(f) => Box::new(f),
            Err(msg) => {
                warn_handler(&msg);
                output::exit(1);
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
                output::warn(&format!("{}: No such file or directory", lossy(target)));
            }
        }
        let opt = std::mem::take(&mut opt);
        let descend_filter = compile_descend_filter(&opt.idirs, report_bad_filenames);
        let file_filter = compile_file_filter(
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
        let descend_filter = if opt.n {
            Some(Box::new(|_: &[u8]| false) as walk::DescendFilter)
        } else {
            descend_filter
        };
        Box::new(walk::Files::new(
            walk::WalkOptions {
                file_filter: Some(file_filter),
                descend_filter,
                error_handler: Box::new(warn_handler),
                sort_files: opt.sort_files,
                follow_symlinks: opt.follow,
            },
            &start,
        ))
    };

    if let Some(pager) = &opt_pager {
        output::set_up_pager(pager);
    }

    let mut searcher = Searcher::new(settings);
    let nmatches = if searcher.s.f || searcher.s.g {
        searcher.file_loop_fg(&mut files)
    } else if searcher.s.c {
        searcher.file_loop_c(&mut files)
    } else if searcher.s.l || searcher.s.big_l {
        searcher.file_loop_l(&mut files)
    } else {
        searcher.file_loop_normal(&mut files)
    };
    drop(files);

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
            output::warn(&format!("{name:<11} = {value}"));
        }
    }

    output::exit(if nmatches > 0 { 0 } else { 1 });
}

/// The pieces of `$opt` that the file filter needs.
struct FilterOpts {
    filters: Vec<filter::FilterRef>,
    ifiles: Option<Collection>,
    idirs: Vec<config::IgnoreDir>,
}

/// `_compile_descend_filter`
fn compile_descend_filter<'a>(
    idirs: &[config::IgnoreDir],
    report_bad_filenames: bool,
) -> Option<walk::DescendFilter<'a>> {
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
        !collections.iter().any(|c| c.borrow().matches(&file, &ctx))
    }))
}

/// `_compile_file_filter`
fn compile_file_filter<'a>(
    fo: FilterOpts,
    start: &[Bytes],
    g_match: Option<Bytes>,
    g_not: Option<Bytes>,
    opt_v: bool,
    report_bad_filenames: bool,
) -> walk::FileFilter<'a> {
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
    let dont_ignore_dir_filter = fo.idirs.iter().any(|d| d.inverted);
    let idirs = fo.idirs;
    let ifiles = fo.ifiles;
    let g_match = g_match.map(|q| compile(&q));
    let g_not = g_not.map(|q| compile(&q));
    let mut previous_dir: Bytes = Vec::new();
    let mut previous_dir_ignore_result = false;

    Box::new(move |entry: &walk::Entry| {
        use matcher::Matcher;
        let name = &entry.fullpath;
        if let Some(g) = &g_match {
            let mut is_match = g.is_match(name);
            if is_match && let Some(not) = &g_not {
                is_match = !not.is_match(name);
            }
            if is_match == opt_v {
                return false;
            }
        }
        // ack always selects files that are specified on the command line,
        // regardless of filetype.
        if starting_set.contains(&get_file_id(name)) {
            return true;
        }

        if dont_ignore_dir_filter {
            let dir = entry.dirname.clone().unwrap_or_default();
            if previous_dir == dir {
                if previous_dir_ignore_result {
                    return false;
                }
            } else {
                let dirs = splitdir(&dir);
                let mut is_ignoring = false;
                for i in 0..dirs.len() {
                    let prefix = dirs[..=i].join(&b"/"[..]);
                    let dir_file = File::new(walk::canonpath(&prefix));
                    for d in &idirs {
                        if d.collection.borrow().matches(&dir_file, &ctx) {
                            is_ignoring = !d.inverted;
                        }
                    }
                }
                previous_dir = dir;
                previous_dir_ignore_result = is_ignoring;
                if is_ignoring {
                    return false;
                }
            }
        }

        // Ignore named pipes found in directory searching.
        let path = to_path(name);
        if std::fs::metadata(&path)
            .map(|m| is_fifo(&m))
            .unwrap_or(false)
        {
            return false;
        }
        // We can't handle unreadable filenames; report them.
        if let Err(e) = std::fs::File::open(&path)
            && e.kind() == std::io::ErrorKind::PermissionDenied
        {
            if report_bad_filenames {
                output::warn(&format!("{}: cannot open file for reading", lossy(name)));
            }
            return false;
        }

        let file = File::new(name.clone());
        if let Some(ifiles) = &ifiles
            && ifiles.matches(&file, &ctx)
        {
            return false;
        }
        let mut match_found = direct.matches(&file, &ctx);
        // Don't bother invoking inverse filters unless we consider the current file a match.
        if match_found && has_inverse && inverse.matches(&file, &ctx) {
            match_found = false;
        }
        match_found
    })
}

#[cfg(unix)]
fn is_fifo(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    m.file_type().is_fifo()
}

#[cfg(not(unix))]
fn is_fifo(_: &std::fs::Metadata) -> bool {
    false
}
