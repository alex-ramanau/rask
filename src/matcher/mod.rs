//! Regex matching over raw bytes (plan D1, D2).
//!
//! The rest of the program talks to [`Matcher`], so a faster engine can be
//! added for patterns that don't need Perl-only features (Phase 5).

mod pcre;

#[allow(unused_imports)] // Used from Phase 1 on.
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

#[allow(dead_code)] // Used from Phase 1 on.
pub trait Matcher {
    /// Find the first match in `haystack` that starts at or after `start`.
    ///
    /// Look-behind and `\b` may inspect bytes before `start`, as Perl's `pos()`
    /// does when iterating with `m//g`.
    fn find_at(&self, haystack: &[u8], start: usize) -> Result<Option<Match>, Error>;

    fn is_match(&self, haystack: &[u8]) -> Result<bool, Error> {
        Ok(self.find_at(haystack, 0)?.is_some())
    }
}
