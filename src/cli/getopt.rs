//! A port of the parts of Getopt::Long 2.54 that ack uses.
//!
//! ack runs several passes over its argument sources with different parser
//! configurations (`App::Ack::ConfigLoader::configure_parser`), and relies on
//! Getopt::Long's exact behaviour: bundling, permuting, auto-abbreviation,
//! `--no`/`--no-` negation, optional values, pass-through, and the wording of
//! its warnings. This module follows `GetOptionsFromArray`, `FindOption` and
//! `ParseOptionSpec` closely, so the Perl source is the best documentation.
//!
//! Arguments are byte strings, as they are in Perl.

use std::collections::BTreeMap;

use crate::bytes::Bytes;

/// Getopt::Long configuration, as set by `configure_parser( @opts )`:
/// `default bundling no_auto_help no_auto_version no_ignore_case`, plus
/// optionally `no_auto_abbrev` and/or `pass_through`.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    autoabbrev: bool,
    /// `+` also starts an option (`getopt_compat`).
    plus_prefix: bool,
    permute: bool,
    passthrough: bool,
}

impl Config {
    pub fn ack(no_auto_abbrev: bool, pass_through: bool) -> Config {
        // ConfigDefaults() looks at POSIXLY_CORRECT.
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        Config {
            autoabbrev: !posix && !no_auto_abbrev,
            plus_prefix: !posix,
            permute: !posix,
            passthrough: pass_through,
        }
    }
}

/// A value passed to an option handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Str(Bytes),
}

impl Value {
    pub fn int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Str(s) => perl_numify(s),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Int(i) => *i != 0,
            Value::Str(s) => !(s.is_empty() || s == b"0"),
        }
    }

    pub fn into_bytes(self) -> Bytes {
        match self {
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Str(s) => s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    /// `''`: a plain flag.
    Flag,
    /// `'!'`: the `no`/`no-` form of a negatable flag.
    Neg,
    /// `'s'`
    Str,
    /// `'i'`
    Int,
}

#[derive(Debug, Clone)]
struct Ctl<A> {
    ty: Type,
    cname: String,
    default: Option<i64>,
    mandatory: bool,
    action: A,
}

/// An option specification, such as `"m|max-count=i"` or `"color|colour!"`,
/// and what to do when it's seen.
pub struct Spec<A> {
    pub spec: String,
    pub action: A,
}

impl<A> Spec<A> {
    pub fn new(spec: impl Into<String>, action: A) -> Self {
        Spec {
            spec: spec.into(),
            action,
        }
    }
}

/// What an option handler reports back.
pub enum HandlerError {
    /// The handler died with this message (e.g. from `App::Ack::die`).
    /// Getopt::Long prints it as a warning and counts an error.
    Died(String),
}

fn parse_spec<A: Clone>(spec: &str, action: A, table: &mut BTreeMap<Bytes, Ctl<A>>) {
    // The specs are all ours, so they're trusted to be well-formed.
    let split = spec.find(['!', '=', ':']).unwrap_or(spec.len());
    let (names, kind) = spec.split_at(split);
    let names: Vec<&str> = names.split('|').collect();
    let cname = names[0].to_string();

    let (ty, default, mandatory) = match kind {
        "" | "!" => (Type::Flag, None, false),
        "=s" => (Type::Str, None, true),
        ":s" => (Type::Str, None, false),
        "=i" => (Type::Int, None, true),
        ":i" => (Type::Int, None, false),
        _ if kind.starts_with(':') => {
            let n: i64 = kind[1..]
                .parse()
                .unwrap_or_else(|_| panic!("bad option spec {spec}"));
            (Type::Int, Some(n), false)
        }
        _ => panic!("unsupported option spec {spec}"),
    };
    let ctl = Ctl {
        ty,
        cname,
        default,
        mandatory,
        action,
    };
    for name in names {
        if kind == "!" {
            let neg = Ctl {
                ty: Type::Neg,
                ..ctl.clone()
            };
            table.insert(format!("no{name}").into_bytes(), neg.clone());
            table.insert(format!("no-{name}").into_bytes(), neg);
        }
        table.insert(name.as_bytes().to_vec(), ctl.clone());
    }
}

/// Where an option was found, with its canonical name and value.
struct Found<A> {
    action: A,
    cname: String,
    value: Value,
}

enum Lookup<A> {
    /// Not an option (or unknown, when passing through).
    NotAnOption,
    /// An option in error; the warning has been printed.
    Error,
    Found(Found<A>),
}

struct Parser<'a, A> {
    cfg: Config,
    table: &'a BTreeMap<Bytes, Ctl<A>>,
    errors: usize,
}

fn warn(msg: &str) {
    eprint!("{msg}");
}

fn cat(a: &[u8], b: &[u8]) -> Bytes {
    let mut v = a.to_vec();
    v.extend_from_slice(b);
    v
}

fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Perl's `0+$str` on the strings Getopt::Long lets through.
pub fn perl_numify(s: &[u8]) -> i64 {
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for &c in digits.iter().take_while(|c| c.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add(i64::from(c - b'0'));
    }
    if neg { -n } else { n }
}

/// `^[-+]?_*[0-9][0-9_]*` (PAT_INT). Returns the length of the match.
fn match_int_prefix(s: &[u8]) -> Option<usize> {
    let mut i = 0;
    if matches!(s.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    while s.get(i) == Some(&b'_') {
        i += 1;
    }
    if !s.get(i).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    i += 1;
    while s.get(i).is_some_and(|c| c.is_ascii_digit() || *c == b'_') {
        i += 1;
    }
    Some(i)
}

impl<A: Clone> Parser<'_, A> {
    /// The starter (`--`, `-` or `+`) and the rest of the argument.
    fn split_prefix<'b>(&self, arg: &'b [u8]) -> Option<(&'static [u8], &'b [u8])> {
        if let Some(rest) = arg.strip_prefix(b"--") {
            Some((b"--", rest))
        } else if let Some(rest) = arg.strip_prefix(b"-") {
            Some((b"-", rest))
        } else if self.cfg.plus_prefix {
            arg.strip_prefix(b"+").map(|rest| (&b"+"[..], rest))
        } else {
            None
        }
    }

    /// `/^$prefix.+/`
    fn looks_like_option(&self, arg: &[u8]) -> bool {
        self.split_prefix(arg)
            .is_some_and(|(_, rest)| !rest.is_empty())
    }

    fn find_option(&mut self, argv: &mut Vec<Bytes>, arg: &[u8]) -> Lookup<A> {
        let Some((starter, mut opt)) = self.split_prefix(arg) else {
            return Lookup::NotAnOption;
        };
        if arg == b"-" {
            return Lookup::NotAnOption;
        }
        let passthrough = self.cfg.passthrough;

        let mut optarg: Option<Bytes> = None;
        let mut rest: Option<Bytes> = None;

        // A long option may include its value.
        if starter == b"--"
            && let Some(pos) = opt.iter().skip(1).position(|&c| c == b'=').map(|p| p + 1)
        {
            optarg = Some(opt[pos + 1..].to_vec());
            opt = &opt[..pos];
        }

        let tryopt: Bytes;
        if starter == b"-" {
            // Bundling: split off a single letter and leave the rest.
            tryopt = opt.get(..1).unwrap_or(b"").to_vec();
            if opt.len() > 1 {
                rest = Some(opt[1..].to_vec());
            }
        } else if self.cfg.autoabbrev && !opt.is_empty() {
            // The table is a BTreeMap, so the names are already sorted.
            let mut hits: Vec<Bytes> = self
                .table
                .keys()
                .filter(|k| k.starts_with(opt))
                .cloned()
                .collect();
            let exact = hits.iter().filter(|h| h.as_slice() == opt).count();
            if !(hits.len() <= 1 || exact == 1) {
                // See if all matches are for the same option.
                let mut distinct: Vec<String> = Vec::new();
                for h in &hits {
                    let ctl = &self.table[h];
                    let name = if ctl.ty == Type::Neg {
                        format!("no{}", ctl.cname)
                    } else {
                        ctl.cname.clone()
                    };
                    if !distinct.contains(&name) {
                        distinct.push(name);
                    }
                }
                if distinct.len() != 1 {
                    if passthrough {
                        return Lookup::NotAnOption;
                    }
                    let list: Vec<String> = hits.iter().map(|h| show(h)).collect();
                    warn(&format!(
                        "Option {} is ambiguous ({})\n",
                        show(opt),
                        list.join(", ")
                    ));
                    self.errors += 1;
                    return Lookup::Error;
                }
                hits = vec![distinct.remove(0).into_bytes()];
            }
            // Complete the option name, if appropriate.
            tryopt = if hits.len() == 1 && hits[0] != opt {
                hits.remove(0)
            } else {
                opt.to_vec()
            };
        } else {
            tryopt = opt.to_vec();
        }
        self.finish_lookup(argv, starter, opt, &tryopt, optarg, rest)
    }

    fn finish_lookup(
        &mut self,
        argv: &mut Vec<Bytes>,
        starter: &[u8],
        opt: &[u8],
        tryopt: &[u8],
        optarg: Option<Bytes>,
        rest: Option<Bytes>,
    ) -> Lookup<A> {
        let passthrough = self.cfg.passthrough;
        let Some(ctl) = self.table.get(tryopt) else {
            if passthrough {
                return Lookup::NotAnOption;
            }
            // Pretend one char when bundling.
            let mut shown = opt.to_vec();
            if starter.len() == 1 {
                shown.truncate(1);
                if let Some(r) = &rest {
                    argv.insert(0, cat(starter, r));
                }
            }
            if shown.is_empty() {
                warn(&format!("Missing option after {}\n", show(starter)));
            } else {
                warn(&format!("Unknown option: {}\n", show(&shown)));
            }
            self.errors += 1;
            return Lookup::Error;
        };
        let name = show(tryopt);
        let found = |value| {
            Lookup::Found(Found {
                action: ctl.action.clone(),
                cname: ctl.cname.clone(),
                value,
            })
        };

        if matches!(ctl.ty, Type::Flag | Type::Neg) {
            if optarg.is_some() {
                if passthrough {
                    return Lookup::NotAnOption;
                }
                warn(&format!("Option {name} does not take an argument\n"));
                self.errors += 1;
                if let Some(r) = &rest {
                    argv.insert(0, cat(starter, r));
                }
                return Lookup::Error;
            }
            let value = Value::Int(if ctl.ty == Type::Flag { 1 } else { 0 });
            if let Some(r) = &rest {
                argv.insert(0, cat(starter, r));
            }
            return found(value);
        }

        let no_value_available = match &optarg {
            Some(a) => a.is_empty(),
            None => rest.is_none() && argv.is_empty(),
        };
        if no_value_available {
            if ctl.mandatory {
                if passthrough {
                    return Lookup::NotAnOption;
                }
                warn(&format!("Option {name} requires an argument\n"));
                self.errors += 1;
                return Lookup::Error;
            }
            return found(match (ctl.default, ctl.ty) {
                (Some(d), _) => Value::Int(d),
                (None, Type::Str) => Value::Str(Vec::new()),
                (None, _) => Value::Int(0),
            });
        }

        let had_optarg = optarg.is_some();
        let had_rest = rest.is_some();
        let arg: Bytes = match (&rest, optarg) {
            (Some(r), _) => r.clone(),
            (None, Some(o)) => o,
            (None, None) => argv.remove(0),
        };

        if ctl.ty == Type::Str {
            if ctl.mandatory || had_optarg || had_rest || arg == b"-" {
                return found(Value::Str(arg));
            }
            if arg == b"--" || self.looks_like_option(&arg) {
                argv.insert(0, arg);
                return found(Value::Str(Vec::new()));
            }
            return found(Value::Str(arg));
        }

        // Integer.
        if let Some(r) = rest.as_ref().filter(|_| had_rest)
            && let Some(n) = match_int_prefix(r)
        {
            let value = perl_numify(&r[..n]);
            let after = &r[n..];
            if !after.is_empty() {
                argv.insert(0, cat(starter, after));
            }
            return found(Value::Int(value));
        }
        if match_int_prefix(&arg) == Some(arg.len()) {
            let cleaned: Bytes = arg.iter().copied().filter(|&c| c != b'_').collect();
            return found(Value::Int(perl_numify(&cleaned)));
        }
        if had_optarg || ctl.mandatory {
            if passthrough {
                if !had_optarg {
                    argv.insert(0, if had_rest { cat(starter, &arg) } else { arg });
                }
                return Lookup::NotAnOption;
            }
            warn(&format!(
                "Value \"{}\" invalid for option {name} (number expected)\n",
                show(&arg)
            ));
            self.errors += 1;
            if had_rest {
                argv.insert(0, cat(starter, &arg));
            }
            return Lookup::Error;
        }
        argv.insert(0, if had_rest { cat(starter, &arg) } else { arg });
        found(Value::Int(ctl.default.unwrap_or(0)))
    }
}

/// `Getopt::Long::GetOptionsFromArray( $argv, %specs )`.
///
/// Calls `handler` for each option found, in order. Non-options are left in
/// `argv`. Returns false if there were errors (which have been printed).
pub fn get_options<A: Clone>(
    argv: &mut Vec<Bytes>,
    specs: &[Spec<A>],
    cfg: Config,
    handler: impl FnMut(&A, &str, Value) -> Result<(), HandlerError>,
) -> bool {
    get_options_with_nonoptions(argv, specs, cfg, handler, None)
}

/// Called with each non-option (Getopt::Long's `'<>'` spec).
pub type NonOptionHandler<'a> = &'a mut dyn FnMut(&[u8]);

/// `GetOptionsFromArray` with a `'<>'` handler, which is given each
/// non-option (when permuting) instead of leaving it in `argv`.
pub fn get_options_with_nonoptions<A: Clone>(
    argv: &mut Vec<Bytes>,
    specs: &[Spec<A>],
    cfg: Config,
    mut handler: impl FnMut(&A, &str, Value) -> Result<(), HandlerError>,
    mut nonoption: Option<NonOptionHandler<'_>>,
) -> bool {
    let mut table = BTreeMap::new();
    for s in specs {
        parse_spec(&s.spec, s.action.clone(), &mut table);
    }
    let mut parser = Parser {
        cfg,
        table: &table,
        errors: 0,
    };

    let mut ret: Vec<Bytes> = Vec::new();
    while !argv.is_empty() {
        let arg = argv.remove(0);
        if arg == b"--" {
            if cfg.passthrough {
                ret.push(arg);
            }
            break;
        }
        match parser.find_option(argv, &arg) {
            Lookup::Found(f) => {
                if let Err(HandlerError::Died(msg)) = handler(&f.action, &f.cname, f.value) {
                    warn(&msg);
                    parser.errors += 1;
                }
            }
            Lookup::Error => {}
            Lookup::NotAnOption => {
                if cfg.permute {
                    match nonoption.as_mut() {
                        Some(f) => f(&arg),
                        None => ret.push(arg),
                    }
                } else {
                    argv.insert(0, arg);
                    return parser.errors == 0;
                }
            }
        }
    }
    if !ret.is_empty() && (cfg.permute || cfg.passthrough) {
        ret.append(argv);
        *argv = ret;
    }
    parser.errors == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum A {
        X,
    }

    fn run(
        specs: &[&str],
        args: &[&str],
        abbrev: bool,
        pass: bool,
    ) -> (Vec<String>, Vec<String>, bool) {
        let specs: Vec<Spec<A>> = specs.iter().map(|s| Spec::new(*s, A::X)).collect();
        let mut argv: Vec<Bytes> = args.iter().map(|a| a.as_bytes().to_vec()).collect();
        let mut seen = Vec::new();
        let cfg = Config {
            autoabbrev: abbrev,
            plus_prefix: true,
            permute: true,
            passthrough: pass,
        };
        let ok = get_options(&mut argv, &specs, cfg, |_, name, v| {
            seen.push(format!("{name}={}", show(&v.into_bytes())));
            Ok(())
        });
        (seen, argv.iter().map(|a| show(a)).collect(), ok)
    }

    const SPECS: &[&str] = &[
        "i|ignore-case",
        "w",
        "m|max-count=i",
        "C|context:-1",
        "color|colour!",
        "column!",
        "pager:s",
        "t|type=s",
        "sort-files",
        "1",
    ];

    #[test]
    fn bundling_and_permute() {
        let (seen, rest, ok) = run(SPECS, &["foo", "-iw", "bar", "-m3", "-C", "5"], true, false);
        assert!(ok);
        assert_eq!(seen, ["i=1", "w=1", "m=3", "C=5"]);
        assert_eq!(rest, ["foo", "bar"]);
    }

    #[test]
    fn optional_int_leaves_non_numbers() {
        let (seen, rest, _) = run(SPECS, &["-C", "3x", "foo"], true, false);
        assert_eq!(seen, ["C=-1"]);
        assert_eq!(rest, ["3x", "foo"]);
    }

    #[test]
    fn abbreviation_and_negation() {
        let (seen, _, ok) = run(
            SPECS,
            &["--sort", "--nocolo", "--no-column", "--colou"],
            true,
            false,
        );
        assert!(ok);
        assert_eq!(seen, ["sort-files=1", "color=0", "column=0", "color=1"]);
        let (_, _, ok) = run(SPECS, &["--col"], true, false);
        assert!(!ok);
        let (_, rest, ok) = run(SPECS, &["--sort"], false, true);
        assert!(ok);
        assert_eq!(rest, ["--sort"]);
    }

    #[test]
    fn double_dash() {
        let (_, rest, _) = run(SPECS, &["a", "--", "-i"], true, false);
        assert_eq!(rest, ["a", "-i"]);
        let (_, rest, _) = run(SPECS, &["a", "--", "-i"], true, true);
        assert_eq!(rest, ["a", "--", "-i"]);
    }

    #[test]
    fn pass_through_keeps_unknown_bundles() {
        let (seen, rest, ok) = run(&["i"], &["-iq", "x"], false, true);
        assert!(ok);
        assert_eq!(seen, ["i=1"]);
        assert_eq!(rest, ["-q", "x"]);
    }

    #[test]
    fn optional_string() {
        let (seen, rest, _) = run(SPECS, &["--pager", "-i"], true, false);
        assert_eq!(seen, ["pager=", "i=1"]);
        assert!(rest.is_empty());
        let (seen, _, _) = run(SPECS, &["--pager=less"], true, false);
        assert_eq!(seen, ["pager=less"]);
    }
}
