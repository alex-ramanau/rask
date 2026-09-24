use pcre2::bytes::{Regex, RegexBuilder};

use super::{Error, Match, Matcher};

/// PCRE2 in non-UTF mode, so patterns and subjects are raw bytes, like Perl
/// strings read from a file without an encoding layer.
#[allow(dead_code)] // Used from Phase 1 on.
pub struct PcreMatcher {
    re: Regex,
}

#[allow(dead_code)]
impl PcreMatcher {
    pub fn new(pattern: &str) -> Result<Self, Error> {
        let re = RegexBuilder::new()
            .jit_if_available(true)
            .build(pattern)
            .map_err(|e| Error(e.to_string()))?;
        Ok(Self { re })
    }
}

impl Matcher for PcreMatcher {
    fn find_at(&self, haystack: &[u8], start: usize) -> Result<Option<Match>, Error> {
        self.re
            .find_at(haystack, start)
            .map(|m| {
                m.map(|m| Match {
                    start: m.start(),
                    end: m.end(),
                })
            })
            .map_err(|e| Error(e.to_string()))
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
            m.find_at(b"x foo-bar y", 0).unwrap(),
            Some(Match { start: 2, end: 9 })
        );
    }

    #[test]
    fn matches_non_utf8_bytes() {
        let m = PcreMatcher::new("caf.").unwrap();
        assert!(m.is_match(b"caf\xe9").unwrap());
    }

    #[test]
    fn rejects_code_blocks() {
        // Perl's (?{ code }) must never run (t/no-re-eval.t). PCRE2 doesn't support it.
        assert!(PcreMatcher::new(r#"(?{print "hi"})x"#).is_err());
    }
}
