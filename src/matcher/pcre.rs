use std::sync::OnceLock;

use pcre2::bytes::{Regex, RegexBuilder};

use super::{Error, Match, Matcher, pattern_to_pcre};

/// PCRE2 in non-UTF mode, so patterns and subjects are raw bytes, like Perl
/// strings read from a file without an encoding layer.
pub struct PcreMatcher {
    re: Regex,
    pattern: String,
    /// Same pattern without JIT, built on first use. The JIT has a small
    /// fixed stack and errors out on long subjects (e.g. `(a|b)*` over a
    /// 5 KB line); the interpreter keeps its frames on the heap and copes.
    interp: OnceLock<Option<Regex>>,
}

impl PcreMatcher {
    pub fn new(pattern: &str) -> Result<Self, Error> {
        let re = RegexBuilder::new()
            .jit_if_available(true)
            .build(pattern)
            .map_err(|e| Error(e.to_string()))?;
        Ok(Self {
            re,
            pattern: pattern.to_string(),
            interp: OnceLock::new(),
        })
    }

    fn interp(&self) -> Option<&Regex> {
        self.interp
            .get_or_init(|| RegexBuilder::new().build(&self.pattern).ok())
            .as_ref()
    }

    pub fn from_bytes(pattern: &[u8]) -> Result<Self, Error> {
        Self::new(&pattern_to_pcre(pattern))
    }

    /// The capture groups of the first match at or after `start`: index 0 is
    /// the whole match, and groups that didn't participate are `None`.
    pub fn captures_at(&self, haystack: &[u8], start: usize) -> Vec<Option<(usize, usize)>> {
        let mut locs = self.re.capture_locations();
        let mut res = self.re.captures_read_at(&mut locs, haystack, start);
        if res.is_err()
            && let Some(re) = self.interp()
        {
            locs = re.capture_locations();
            res = re.captures_read_at(&mut locs, haystack, start);
        }
        match res {
            Ok(Some(_)) => (0..locs.len()).map(|i| locs.get(i)).collect(),
            _ => Vec::new(),
        }
    }

    /// `find_at` returning a plain tuple; used by internal helpers.
    pub fn find_at_raw(&self, haystack: &[u8], start: usize) -> Option<(usize, usize)> {
        self.find_at(haystack, start).map(|m| (m.start, m.end))
    }
}

impl Matcher for PcreMatcher {
    fn find_at(&self, haystack: &[u8], start: usize) -> Option<Match> {
        // On a JIT error (stack limit), retry on the interpreter. An error
        // there too (match/heap limit) counts as no match: Perl's regex
        // engine gives up in the same situations.
        self.re
            .find_at(haystack, start)
            .or_else(|_| match self.interp() {
                Some(re) => re.find_at(haystack, start),
                None => Ok(None),
            })
            .ok()
            .flatten()
            .map(|m| Match {
                start: m.start(),
                end: m.end(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perl_only_syntax_compiles() {
        // The -w wrapping from App::Ack::build_regex needs \K and look-ahead.
        let m = PcreMatcher::new(r"(?:^|\b|\s)\K(?:foo-bar)(?=\s|\b|$)").unwrap();
        assert_eq!(
            m.find_at(b"x foo-bar y", 0),
            Some(Match { start: 2, end: 9 })
        );
    }

    #[test]
    fn matches_non_utf8_bytes() {
        let m = PcreMatcher::new("caf.").unwrap();
        assert!(m.is_match(b"caf\xe9"));
        let m = PcreMatcher::from_bytes(b"caf\\\xe9").unwrap();
        assert!(m.is_match(b"un caf\xe9"));
        assert!(!m.is_match(b"un cafe"));
    }

    #[test]
    fn rejects_code_blocks() {
        // Perl's (?{ code }) must never run (t/no-re-eval.t). PCRE2 doesn't support it.
        assert!(PcreMatcher::new(r#"(?{print "hi"})x"#).is_err());
    }

    #[test]
    fn long_line_past_jit_stack() {
        // Regression: the default JIT stack gave up here and reported no match.
        let line = vec![b'a'; 200_000];
        let m = PcreMatcher::new("^(?:a|b)*$").unwrap();
        assert!(m.is_match(&line));
        assert_eq!(m.captures_at(&line, 0).len(), 1);
    }

    #[test]
    fn perl_qr_syntax() {
        let m = PcreMatcher::new("(?^:(?i)foo)|(?^m:^bar)").unwrap();
        assert!(m.is_match(b"FOO"));
        assert!(m.is_match(b"x\nbar"));
    }
}
