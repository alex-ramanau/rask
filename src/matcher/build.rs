//! Port of `App::Ack::build_regex`, `build_all_regexes` and `is_lowercase`.
//!
//! Perl interpolates a `qr//` object as `(?^flags:pattern)`. Regexes here are
//! kept in that "qr form" as pattern strings, so combining them for `--and`,
//! `--or` and `--not` gives the same regexes Perl ack builds.

use std::sync::LazyLock;

use super::perl_syntax::{self, PerlError};
use super::{PcreMatcher, pattern_to_pcre};
use crate::bytes::{Bytes, lossy};

/// Regex-related options (`$opt->{i}`, `{S}`, `{w}`, `{Q}`).
#[derive(Debug, Default, Clone, Copy)]
pub struct RegexOpts {
    pub i: bool,
    pub smart_case: bool,
    pub w: bool,
    pub literal: bool,
}

/// A regex in qr form, e.g. `(?^:foo)`.
pub type Qr = Bytes;

fn qr(pattern: &[u8], flags: &str) -> Qr {
    let mut q = format!("(?^{flags}:").into_bytes();
    q.extend_from_slice(pattern);
    // Perl ends the stringified qr with a newline when /x could make a
    // trailing comment swallow the closing paren.
    if pattern.contains(&b'#') && has_inline_x(pattern) {
        q.push(b'\n');
    }
    q.push(b')');
    q
}

fn has_inline_x(p: &[u8]) -> bool {
    p.windows(2).enumerate().any(|(i, w)| {
        w == b"(?"
            && p[i + 2..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic() || **c == b'^' || **c == b'-')
                .take_while(|c| **c != b'-')
                .any(|c| *c == b'x')
    })
}

fn re(pattern: &str) -> PcreMatcher {
    PcreMatcher::new(pattern).expect("internal regex")
}

/// Removes every match of `m` from `s` (Perl's `s/$re//g`).
fn remove_all(m: &PcreMatcher, s: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(s.len());
    let mut at = 0;
    while at <= s.len() {
        match m.find_at_raw(s, at) {
            Some((start, end)) if end > start => {
                out.extend_from_slice(&s[at..start]);
                at = end;
            }
            Some((start, _)) => {
                // Empty match: keep one byte and move on.
                if start < s.len() {
                    out.extend_from_slice(&s[at..=start]);
                } else {
                    out.extend_from_slice(&s[at..]);
                }
                at = start + 1;
            }
            None => {
                out.extend_from_slice(&s[at..]);
                break;
            }
        }
    }
    out
}

/// `App::Ack::is_lowercase`: is the pattern free of upper case letters, once
/// metacharacters that contain capitals (`\A`, `\W`, `\p{Lu}`, named groups,
/// ...) are ignored?
pub fn is_lowercase(pat: &[u8]) -> bool {
    static META: LazyLock<PcreMatcher> = LazyLock::new(|| {
        re(concat!(
            r"\\A|\\B|\\c[a-zA-Z]|\\D|\\G|\\H|\\K|\\N(\{.+?\})?|\\[pP]\{.+?\}|\\[pP][A-Z]",
            r"|\\R|\\S|\\V|\\W|\\X|\\x[0-9A-Fa-f]{2}|\\Z"
        ))
    });
    static NAMED: LazyLock<PcreMatcher> = LazyLock::new(|| {
        let name = "[_A-Za-z][_A-Za-z0-9]*?";
        re(&format!(
            r"\(\?'{name}'|\(\?<{name}>|\\k'{name}'|\\k<{name}>|\\k\{{{name}\}}"
        ))
    });

    let is_lc = |p: &[u8]| !p.iter().any(u8::is_ascii_uppercase);
    if is_lc(pat) {
        return true;
    }
    let pat: Bytes = remove_all(&re(r"\\\\"), pat);
    let pat = remove_all(&META, &pat);
    let pat = remove_all(&NAMED, &pat);
    is_lc(&pat)
}

/// Perl's `quotemeta` on a byte string: backslash everything except `[A-Za-z0-9_]`.
pub fn quotemeta(s: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(s.len() * 2);
    for &c in s {
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            out.push(b'\\');
        }
        out.push(c);
    }
    out
}

/// PCRE2's message without the "PCRE2: error compiling pattern at offset N: " prefix.
fn pcre_message(e: &pcre2::Error) -> String {
    let s = e.to_string();
    match s
        .find(": ")
        .and_then(|i| s[i + 2..].find(": ").map(|j| i + 2 + j + 2))
    {
        Some(i) => s[i..].to_string(),
        None => s,
    }
}

fn invalid_regex(pattern: &[u8], e: PerlError) -> String {
    let s = lossy(pattern);
    match e.here {
        Some(here) => {
            let pointy = " ".repeat(6 + here);
            format!(
                "Invalid regex '{s}'\nRegex: {s}\n{pointy}^---HERE {} in regex",
                e.why
            )
        }
        None => format!("Invalid regex '{s}'\n{}.", e.why),
    }
}

/// Compiles `pattern` the way `qr/$str/` would, returning Perl's diagnostics
/// on failure.
fn perl_compile_check(pattern: &[u8]) -> Result<(), PerlError> {
    perl_syntax::check(pattern)?;
    match pcre2::bytes::RegexBuilder::new().build(&pattern_to_pcre(pattern)) {
        Ok(_) => Ok(()),
        Err(e) => {
            let here = e
                .offset()
                .map(|o| o.min(pattern.len()))
                .unwrap_or(pattern.len());
            Err(PerlError {
                why: pcre_message(&e),
                here: Some(here),
            })
        }
    }
}

/// `App::Ack::build_regex`. Returns the match regex and, when the pattern
/// allows it, the regex used to pre-scan whole files (`$scan_regex`), both in
/// qr form. `Err` is the message for `App::Ack::die`.
pub fn build_regex(input: &[u8], opt: &RegexOpts) -> Result<(Qr, Option<Qr>), String> {
    // Check for lowercaseness before we do any modifications.
    let regex_is_lc = is_lowercase(input);

    let mut s: Bytes = input.to_vec();
    if opt.literal {
        s = quotemeta(&s);
    } else if let Err(e) = perl_compile_check(&s) {
        return Err(invalid_regex(&s, e));
    }

    let mut scan_str = s.clone();

    if opt.w {
        static EXPLICIT_WD: LazyLock<PcreMatcher> = LazyLock::new(|| re(r"^\\[wd]"));
        static GOOD_START: LazyLock<PcreMatcher> = LazyLock::new(|| re(r"^[\w\(\[\.]"));
        static GOOD_END: LazyLock<PcreMatcher> = LazyLock::new(|| re(r"[\w\}\)\]\+\*\?\.]$"));
        static ESCAPED_END: LazyLock<PcreMatcher> = LazyLock::new(|| re(r"\\[\}\)\]\+\*\?\.]$"));
        static SIMPLE_WORD: LazyLock<PcreMatcher> = LazyLock::new(|| re(r"^\w+$"));

        let m = |r: &PcreMatcher| r.find_at_raw(&s, 0).is_some();
        let mut ok = m(&EXPLICIT_WD) || m(&GOOD_START);
        if !m(&GOOD_END) || m(&ESCAPED_END) {
            ok = false;
        }
        if !ok {
            return Err("-w will not do the right thing if your regex does not begin and end with a word character.".into());
        }

        let wrapped = if m(&SIMPLE_WORD) {
            [&b"\\b(?:"[..], &s, b")\\b"].concat()
        } else {
            [&b"(?:^|\\b|\\s)\\K(?:"[..], &s, b")(?=\\s|\\b|$)"].concat()
        };
        s = wrapped;
    }

    if opt.i || (opt.smart_case && regex_is_lc) {
        s.splice(0..0, b"(?i)".iter().copied());
        scan_str.splice(0..0, b"(?i)".iter().copied());
    }

    if let Err(e) = pcre2::bytes::RegexBuilder::new().build(&pattern_to_pcre(&s)) {
        return Err(format!(
            "Invalid regex '{}':\n  {}",
            lossy(&s),
            pcre_message(&e)
        ));
    }
    // No line_scan is possible if there's a $ in the regex.
    let scan = (!scan_str.contains(&b'$')).then(|| qr(&scan_str, "m"));
    Ok((qr(&s, ""), scan))
}

/// The regexes from `App::Ack::build_all_regexes`, in qr form.
#[derive(Debug, Clone)]
pub struct AllRegexes {
    pub re_match: Qr,
    pub re_not: Option<Qr>,
    pub re_hilite: Qr,
    pub re_scan: Option<Qr>,
}

pub fn build_all_regexes(
    regex: &[u8],
    and: &[Bytes],
    or: &[Bytes],
    not: &[Bytes],
    opt: &RegexOpts,
) -> Result<AllRegexes, String> {
    let join = |parts: &[Qr], sep: &[u8]| parts.join(sep);
    let (re_match, re_hilite, re_scan);

    if !and.is_empty() {
        let mut match_parts = Vec::new();
        let mut hilite_parts = Vec::new();
        for part in and {
            let (m, _) = build_regex(part, opt)?;
            match_parts.push([&b"(?=.*"[..], &m, b")"].concat());
            hilite_parts.push(m);
        }
        let (m, scan) = build_regex(regex, opt)?;
        match_parts.push([&b".*"[..], &m].concat());
        hilite_parts.push(m);
        re_match = join(&match_parts, b"");
        re_hilite = join(&hilite_parts, b"|");
        re_scan = scan;
    } else if !or.is_empty() {
        let mut match_parts = Vec::new();
        let mut scan_parts = Vec::new();
        for part in std::iter::once(&regex.to_vec()).chain(or) {
            let (m, scan) = build_regex(part, opt)?;
            match_parts.push(m);
            // Perl joins an undef scan regex as an empty string.
            scan_parts.push(scan.unwrap_or_default());
        }
        re_match = join(&match_parts, b"|");
        re_hilite = re_match.clone();
        re_scan = Some(join(&scan_parts, b"|"));
    } else {
        let (m, scan) = build_regex(regex, opt)?;
        re_hilite = m.clone();
        re_match = m;
        re_scan = scan;
    }

    // The --not does not affect the main regex. It is checked separately.
    let re_not = if !not.is_empty() {
        let mut not_parts = Vec::new();
        for part in not {
            not_parts.push(build_regex(part, opt)?.0);
        }
        Some(join(&not_parts, b"|"))
    } else {
        None
    };

    Ok(AllRegexes {
        re_match,
        re_not,
        re_hilite,
        re_scan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(s: &str, opt: RegexOpts) -> Result<(String, Option<String>), String> {
        build_regex(s.as_bytes(), &opt).map(|(m, s)| (lossy(&m), s.map(|s| lossy(&s))))
    }

    #[test]
    fn plain_and_flags() {
        assert_eq!(
            build("foo", RegexOpts::default()).unwrap(),
            ("(?^:foo)".into(), Some("(?^m:foo)".into()))
        );
        let i = RegexOpts {
            i: true,
            ..Default::default()
        };
        assert_eq!(build("foo", i).unwrap().0, "(?^:(?i)foo)");
        assert_eq!(build("foo$", i).unwrap().1, None);
        let s = RegexOpts {
            smart_case: true,
            ..Default::default()
        };
        assert_eq!(build("foo", s).unwrap().0, "(?^:(?i)foo)");
        assert_eq!(build("Foo", s).unwrap().0, "(?^:Foo)");
        assert_eq!(build("\\Wfoo", s).unwrap().0, "(?^:(?i)\\Wfoo)");
    }

    #[test]
    fn word_and_literal() {
        let w = RegexOpts {
            w: true,
            ..Default::default()
        };
        assert_eq!(build("foo", w).unwrap().0, "(?^:\\b(?:foo)\\b)");
        assert_eq!(
            build("foo?", w).unwrap().0,
            "(?^:(?:^|\\b|\\s)\\K(?:foo?)(?=\\s|\\b|$))"
        );
        assert!(build("-foo", w).unwrap_err().starts_with("-w will not"));
        assert!(build("foo\\.", w).is_err());
        let q = RegexOpts {
            literal: true,
            ..Default::default()
        };
        assert_eq!(build("svn+ssh", q).unwrap().0, "(?^:svn\\+ssh)");
    }

    #[test]
    fn errors_look_like_perl() {
        assert_eq!(
            build("(set|get)_user_(id|(username)", RegexOpts::default()).unwrap_err(),
            "Invalid regex '(set|get)_user_(id|(username)'\nRegex: (set|get)_user_(id|(username)\n                      ^---HERE Unmatched ( in regex"
        );
    }

    #[test]
    fn lowercase_detection() {
        for lc in [
            "foo",
            "\\Afoo",
            "\\x4Ffoo",
            "(?<name>foo)",
            "\\k<Name>foo",
            "\\p{Lu}x",
            "\\\\foo",
        ] {
            assert!(is_lowercase(lc.as_bytes()), "{lc}");
        }
        for uc in ["Foo", "\\\\Afoo", "\\p{lu}Foo", "\\xZZ"] {
            assert!(!is_lowercase(uc.as_bytes()), "{uc}");
        }
    }

    #[test]
    fn combinations() {
        let o = RegexOpts::default();
        let all = build_all_regexes(
            b"c",
            &[b"a".to_vec(), b"b".to_vec()],
            &[],
            &[b"x".to_vec()],
            &o,
        )
        .unwrap();
        assert_eq!(lossy(&all.re_match), "(?=.*(?^:a))(?=.*(?^:b)).*(?^:c)");
        assert_eq!(lossy(&all.re_hilite), "(?^:a)|(?^:b)|(?^:c)");
        assert_eq!(lossy(&all.re_not.unwrap()), "(?^:x)");
        let all = build_all_regexes(b"a$", &[], &[b"b".to_vec()], &[], &o).unwrap();
        assert_eq!(lossy(&all.re_match), "(?^:a$)|(?^:b)");
        assert_eq!(lossy(&all.re_scan.unwrap()), "|(?^m:b)");
    }
}
