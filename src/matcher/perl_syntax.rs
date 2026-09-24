//! Emulation of Perl's regex *diagnostics*.
//!
//! PCRE2 matches like Perl (docs/regex-compat.md), but it rejects different
//! patterns, and with different wording. `App::Ack::build_regex` also turns
//! Perl's regex *warnings* into errors. This scanner walks a pattern the way
//! Perl's regcomp does and reports the first error or warning a user is likely
//! to hit, with Perl's message and `<-- HERE` position. Patterns it passes are
//! then compiled by PCRE2, whose own message is the fallback.
//!
//! The expected outputs come from Perl 5.38; see the tests at the bottom.

/// A Perl regex error: the message, and the byte offset just past the text
/// Perl shows before `<-- HERE` (or `None` for messages without a marker).
#[derive(Debug, PartialEq, Eq)]
pub struct PerlError {
    pub why: String,
    pub here: Option<usize>,
}

fn err<T>(why: impl Into<String>, here: usize) -> Result<T, PerlError> {
    Err(PerlError {
        why: why.into(),
        here: Some(here),
    })
}

/// What the previous token was, which decides how `{` and quantifiers are read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prev {
    /// Start of the pattern or of an alternative, or after `(`, `(?flags)` or `^`.
    Start,
    /// After a quantifier.
    Quantifier,
    /// After `\` and a letter (`\d`, `\n`, ...).
    AlphaEscape,
    /// After anything else that can be quantified.
    Atom,
}

/// Escapes that are valid outside a character class.
const ESCAPES: &[u8] = b"aAbBcdDefgGhHkKnNopPrRsStvVwWxXzZ";
/// Escapes that are valid inside a character class.
const CLASS_ESCAPES: &[u8] = b"abcdDefhHnNoprsStvVwWx";

/// If `p[i..]` is a valid `{n}`, `{n,}`, `{n,m}` or `{,m}` quantifier (spaces
/// allowed around the numbers, as in Perl 5.34+), returns its end and bounds.
fn quantifier_at(p: &[u8], i: usize) -> Option<(usize, Option<u64>, Option<u64>)> {
    let mut j = i + 1;
    let skip_spaces = |j: &mut usize| {
        while p.get(*j) == Some(&b' ') {
            *j += 1;
        }
    };
    let number = |j: &mut usize| -> Option<u64> {
        let start = *j;
        while p.get(*j).is_some_and(u8::is_ascii_digit) {
            *j += 1;
        }
        (start < *j).then(|| {
            std::str::from_utf8(&p[start..*j])
                .unwrap()
                .parse()
                .unwrap_or(u64::MAX)
        })
    };
    skip_spaces(&mut j);
    let min = number(&mut j);
    skip_spaces(&mut j);
    let mut max = min;
    if p.get(j) == Some(&b',') {
        j += 1;
        skip_spaces(&mut j);
        max = number(&mut j);
        skip_spaces(&mut j);
        if min.is_none() && max.is_none() {
            return None;
        }
    } else if min.is_none() {
        return None;
    }
    (p.get(j) == Some(&b'}')).then_some((j + 1, min, max))
}

/// Parses a character class starting at `p[i] == b'['`. Returns the index
/// just past the closing `]`.
fn class_at(p: &[u8], i: usize) -> Result<usize, PerlError> {
    let mut j = i + 1;
    if p.get(j) == Some(&b'^') {
        j += 1;
    }
    let first = j;
    // The start of the range being built: (offset, text, is a single char).
    let mut prev: Option<(usize, bool)> = None;
    while j < p.len() {
        let c = p[j];
        if c == b']' && j > first {
            return Ok(j + 1);
        }
        let item_start = j;
        let mut single = true;
        if c == b'[' && matches!(p.get(j + 1), Some(b':' | b'.' | b'=')) {
            let kind = p[j + 1];
            if let Some(end) = p[j + 2..]
                .windows(2)
                .position(|w| w[0] == kind && w[1] == b']')
            {
                let name = &p[j + 2..j + 2 + end];
                j += 2 + end + 2;
                if kind == b':' {
                    let name = name.strip_prefix(b"^").unwrap_or(name);
                    const NAMES: &[&[u8]] = &[
                        b"alpha", b"alnum", b"ascii", b"blank", b"cntrl", b"digit", b"graph",
                        b"lower", b"print", b"punct", b"space", b"upper", b"word", b"xdigit",
                    ];
                    if !NAMES.contains(&name) {
                        return err(format!("POSIX class [:{}:] unknown", lossy(name)), j);
                    }
                }
                single = false;
            } else {
                j += 1;
            }
        } else if c == b'\\' {
            let Some(&e) = p.get(j + 1) else { break };
            j += 2;
            if e.is_ascii_alphabetic() {
                if !CLASS_ESCAPES.contains(&e) {
                    return err(
                        format!(
                            "Unrecognized escape \\{} in character class passed through",
                            e as char
                        ),
                        j,
                    );
                }
                if b"dDsSwWhHvVpPN".contains(&e) {
                    single = false;
                }
                if matches!(e, b'p' | b'P' | b'N' | b'x' | b'o') && p.get(j) == Some(&b'{') {
                    if let Some(close) = p[j..].iter().position(|&b| b == b'}') {
                        j += close + 1;
                    }
                } else if e == b'x' {
                    while j < p.len() && j < item_start + 4 && p[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                } else if e == b'c' {
                    j += 1;
                }
            }
        } else {
            j += 1;
        }

        // A range: the previous item, '-', this item.
        if let Some((start, prev_single)) = prev.take() {
            let text = &p[start..j];
            if !(prev_single && single) {
                let shown = if prev_single {
                    text
                } else {
                    &p[start..item_start]
                };
                return err(format!("False [] range \"{}\"", lossy(shown)), j);
            }
            let lo = class_char(&p[start..]);
            let hi = class_char(&p[item_start..]);
            if let (Some(lo), Some(hi)) = (lo, hi)
                && lo > hi
            {
                return err(format!("Invalid [] range \"{}\"", lossy(text)), j);
            }
            continue;
        }
        if p.get(j) == Some(&b'-') && p.get(j + 1).is_some_and(|&n| n != b']') {
            if !single {
                return err(
                    format!("False [] range \"{}-\"", lossy(&p[item_start..j])),
                    j + 1,
                );
            }
            prev = Some((item_start, true));
            j += 1;
        }
    }
    err("Unmatched [", i + 1)
}

/// The byte value of a simple class member (a literal byte or `\` + non-letter).
fn class_char(p: &[u8]) -> Option<u8> {
    match p {
        [b'\\', c, ..] if !c.is_ascii_alphanumeric() => Some(*c),
        [b'\\', ..] => None,
        [c, ..] => Some(*c),
        [] => None,
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// What a `(` starts.
enum Header {
    /// A group whose body starts at the given offset.
    Open(usize),
    /// `(?flags)`, ending at the given offset.
    Flags(usize),
    /// A complete item such as `(?1)`, `(?&name)` or `(*FAIL)`.
    Atom(usize),
    /// `(?#...)`.
    Comment(usize),
}

fn find_from(p: &[u8], from: usize, byte: u8) -> Option<usize> {
    p.get(from..)?
        .iter()
        .position(|&b| b == byte)
        .map(|n| from + n)
}

fn group_header(p: &[u8], i: usize) -> Result<Header, PerlError> {
    let to_close = |from: usize| find_from(p, from, b')').map(|c| c + 1).unwrap_or(p.len());
    let after = |from: usize, byte: u8| find_from(p, from, byte).map(|c| c + 1).unwrap_or(p.len());

    if p.get(i + 1) == Some(&b'*') {
        // (*VERB), (*VERB:arg) or (*positive_lookahead:...).
        let mut j = i + 2;
        while p
            .get(j)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            j += 1;
        }
        return Ok(match p.get(j) {
            Some(b':') if p[i + 2..j].iter().any(u8::is_ascii_lowercase) => Header::Open(j + 1),
            _ => Header::Atom(to_close(j)),
        });
    }
    if p.get(i + 1) != Some(&b'?') {
        return Ok(Header::Open(i + 1));
    }
    let Some(&c) = p.get(i + 2) else {
        return err("Sequence (? incomplete", i + 2);
    };
    Ok(match c {
        b'#' => match find_from(p, i, b')') {
            Some(close) => Header::Comment(close + 1),
            None => {
                return Err(PerlError {
                    why: format!("Sequence (?#... not terminated in regex m/{}/", lossy(p)),
                    here: None,
                });
            }
        },
        b':' | b'=' | b'!' | b'>' | b'|' => Header::Open(i + 3),
        b'<' if matches!(p.get(i + 3), Some(b'=' | b'!')) => Header::Open(i + 4),
        b'<' => Header::Open(after(i + 3, b'>')),
        b'\'' => Header::Open(after(i + 3, b'\'')),
        b'P' => match p.get(i + 3) {
            Some(b'<') => Header::Open(after(i + 4, b'>')),
            _ => Header::Atom(to_close(i + 3)),
        },
        b'&' | b'R' | b'0'..=b'9' => Header::Atom(to_close(i + 2)),
        b'+' | b'-' if p.get(i + 3).is_some_and(u8::is_ascii_digit) => {
            Header::Atom(to_close(i + 2))
        }
        b'(' => Header::Open(after(i + 3, b')')),
        b'^' | b'-' | b'a'..=b'z' => {
            // Inline flags: (?flags) or (?flags:...).
            let mut j = i + 2;
            while p
                .get(j)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'^' || *b == b'-')
            {
                j += 1;
            }
            match p.get(j) {
                Some(b')') => Header::Flags(j + 1),
                Some(b':') => Header::Open(j + 1),
                None => return err("Sequence (?... not terminated", j),
                _ => Header::Open(j),
            }
        }
        _ => Header::Open(i + 3),
    })
}

/// Checks `p` for the Perl errors and warnings described in the module docs.
pub fn check(p: &[u8]) -> Result<(), PerlError> {
    if let Some(i) = p
        .windows(3)
        .position(|w| w == b"(?{")
        .or_else(|| p.windows(4).position(|w| w == b"(??{"))
    {
        let _ = i;
        return Err(PerlError {
            why: format!(
                "Eval-group not allowed at runtime, use re 'eval' in regex m/{}/",
                lossy(p)
            ),
            here: None,
        });
    }

    let mut groups: Vec<usize> = Vec::new();
    let mut prev = Prev::Start;
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        match c {
            b'\\' => {
                let Some(&e) = p.get(i + 1) else {
                    return Err(PerlError {
                        why: format!("Trailing \\ in regex m/{}/", lossy(p)),
                        here: None,
                    });
                };
                i += 2;
                prev = Prev::Atom;
                if e.is_ascii_alphabetic() {
                    if e == b'C' {
                        return err("\\C no longer supported", i);
                    }
                    if !ESCAPES.contains(&e) {
                        return err(
                            format!("Unrecognized escape \\{} passed through", e as char),
                            i,
                        );
                    }
                    prev = Prev::AlphaEscape;
                    match e {
                        b'x' => {
                            if p.get(i) == Some(&b'{') {
                                match p[i..].iter().position(|&b| b == b'}') {
                                    None => return err("Missing right brace on \\x{}", i + 1),
                                    Some(close) => {
                                        if let Some(bad) = p[i + 1..i + close]
                                            .iter()
                                            .find(|b| !b.is_ascii_hexdigit() && **b != b'_')
                                        {
                                            return err(
                                                format!(
                                                    "Non-hex character '{}' terminates \\x early.  Resolved as \"\\x{{00}}\"",
                                                    *bad as char
                                                ),
                                                i + close + 1,
                                            );
                                        }
                                        i += close + 1;
                                    }
                                }
                            } else {
                                let digits = p[i..]
                                    .iter()
                                    .take(2)
                                    .take_while(|b| b.is_ascii_hexdigit())
                                    .count();
                                if digits == 0
                                    && let Some(&bad) = p.get(i)
                                    && bad.is_ascii_alphanumeric()
                                {
                                    return err(
                                        format!(
                                            "Non-hex character '{}' terminates \\x early.  Resolved as \"\\x00{}\"",
                                            bad as char, bad as char
                                        ),
                                        i + 1,
                                    );
                                }
                                i += digits;
                            }
                            prev = Prev::Atom;
                        }
                        b'o' => {
                            if p.get(i) != Some(&b'{') {
                                return err("Missing braces on \\o{}", i);
                            }
                            if let Some(close) = p[i..].iter().position(|&b| b == b'}') {
                                i += close + 1;
                            }
                            prev = Prev::Atom;
                        }
                        b'c' => {
                            if i >= p.len() {
                                return err(
                                    "Character following \"\\c\" must be printable ASCII",
                                    i,
                                );
                            }
                            i += 1;
                            prev = Prev::Atom;
                        }
                        b'b' | b'B' if p.get(i) == Some(&b'{') => {
                            match p[i..].iter().position(|&b| b == b'}') {
                                None => {
                                    return err(
                                        format!("Missing right brace on \\{}{{}}", e as char),
                                        i + 1,
                                    );
                                }
                                Some(close) => i += close + 1,
                            }
                        }
                        b'p' | b'P' | b'N' | b'g' | b'k' if p.get(i) == Some(&b'{') => {
                            if let Some(close) = p[i..].iter().position(|&b| b == b'}') {
                                i += close + 1;
                            }
                            prev = Prev::Atom;
                        }
                        _ => {}
                    }
                } else if e.is_ascii_digit() {
                    while p.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                }
            }
            b'[' => {
                i = class_at(p, i)?;
                prev = Prev::Atom;
            }
            b'(' => match group_header(p, i)? {
                Header::Open(end) => {
                    groups.push(i);
                    i = end;
                    prev = Prev::Start;
                }
                Header::Flags(end) => {
                    i = end;
                    prev = Prev::Start;
                }
                Header::Atom(end) => {
                    i = end;
                    prev = Prev::Atom;
                }
                Header::Comment(end) => i = end,
            },
            b')' => {
                if groups.pop().is_none() {
                    return err("Unmatched )", i + 1);
                }
                i += 1;
                prev = Prev::Atom;
            }
            b'|' => {
                i += 1;
                prev = Prev::Start;
            }
            b'^' => {
                i += 1;
                prev = Prev::Start;
            }
            b'*' | b'+' | b'?' => {
                match prev {
                    Prev::Start => return err("Quantifier follows nothing", i + 1),
                    Prev::Quantifier => return err("Nested quantifiers", i + 1),
                    _ => {}
                }
                i += 1;
                if matches!(p.get(i), Some(b'+' | b'?')) {
                    i += 1;
                }
                prev = Prev::Quantifier;
            }
            b'{' => {
                if let Some((end, min, max)) = quantifier_at(p, i)
                    && prev != Prev::Start
                {
                    if prev == Prev::Quantifier {
                        return err("Nested quantifiers", i + 1);
                    }
                    if let (Some(min), Some(max)) = (min, max)
                        && min > max
                    {
                        return err("Quantifier {n,m} with n > m can't match", end);
                    }
                    i = end;
                    if matches!(p.get(i), Some(b'+' | b'?')) {
                        i += 1;
                    }
                    prev = Prev::Quantifier;
                } else {
                    match prev {
                        Prev::Start | Prev::Quantifier => {}
                        Prev::AlphaEscape => {
                            return err("Unescaped left brace in regex is illegal here", i + 1);
                        }
                        Prev::Atom => {
                            return err("Unescaped left brace in regex is passed through", i + 1);
                        }
                    }
                    i += 1;
                    prev = Prev::Atom;
                }
            }
            _ => {
                i += 1;
                prev = Prev::Atom;
            }
        }
    }
    if let Some(&open) = groups.last() {
        return err("Unmatched (", open + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (pattern, expected) where expected is None for "Perl accepts it", or
    /// (why, marker offset). From Perl 5.38 via App::Ack::build_regex.
    const CASES: &[(&str, Option<(&str, usize)>)] = &[
        ("(", Some(("Unmatched (", 1))),
        (")", Some(("Unmatched )", 1))),
        ("[", Some(("Unmatched [", 1))),
        ("[a", Some(("Unmatched [", 1))),
        ("[]", Some(("Unmatched [", 1))),
        ("((a", Some(("Unmatched (", 2))),
        ("a)b)", Some(("Unmatched )", 2))),
        ("({", Some(("Unmatched (", 1))),
        (")a{", Some(("Unmatched )", 1))),
        ("(set|get)_user_(id|(username)", Some(("Unmatched (", 16))),
        ("*", Some(("Quantifier follows nothing", 1))),
        ("+foo", Some(("Quantifier follows nothing", 1))),
        ("a|*", Some(("Quantifier follows nothing", 3))),
        ("(?i)*", Some(("Quantifier follows nothing", 5))),
        ("a**", Some(("Nested quantifiers", 3))),
        ("a{2}*", Some(("Nested quantifiers", 5))),
        ("x{1}{2}", Some(("Nested quantifiers", 5))),
        (
            "a{3,1}",
            Some(("Quantifier {n,m} with n > m can't match", 6)),
        ),
        (
            "foo{",
            Some(("Unescaped left brace in regex is passed through", 4)),
        ),
        (
            "a{1",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{1,",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{1,2",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{b}",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{}",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{,}",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "a{-1}",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "-{",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            ".{",
            Some(("Unescaped left brace in regex is passed through", 2)),
        ),
        (
            "[a]{",
            Some(("Unescaped left brace in regex is passed through", 4)),
        ),
        (
            "(a){",
            Some(("Unescaped left brace in regex is passed through", 4)),
        ),
        (
            "a(?:x){",
            Some(("Unescaped left brace in regex is passed through", 7)),
        ),
        (
            "a{1}b{",
            Some(("Unescaped left brace in regex is passed through", 6)),
        ),
        (
            "\\x41{",
            Some(("Unescaped left brace in regex is passed through", 5)),
        ),
        (
            "\\.{",
            Some(("Unescaped left brace in regex is passed through", 3)),
        ),
        (
            "\\{{",
            Some(("Unescaped left brace in regex is passed through", 3)),
        ),
        (
            "\u{e9}{",
            Some(("Unescaped left brace in regex is passed through", 3)),
        ),
        (
            "\\d{",
            Some(("Unescaped left brace in regex is illegal here", 3)),
        ),
        (
            "\\w{",
            Some(("Unescaped left brace in regex is illegal here", 3)),
        ),
        (
            "\\n{",
            Some(("Unescaped left brace in regex is illegal here", 3)),
        ),
        ("\\b{", Some(("Missing right brace on \\b{}", 3))),
        ("\\y", Some(("Unrecognized escape \\y passed through", 2))),
        ("\\E", Some(("Unrecognized escape \\E passed through", 2))),
        ("a\\Eb", Some(("Unrecognized escape \\E passed through", 3))),
        ("\\Q", Some(("Unrecognized escape \\Q passed through", 2))),
        (
            "[\\y]",
            Some((
                "Unrecognized escape \\y in character class passed through",
                3,
            )),
        ),
        ("\\C", Some(("\\C no longer supported", 2))),
        ("\\o", Some(("Missing braces on \\o{}", 2))),
        (
            "\\c",
            Some(("Character following \"\\c\" must be printable ASCII", 2)),
        ),
        (
            "\\xZ",
            Some((
                "Non-hex character 'Z' terminates \\x early.  Resolved as \"\\x00Z\"",
                3,
            )),
        ),
        ("\\x{41", Some(("Missing right brace on \\x{}", 3))),
        ("[z-a]", Some(("Invalid [] range \"z-a\"", 4))),
        ("[a-\\d]", Some(("False [] range \"a-\\d\"", 5))),
        ("[\\d-z]", Some(("False [] range \"\\d-\"", 4))),
        ("(?", Some(("Sequence (? incomplete", 2))),
        ("a(?", Some(("Sequence (? incomplete", 3))),
        ("(?i", Some(("Sequence (?... not terminated", 3))),
        ("[[:foo:]]", Some(("POSIX class [:foo:] unknown", 8))),
        ("{foo", None),
        ("{", None),
        ("{1}", None),
        ("x{2}", None),
        ("a{,3}", None),
        ("a{ ,3}", None),
        ("a{1 ,2}", None),
        ("a{ 1 }", None),
        ("a{2}{", None),
        ("a{1,2}{", None),
        ("a*{", None),
        ("^{", None),
        ("a|{", None),
        ("x\\{", None),
        ("[{]", None),
        ("a}", None),
        ("a]", None),
        ("[]]", None),
        ("[^]]", None),
        ("[a-]", None),
        ("x++", None),
        ("a?+", None),
        ("a{2}+", None),
        ("\\cA", None),
        ("\\x", None),
        ("\\x4", None),
        ("\\K", None),
        ("\\N", None),
        ("\\%", None),
        ("(?^i:a)", None),
        ("(?#c)a", None),
        ("(?x) a # c", None),
        ("(?P<n>a)", None),
        ("(?<n>a)", None),
        ("(?:a)", None),
        ("(?=a)b", None),
        ("(?|a|b)", None),
        ("(*FAIL)", None),
        ("(a)\\g{1}", None),
        ("svn+ssh", None),
        ("(?:^|\\b|\\s)\\K(?:foo)(?=\\s|\\b|$)", None),
    ];

    #[test]
    fn matches_perl() {
        let mut failures = Vec::new();
        for (pat, want) in CASES {
            let got = check(pat.as_bytes());
            let got = match got {
                Ok(()) => None,
                Err(PerlError { why, here }) => Some((why, here.unwrap_or(usize::MAX))),
            };
            let want = want.map(|(w, h)| (w.to_string(), h));
            if got != want {
                failures.push(format!("{pat:?}: got {got:?}, want {want:?}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn eval_groups() {
        for p in ["(?{ 1 })", "(??{ 1 })"] {
            let e = check(p.as_bytes()).unwrap_err();
            assert!(
                e.why
                    .starts_with("Eval-group not allowed at runtime, use re 'eval'")
            );
        }
    }
}
