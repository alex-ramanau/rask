//! Option sources and option processing: a port of `App::Ack::ConfigLoader`
//! and `App::Ack::ConfigFinder` (in `finder.rs`).
//!
//! Options come from a list of sources: the built-in defaults, ackrc files,
//! and the command line. Each source is run through several Getopt::Long
//! passes with different configurations, in the same order as Perl ack.

mod finder;
mod mutex;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::bytes::{Bytes, errno_text, lossy, to_path};
use crate::cli::getopt::{self, Config, HandlerError, Spec, Value};
use crate::filter::{Collection, Filter, FilterRc, FilterRef};
use crate::matcher::build::RegexOpts;
use crate::output::{self, die, die_text};

/// The built-in ackrc (`App::Ack::ConfigDefault::options()`), synced from ack3
/// by `scripts/sync-from-ack3`.
pub const DEFAULT_ACKRC: &str = include_str!("default_ackrc.txt");

/// `App::Ack::ConfigDefault::options_clean()`
pub fn default_options() -> Vec<Bytes> {
    DEFAULT_ACKRC
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.as_bytes().to_vec())
        .collect()
}

/// One source of options.
pub struct Source {
    pub name: String,
    pub contents: Vec<Bytes>,
    pub is_ackrc: bool,
    pub project: bool,
}

/// An `--ignore-dir`/`--noignore-dir` collection, possibly inverted.
pub struct IgnoreDir {
    pub collection: Rc<RefCell<Collection>>,
    pub inverted: bool,
}

/// Colours from `--color-*` options (Perl ack stores them in `%ENV`).
#[derive(Default, Clone)]
pub struct ColorOpts {
    pub match_: Option<Bytes>,
    pub filename: Option<Bytes>,
    pub lineno: Option<Bytes>,
    pub colno: Option<Bytes>,
}

/// The `$opt` hash.
#[derive(Default)]
pub struct Opt {
    pub one: bool,
    pub m: Option<i64>,
    pub a: Option<i64>,
    pub b: Option<i64>,
    pub break_: Option<bool>,
    pub c: bool,
    pub color: Option<bool>,
    pub column: bool,
    pub debug: bool,
    pub f: bool,
    pub files_from: Option<Bytes>,
    pub filter_mode: Option<bool>,
    pub follow: bool,
    pub g: bool,
    pub heading: Option<bool>,
    pub h: Option<bool>,
    pub big_h: Option<bool>,
    pub regex_opts: RegexOpts,
    pub l: bool,
    pub big_l: bool,
    pub regex: Option<Bytes>,
    pub n: bool,
    pub output: Option<Bytes>,
    pub pager: Option<Bytes>,
    pub and: Vec<Bytes>,
    pub or: Vec<Bytes>,
    pub not: Vec<Bytes>,
    pub passthru: bool,
    pub print0: bool,
    pub p: Option<i64>,
    pub range_start: Option<Bytes>,
    pub range_end: Option<Bytes>,
    pub range_invert: bool,
    pub s: bool,
    pub show_types: bool,
    pub sort_files: bool,
    pub underline: bool,
    pub v: bool,
    pub noenv_seen: bool,
    pub colors: ColorOpts,

    pub filters: Vec<FilterRef>,
    pub idirs: Vec<IgnoreDir>,
    pub ifiles: Option<Collection>,
}

/// File types: `%App::Ack::mappings`, and which `--TYPE` options exist.
#[derive(Default)]
pub struct Types {
    pub mappings: HashMap<String, Vec<FilterRc>>,
    pub specs: BTreeSet<String>,
}

/// Perl's `split /,/, $s`: trailing empty fields are dropped.
fn split_commas(s: &[u8]) -> Vec<Bytes> {
    let mut parts: Vec<Bytes> = s.split(|&c| c == b',').map(<[u8]>::to_vec).collect();
    while parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

fn split_once(s: &[u8], c: u8) -> (Bytes, Option<Bytes>) {
    match s.iter().position(|&b| b == c) {
        Some(i) => (s[..i].to_vec(), Some(s[i + 1..].to_vec())),
        None => (s.to_vec(), None),
    }
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `_process_filter_spec`: `name:type:args` or the ack 1 style `name=.ext,.ext`.
fn process_filter_spec(spec: &[u8]) -> Result<(String, Filter), String> {
    let word_len = spec.iter().take_while(|&&c| is_word(c)).count();
    let (name, rest) = spec.split_at(word_len);
    if word_len > 0 && rest.first() == Some(&b':') {
        let rest = &rest[1..];
        let type_len = rest.iter().take_while(|&&c| is_word(c)).count();
        if type_len > 0 && rest.get(type_len) == Some(&b':') {
            let args = &rest[type_len + 1..];
            // (.*) stops at a newline.
            let args = &args[..args.iter().position(|&c| c == b'\n').unwrap_or(args.len())];
            let filter = Filter::create(&rest[..type_len], &split_commas(args))?;
            return Ok((lossy(name), filter));
        }
    }
    if word_len > 0 && rest.first() == Some(&b'=') {
        let exts: Vec<Bytes> = split_commas(&rest[1..])
            .into_iter()
            .map(|e| e.strip_prefix(b".").map(<[u8]>::to_vec).unwrap_or(e))
            .collect();
        return Ok((lossy(name), Filter::create(b"ext", &exts)?));
    }
    Err(die_text(&format!(
        "Invalid filter specification '{}'",
        lossy(spec)
    )))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeAct {
    Add,
    Set,
    Del,
}

/// `_process_filetypes`: pulls `--type-add`, `--type-set` and `--type-del`
/// out of every source.
fn process_filetypes(sources: &mut [Source]) -> Types {
    let mut types = Types::default();
    let specs = [
        Spec::new("type-add=s", TypeAct::Add),
        Spec::new("type-set=s", TypeAct::Set),
        Spec::new("type-del=s", TypeAct::Del),
    ];
    for source in sources.iter_mut() {
        getopt::get_options(
            &mut source.contents,
            &specs,
            Config::ack(true, true),
            |act, _, v| {
                let v = v.into_bytes();
                match act {
                    TypeAct::Add | TypeAct::Set => {
                        let (name, filter) = process_filter_spec(&v).map_err(HandlerError::Died)?;
                        let filter = Rc::new(filter);
                        if *act == TypeAct::Add {
                            types.mappings.entry(name.clone()).or_default().push(filter);
                        } else {
                            types.mappings.insert(name.clone(), vec![filter]);
                        }
                        types.specs.insert(name);
                    }
                    TypeAct::Del => {
                        let name = lossy(&v);
                        types.mappings.remove(&name);
                        types.specs.remove(&name);
                    }
                }
                Ok(())
            },
        );
    }
    types
}

/// The actions behind `get_arg_spec`'s option table.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Act {
    One,
    And,
    AfterContext,
    BeforeContext,
    Context,
    Break,
    Count,
    Color,
    ColorMatch,
    ColorFilename,
    ColorColno,
    ColorLineno,
    Column,
    CreateAckrc,
    Debug,
    Env,
    F,
    FilesFrom,
    Filter,
    Flush,
    Follow,
    G,
    Group,
    Heading,
    NoFilename,
    WithFilename,
    IgnoreCase,
    NoIgnoreCase,
    IgnoreDir,
    IgnoreFile,
    FilesWithMatches,
    FilesWithoutMatches,
    MaxCount,
    Match,
    NoRecurse,
    O,
    Output,
    Pager,
    NoIgnoreDir,
    NoPager,
    Not,
    Or,
    Passthru,
    Print0,
    Proximate,
    NoProximate,
    Literal,
    Recurse,
    RangeStart,
    RangeEnd,
    RangeInvert,
    S,
    ShowTypes,
    SmartCase,
    SortFiles,
    Type,
    NoType,
    Underline,
    InvertMatch,
    WordRegexp,
    X,
    Help,
    HelpTypes,
    HelpColors,
    HelpRgb,
    /// `--TYPE` / `--noTYPE`.
    TypeToggle(String),
    KnownTypes,
    /// An option that's forbidden in this source; the message says where.
    Forbidden(&'static str),
}

/// `get_arg_spec`, plus the per-type specs from `_process_filetypes`.
fn arg_spec(types: &Types) -> Vec<Spec<Act>> {
    use Act::*;
    let mut specs = vec![
        Spec::new("1", One),
        Spec::new("and=s", And),
        Spec::new("A|after-context:-1", AfterContext),
        Spec::new("B|before-context:-1", BeforeContext),
        Spec::new("C|context:-1", Context),
        Spec::new("break!", Break),
        Spec::new("c|count", Count),
        Spec::new("color|colour!", Color),
        Spec::new("color-match=s", ColorMatch),
        Spec::new("color-filename=s", ColorFilename),
        Spec::new("color-colno=s", ColorColno),
        Spec::new("color-lineno=s", ColorLineno),
        Spec::new("column!", Column),
        Spec::new("create-ackrc", CreateAckrc),
        Spec::new("debug", Debug),
        Spec::new("env!", Env),
        Spec::new("f", F),
        Spec::new("files-from=s", FilesFrom),
        Spec::new("filter!", Filter),
        Spec::new("flush", Flush),
        Spec::new("follow!", Follow),
        Spec::new("g", G),
        Spec::new("group!", Group),
        Spec::new("heading!", Heading),
        Spec::new("h|no-filename", NoFilename),
        Spec::new("H|with-filename", WithFilename),
        Spec::new("i|ignore-case", IgnoreCase),
        Spec::new("I|no-ignore-case", NoIgnoreCase),
        Spec::new("ignore-directory|ignore-dir=s", IgnoreDir),
        Spec::new("ignore-file=s", IgnoreFile),
        Spec::new("l|files-with-matches", FilesWithMatches),
        Spec::new("L|files-without-matches", FilesWithoutMatches),
        Spec::new("m|max-count=i", MaxCount),
        Spec::new("match=s", Match),
        Spec::new("n|no-recurse", NoRecurse),
        Spec::new("o", O),
        Spec::new("output=s", Output),
        Spec::new("pager:s", Pager),
        Spec::new("noignore-directory|noignore-dir=s", NoIgnoreDir),
        Spec::new("nopager", NoPager),
        Spec::new("not=s", Not),
        Spec::new("or=s", Or),
        Spec::new("passthru", Passthru),
        Spec::new("print0", Print0),
        Spec::new("p|proximate:1", Proximate),
        Spec::new("P", NoProximate),
        Spec::new("Q|literal", Literal),
        Spec::new("r|R|recurse", Recurse),
        Spec::new("range-start=s", RangeStart),
        Spec::new("range-end=s", RangeEnd),
        Spec::new("range-invert!", RangeInvert),
        Spec::new("s", S),
        Spec::new("show-types", ShowTypes),
        Spec::new("S|smart-case!", SmartCase),
        Spec::new("sort-files", SortFiles),
        Spec::new("t|type=s", Type),
        Spec::new("T=s", NoType),
        Spec::new("underline!", Underline),
        Spec::new("v|invert-match", InvertMatch),
        Spec::new("w|word-regexp", WordRegexp),
        Spec::new("x", X),
        Spec::new("help", Help),
        Spec::new("help-types", HelpTypes),
        Spec::new("help-colors", HelpColors),
        Spec::new("help-rgb-colors", HelpRgb),
    ];
    for name in &types.specs {
        specs.push(Spec::new(format!("{name}!"), TypeToggle(name.clone())));
    }
    specs.push(Spec::new("k|known-types", KnownTypes));
    specs
}

/// `_generate_ignore_dir`: handles `--ignore-dir` and `--noignore-dir`.
fn add_ignore_dir(opt: &mut Opt, option_name: &str, dir: &[u8]) -> Result<(), String> {
    let is_inverted = option_name.starts_with("--no");

    // _remove_directory_separator
    let mut dir = dir.to_vec();
    let is_sep = |c: u8| {
        if cfg!(windows) {
            c == b'/' || c == b'\\'
        } else {
            c == b'/'
        }
    };
    if dir.last().is_some_and(|&c| is_sep(c)) {
        dir.pop();
    }
    if !dir.contains(&b':') {
        dir.splice(0..0, b"is:".iter().copied());
    }
    let (kind, args) = split_once(&dir, b':');
    let args = args.unwrap_or_default();
    if kind == b"firstlinematch" {
        return Err(die_text(&format!(
            "Invalid filter specification \"{}\" for option '{option_name}'",
            lossy(&kind)
        )));
    }
    let filter = Rc::new(crate::filter::Filter::create(&kind, &split_commas(&args))?);

    let previous_inversion_matches = opt
        .idirs
        .last()
        .is_some_and(|last| is_inverted == last.inverted);
    let collection = if previous_inversion_matches {
        opt.idirs.last().unwrap().collection.clone()
    } else {
        let c = Rc::new(RefCell::new(Collection::default()));
        opt.idirs.push(IgnoreDir {
            collection: c.clone(),
            inverted: is_inverted,
        });
        c
    };
    collection.borrow_mut().add(filter);
    if kind == b"is" {
        collection.borrow_mut().add(Rc::new(Filter::IsPath(args)));
    }
    Ok(())
}

/// `_context_value`
fn context_value(v: i64) -> Result<i64, String> {
    if v < 0 {
        return Ok(2);
    }
    if v > 10_000 {
        return Err(die_text(&format!(
            "Context value {v} exceeds the maximum allowed value of 10000."
        )));
    }
    Ok(v)
}

/// `_type_handler`, for `-t`, `--type` and `-T`.
fn type_handler(opt: &mut Opt, types: &Types, value: &[u8]) -> Result<(), String> {
    let (on, name) = match value.strip_prefix(b"no") {
        Some(rest) => (false, rest),
        None => (true, value),
    };
    let name = lossy(name);
    if !types.specs.contains(&name) {
        return Err(die_text(&format!("Unknown type '{name}'")));
    }
    toggle_type(opt, types, &name, on);
    Ok(())
}

/// The `name!` callback from `_process_filetypes`.
fn toggle_type(opt: &mut Opt, types: &Types, name: &str, on: bool) {
    let filters = types.mappings.get(name).cloned().unwrap_or_default();
    if on {
        // _uninvert_filter: drop inverted copies of these filters.
        opt.filters
            .retain(|f| !(f.inverted && filters.iter().any(|g| Rc::ptr_eq(g, &f.filter))));
    }
    opt.filters
        .extend(filters.into_iter().map(|filter| FilterRef {
            filter,
            inverted: !on,
        }));
}

/// What a handler asks the caller to do after parsing stops.
enum Exit {
    Help,
    HelpTypes,
    HelpColors,
    HelpRgb,
    CreateAckrc,
}

fn handle(
    opt: &mut Opt,
    types: &Types,
    act: &Act,
    cname: &str,
    v: Value,
) -> Result<Option<Exit>, String> {
    use Act::*;
    let flag = v.truthy();
    match act {
        One => {
            opt.one = true;
            opt.m = Some(1);
        }
        And => opt.and.push(v.into_bytes()),
        AfterContext => opt.a = Some(context_value(v.int())?),
        BeforeContext => opt.b = Some(context_value(v.int())?),
        Context => {
            let n = context_value(v.int())?;
            opt.a = Some(n);
            opt.b = Some(n);
        }
        Break => opt.break_ = Some(flag),
        Count => opt.c = true,
        Color => opt.color = Some(flag),
        ColorMatch => opt.colors.match_ = Some(v.into_bytes()),
        ColorFilename => opt.colors.filename = Some(v.into_bytes()),
        ColorColno => opt.colors.colno = Some(v.into_bytes()),
        ColorLineno => opt.colors.lineno = Some(v.into_bytes()),
        Column => opt.column = flag,
        CreateAckrc => return Ok(Some(Exit::CreateAckrc)),
        Debug => opt.debug = true,
        Env => {
            if !flag {
                opt.noenv_seen = true;
            }
        }
        F => opt.f = true,
        FilesFrom => opt.files_from = Some(v.into_bytes()),
        Filter => opt.filter_mode = Some(flag),
        Flush => {}
        Follow => opt.follow = flag,
        G => opt.g = true,
        Group => {
            opt.heading = Some(flag);
            opt.break_ = Some(flag);
        }
        Heading => opt.heading = Some(flag),
        NoFilename => opt.h = Some(true),
        WithFilename => opt.big_h = Some(true),
        IgnoreCase => {
            opt.regex_opts.i = true;
            opt.regex_opts.smart_case = false;
        }
        NoIgnoreCase => {
            opt.regex_opts.i = false;
            opt.regex_opts.smart_case = false;
        }
        IgnoreDir => add_ignore_dir(opt, "--ignore-dir", &v.into_bytes())?,
        NoIgnoreDir => add_ignore_dir(opt, "--noignore-dir", &v.into_bytes())?,
        IgnoreFile => {
            let v = v.into_bytes();
            let (kind, args) = split_once(&v, b':');
            let filter =
                crate::filter::Filter::create(&kind, &split_commas(&args.unwrap_or_default()))?;
            opt.ifiles
                .get_or_insert_with(Collection::default)
                .add(Rc::new(filter));
        }
        FilesWithMatches => opt.l = true,
        FilesWithoutMatches => opt.big_l = true,
        MaxCount => opt.m = Some(v.int()),
        Match => opt.regex = Some(v.into_bytes()),
        NoRecurse => opt.n = true,
        O => opt.output = Some(b"$&".to_vec()),
        Output => opt.output = Some(v.into_bytes()),
        Pager => {
            let v = v.into_bytes();
            opt.pager = if v.is_empty() || v == b"0" {
                std::env::var_os("PAGER").map(|p| crate::bytes::from_os(&p))
            } else {
                Some(v)
            };
        }
        NoPager => opt.pager = None,
        Not => opt.not.push(v.into_bytes()),
        Or => opt.or.push(v.into_bytes()),
        Passthru => opt.passthru = true,
        Print0 => opt.print0 = true,
        Proximate => opt.p = Some(v.int()),
        NoProximate => opt.p = Some(0),
        Literal => opt.regex_opts.literal = true,
        Recurse => opt.n = false,
        RangeStart => opt.range_start = Some(v.into_bytes()),
        RangeEnd => opt.range_end = Some(v.into_bytes()),
        RangeInvert => opt.range_invert = flag,
        S => opt.s = true,
        ShowTypes => opt.show_types = true,
        SmartCase => {
            opt.regex_opts.smart_case = flag;
            if flag {
                opt.regex_opts.i = false;
            }
        }
        SortFiles => opt.sort_files = true,
        Type => type_handler(opt, types, &v.into_bytes())?,
        NoType => type_handler(opt, types, &[b"no", v.into_bytes().as_slice()].concat())?,
        Underline => opt.underline = flag,
        InvertMatch => opt.v = true,
        WordRegexp => opt.regex_opts.w = true,
        X => opt.files_from = Some(b"-".to_vec()),
        Help => return Ok(Some(Exit::Help)),
        HelpTypes => return Ok(Some(Exit::HelpTypes)),
        HelpColors => return Ok(Some(Exit::HelpColors)),
        HelpRgb => return Ok(Some(Exit::HelpRgb)),
        TypeToggle(name) => toggle_type(opt, types, name, flag),
        KnownTypes => {
            for filters in types.mappings.values() {
                opt.filters.extend(filters.iter().map(|f| FilterRef {
                    filter: f.clone(),
                    inverted: false,
                }));
            }
        }
        Forbidden(where_) => {
            return Err(die_text(&format!(
                "Option --{cname} is forbidden in {where_}."
            )));
        }
    }
    Ok(None)
}

/// `read_rcfile`
pub fn read_rcfile(path: &[u8]) -> Vec<Bytes> {
    let p = to_path(path);
    if !p.exists() {
        return Vec::new();
    }
    let content = match std::fs::read(&p) {
        Ok(c) => c,
        Err(e) => die(&format!(
            "Unable to read {}: {}",
            lossy(path),
            errno_text(&e)
        )),
    };
    content
        .split(|&c| c == b'\n')
        .map(|l| l.trim_ascii())
        .filter(|l| !l.is_empty() && !l.starts_with(b"#"))
        .map(<[u8]>::to_vec)
        .collect()
}

/// `retrieve_arg_sources`
pub fn retrieve_arg_sources(argv: &mut Vec<Bytes>) -> Vec<Source> {
    #[derive(Clone)]
    enum Pre {
        NoEnv,
        Ackrc,
    }
    let mut noenv = false;
    let mut ackrc: Option<Bytes> = None;
    getopt::get_options(
        argv,
        &[
            Spec::new("noenv", Pre::NoEnv),
            Spec::new("ackrc=s", Pre::Ackrc),
        ],
        Config::ack(true, true),
        |act, _, v| {
            match act {
                Pre::NoEnv => noenv = v.truthy(),
                Pre::Ackrc => ackrc = Some(v.into_bytes()),
            }
            Ok(())
        },
    );

    let mut files: Vec<(Bytes, bool)> = Vec::new();
    if !noenv {
        files.extend(
            finder::find_config_files()
                .into_iter()
                .map(|f| (f.path, f.project)),
        );
    }
    if let Some(a) = ackrc.filter(|a| !a.is_empty() && a != b"0") {
        if let Err(e) = std::fs::File::open(to_path(&a)) {
            die(&format!(
                "Unable to load ackrc '{}': {}",
                lossy(&a),
                errno_text(&e)
            ));
        }
        files.push((a, false));
    }

    let mut sources = vec![Source {
        name: "Defaults".into(),
        contents: default_options(),
        is_ackrc: false,
        project: false,
    }];
    for (path, project) in files {
        let lines = read_rcfile(&path);
        if !lines.is_empty() {
            sources.push(Source {
                name: lossy(&path),
                contents: lines,
                is_ackrc: true,
                project,
            });
        }
    }
    sources.push(Source {
        name: "ARGV".into(),
        contents: std::mem::take(argv),
        is_ackrc: false,
        project: false,
    });
    sources
}

/// `_remove_default_options_if_needed`
fn remove_default_options_if_needed(sources: &mut Vec<Source>) {
    let Some(default_index) = sources.iter().position(|s| s.name == "Defaults") else {
        return;
    };
    let mut should_remove = false;
    for source in &mut sources[default_index + 1..] {
        getopt::get_options(
            &mut source.contents,
            &[Spec::new("ignore-ack-defaults", ())],
            Config::ack(true, true),
            |_, _, _| {
                should_remove = true;
                Ok(())
            },
        );
    }
    if should_remove {
        sources.remove(default_index);
    }
}

/// `_dump_options`: prints each source's options, one option (with its
/// arguments) per line, sorted.
fn dump_options(sources: &[Source]) {
    let mut names: Vec<&str> = Vec::new();
    let mut by_source: HashMap<&str, Vec<Vec<Bytes>>> = HashMap::new();
    for source in sources {
        // _explode_sources: split into chunks of an option and its arguments.
        let opts = &source.contents;
        let mut j = 0;
        while j < opts.len() {
            if !opts[j].starts_with(b"-") {
                j += 1;
                continue;
            }
            let mut chunk = vec![opts[j].clone()];
            j += 1;
            while j < opts.len() && !opts[j].starts_with(b"-") {
                chunk.push(opts[j].clone());
                j += 1;
            }
            if !by_source.contains_key(source.name.as_str()) {
                names.push(&source.name);
            }
            by_source.entry(&source.name).or_default().push(chunk);
        }
    }
    let key = |chunk: &Vec<Bytes>| -> Bytes {
        let first = &chunk[0];
        let first = first.strip_prefix(b"-").unwrap_or(first);
        first.strip_prefix(b"-").unwrap_or(first).to_vec()
    };
    for name in names {
        let mut chunks = by_source.remove(name).unwrap_or_default();
        chunks.sort_by_key(key);
        output::print(&[
            name.as_bytes(),
            b"\n",
            "=".repeat(name.len()).as_bytes(),
            b"\n",
        ]);
        for chunk in chunks {
            output::print(&[b"  ", &chunk.join(&b" "[..]), b"\n"]);
        }
    }
}

/// The result of option processing.
pub struct Processed {
    pub opt: Opt,
    pub types: Types,
    /// What's left of the command line: the pattern and files.
    pub argv: Vec<Bytes>,
}

/// `process_args`
pub fn process_args(mut sources: Vec<Source>) -> Processed {
    let mut opt = Opt {
        pager: crate::env::var("ACK_PAGER_COLOR")
            .filter(|v| !v.is_empty() && v != b"0")
            .or_else(|| crate::env::var("ACK_PAGER").filter(|v| !v.is_empty() && v != b"0")),
        ..Opt::default()
    };

    let original_argv: Vec<Bytes> = sources
        .iter()
        .find(|s| s.name == "ARGV")
        .map(|s| s.contents.clone())
        .unwrap_or_default();
    remove_default_options_if_needed(&mut sources);

    // Check for --dump early.
    if let Some(argv) = sources.iter_mut().find(|s| s.name == "ARGV") {
        let mut dump = false;
        getopt::get_options(
            &mut argv.contents,
            &[Spec::new("dump", ())],
            Config::ack(false, true),
            |_, _, _| {
                dump = true;
                Ok(())
            },
        );
        if dump {
            dump_options(&sources);
            output::exit(0);
        }
    }

    let types = process_filetypes(&mut sources);

    let spec_strings: Vec<String> = arg_spec(&types).into_iter().map(|s| s.spec).collect();
    mutex::check(&original_argv, &spec_strings);

    // _process_other
    let mut is_help_types_active = false;
    if let Some(argv) = sources.iter().find(|s| s.name == "ARGV") {
        let mut copy = argv.contents.clone();
        getopt::get_options(
            &mut copy,
            &[Spec::new("help-types", ())],
            Config::ack(false, true),
            |_, _, _| {
                is_help_types_active = true;
                Ok(())
            },
        );
    }

    let base = arg_spec(&types);
    for source in &mut sources {
        let mut specs: Vec<Spec<Act>> = base
            .iter()
            .map(|s| Spec::new(s.spec.clone(), s.action.clone()))
            .collect();
        if source.is_ackrc {
            specs.retain(|s| s.spec != "output=s" && s.spec != "match=s");
            specs.push(Spec::new("output=s", Act::Forbidden(".ackrc files")));
            specs.push(Spec::new("match=s", Act::Forbidden(".ackrc files")));
        }
        if source.project {
            specs.retain(|s| s.spec != "pager:s" && s.spec != "follow!");
            specs.push(Spec::new("pager:s", Act::Forbidden("project .ackrc files")));
            specs.push(Spec::new("follow!", Act::Forbidden("project .ackrc files")));
        }

        let ok = getopt::get_options(
            &mut source.contents,
            &specs,
            Config::ack(false, false),
            |act, cname, v| {
                match handle(&mut opt, &types, act, cname, v) {
                    // These options print something and exit from inside Getopt::Long.
                    Ok(Some(e)) => run_exit(e, &types),
                    Ok(None) => {}
                    Err(text) => return Err(HandlerError::Died(text)),
                }
                Ok(())
            },
        );
        if !ok && !is_help_types_active {
            let where_ = if source.name == "ARGV" {
                "on command line".to_string()
            } else {
                format!("in {}", source.name)
            };
            die(&format!("Invalid option {where_}"));
        }
        if opt.noenv_seen {
            die(&format!("--noenv found in {}", source.name));
        }
    }

    let mut argv = Vec::new();
    for source in sources {
        if source.name == "ARGV" {
            argv = source.contents;
        } else if !source.contents.is_empty() {
            die(&format!("Source '{}' has extra arguments!", source.name));
        }
    }

    // Throw the default filter in if no others are selected.
    if !opt.filters.iter().any(|f| !f.inverted) {
        opt.filters.push(FilterRef {
            filter: Rc::new(Filter::Default),
            inverted: false,
        });
    }
    Processed { opt, types, argv }
}

fn run_exit(e: Exit, types: &Types) -> ! {
    match e {
        Exit::Help => crate::help::show_help(),
        Exit::HelpTypes => crate::help::show_help_types(types),
        Exit::HelpColors => die("--help-colors is not implemented yet (rask Phase 4)"),
        Exit::HelpRgb => die("--help-rgb-colors is not implemented yet (rask Phase 4)"),
        Exit::CreateAckrc => {
            output::print(&[b"--ignore-ack-defaults\n"]);
            for line in DEFAULT_ACKRC.lines() {
                output::print(&[line.as_bytes(), b"\n"]);
            }
        }
    }
    output::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_specs() {
        let (name, f) = process_filter_spec(b"perl:ext:pl,pm").unwrap();
        assert_eq!(name, "perl");
        assert!(matches!(f, Filter::Ext { .. }));
        let (name, f) = process_filter_spec(b"foo=.x,y").unwrap();
        assert_eq!(name, "foo");
        let Filter::Ext { extensions, .. } = f else {
            panic!()
        };
        assert_eq!(extensions, [b"x".to_vec(), b"y".to_vec()]);
        assert!(process_filter_spec(b"foo").is_err());
    }

    #[test]
    fn defaults_parse() {
        let mut sources = vec![Source {
            name: "Defaults".into(),
            contents: default_options(),
            is_ackrc: false,
            project: false,
        }];
        let types = process_filetypes(&mut sources);
        assert!(types.specs.contains("perl"));
        assert!(types.specs.contains("make"));
        // Only the ignore options are left for the main pass.
        assert!(
            sources[0]
                .contents
                .iter()
                .all(|o| o.starts_with(b"--ignore-"))
        );
    }
}
