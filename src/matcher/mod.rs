//! Regex matching over raw bytes (plan D1, D2).
//!
//! The rest of the program talks to [`Matcher`], so a faster engine can be
//! added for patterns that don't need Perl-only features (Phase 5).

pub mod build;
mod pcre;
pub mod perl_syntax;

pub use pcre::PcreMatcher;

/// Byte offsets of one match, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub trait Matcher {
    /// Find the first match in `haystack` that starts at or after `start`.
    ///
    /// Look-behind and `\b` may inspect bytes before `start`, as Perl's `pos()`
    /// does when iterating with `m//g`.
    fn find_at(&self, haystack: &[u8], start: usize) -> Option<Match>;

    fn is_match(&self, haystack: &[u8]) -> bool {
        self.find_at(haystack, 0).is_some()
    }
}

/// PCRE2's API takes the pattern as `&str`, but in non-UTF mode it treats it
/// as bytes. A pattern that isn't UTF-8 has its high bytes written as `\x{HH}`
/// (dropping a backslash that escaped the byte, as `quotemeta` does).
pub fn pattern_to_pcre(pattern: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(pattern) {
        return s.to_string();
    }
    let mut out = String::with_capacity(pattern.len() * 2);
    let mut escaped = false;
    for &b in pattern {
        if b >= 0x80 {
            if escaped {
                out.pop();
            }
            out.push_str(&format!("\\x{{{b:02x}}}"));
            escaped = false;
        } else {
            escaped = !escaped && b == b'\\';
            out.push(b as char);
        }
    }
    out
}
