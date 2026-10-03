//! File filters: a port of `App::Ack::File`, `App::Ack::Filter` and its
//! subclasses (`ext`, `is`, `match`, `firstlinematch`, plus the internal
//! `IsPath` and `Default`), `Inverse`, `Collection` and the group classes.

pub mod texttest;

use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;

pub use texttest::is_text;

use crate::bytes::{Bytes, errno_text, lossy, to_path};
use crate::matcher::build::quotemeta;
use crate::matcher::{Matcher, PcreMatcher, perl_syntax};

/// `App::Ack::File`: a file name, with its basename and first line computed
/// on demand.
///
/// A file can carry an open handle, and caches the first block it reads
/// from it. The `-T` test, `firstlinematch:` and the search all share them,
/// so each file is opened once and its start is read once.
pub struct File {
    pub name: Bytes,
    /// Whether it's a regular file (Perl's `-f`), if already known.
    pub is_regular: Option<bool>,
    basename: OnceCell<Bytes>,
    firstliney: OnceCell<Bytes>,
    handle: RefCell<Option<std::fs::File>>,
    /// The first `texttest::BLOCK` bytes, or the error, and whether it came
    /// from opening the file (rather than reading it).
    head: OnceCell<Result<Bytes, (std::io::Error, bool)>>,
}

impl File {
    pub fn new(name: Bytes) -> File {
        File {
            name,
            is_regular: None,
            basename: OnceCell::new(),
            firstliney: OnceCell::new(),
            handle: RefCell::new(None),
            head: OnceCell::new(),
        }
    }

    /// A file that's already open.
    pub fn with_handle(name: Bytes, handle: std::fs::File) -> File {
        let f = File::new(name);
        *f.handle.borrow_mut() = Some(handle);
        f
    }

    /// Records whether this is a regular file.
    pub fn regular(mut self, is_regular: bool) -> File {
        self.is_regular = Some(is_regular);
        self
    }

    /// `(File::Spec->splitpath($name))[2]`
    pub fn basename(&self) -> &[u8] {
        self.basename
            .get_or_init(|| splitpath_file(&self.name).to_vec())
    }

    /// The first block of the file, opening it if needed.
    pub fn head(&self) -> Result<&[u8], &std::io::Error> {
        self.head_or_error().map_err(|(e, _)| e)
    }

    fn head_or_error(&self) -> Result<&[u8], &(std::io::Error, bool)> {
        self.head
            .get_or_init(|| {
                let mut handle = self.handle.borrow_mut();
                if handle.is_none() {
                    *handle =
                        Some(std::fs::File::open(to_path(&self.name)).map_err(|e| (e, true))?);
                }
                texttest::read_block(handle.as_mut().unwrap()).map_err(|e| (e, false))
            })
            .as_deref()
    }

    /// Whether the first block has been read (and so is part of `into_parts`).
    pub fn head_is_read(&self) -> bool {
        self.head.get().is_some_and(|h| h.is_ok())
    }

    /// The open handle (if any) and the bytes already read from it. The
    /// handle is positioned just after those bytes.
    pub fn into_parts(self) -> (Option<std::fs::File>, Bytes) {
        let head = match self.head.into_inner() {
            Some(Ok(h)) => h,
            _ => Vec::new(),
        };
        (self.handle.into_inner(), head)
    }

    /// The first line of the file, or its first 250 bytes, whichever is shorter.
    pub fn firstliney(&self, report_bad_filenames: bool) -> &[u8] {
        self.firstliney.get_or_init(|| match self.head_or_error() {
            Ok(head) => {
                let mut buf = head[..head.len().min(250)].to_vec();
                if let Some(i) = buf.iter().position(|&c| c == b'\r' || c == b'\n') {
                    buf.truncate(i);
                }
                buf
            }
            Err((e, at_open)) => {
                // Perl warns about a failed open only when reporting bad
                // filenames, but always about a failed read.
                if report_bad_filenames || !at_open {
                    crate::output::warn(&format!("{}: {}", lossy(&self.name), errno_text(e)));
                }
                Vec::new()
            }
        })
    }
}

/// The file part of Unix `File::Spec->splitpath`: everything after the last
/// `/`, unless the path ends in `/.` or `/..`.
pub fn splitpath_file(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&c| c == b'/') {
        Some(i) => {
            let file = &path[i + 1..];
            if file == b"." || file == b".." {
                b""
            } else {
                file
            }
        }
        None => path,
    }
}

/// A single filter.
pub enum Filter {
    /// `ext:` matches the extension, case-insensitively.
    Ext {
        extensions: Vec<Bytes>,
        re: PcreMatcher,
    },
    /// `is:` matches the basename exactly.
    Is(Bytes),
    /// Matches the whole path exactly (added alongside `is:` for directories).
    IsPath(Bytes),
    /// `match:` matches the basename against a regex, case-insensitively.
    Match { source: Bytes, re: PcreMatcher },
    /// `firstlinematch:` matches the first line against a regex, case-insensitively.
    FirstLineMatch { source: Bytes, re: PcreMatcher },
    /// The default filter: is it a text file, by Perl's `-T` rules?
    Default,
}

pub type FilterRc = Arc<Filter>;

/// Strips a leading and a trailing `/` from a `match:` regex.
fn strip_slashes(re: &[u8]) -> Bytes {
    let re = re.strip_prefix(b"/").unwrap_or(re);
    re.strip_suffix(b"/").unwrap_or(re).to_vec()
}

/// Compiles `qr/$re/i`. On failure, the error is Perl's own `die` text.
fn case_insensitive(re: &[u8]) -> Result<(Bytes, PcreMatcher), String> {
    if let Err(e) = perl_syntax::check(re) {
        return Err(match e.here {
            Some(here) => format!(
                "{} in regex; marked by <-- HERE in m/{} <-- HERE {}/\n",
                e.why,
                lossy(&re[..here]),
                lossy(&re[here..])
            ),
            None => format!("{}\n", e.why),
        });
    }
    let pattern = [&b"(?^i:"[..], re, b")"].concat();
    match PcreMatcher::from_bytes(&pattern) {
        Ok(m) => Ok((pattern, m)),
        Err(e) => Err(format!("{e} in regex m/{}/\n", lossy(re))),
    }
}

impl Filter {
    /// `App::Ack::Filter->create_filter( $type, @args )`. `Err` is the full
    /// text of the error (from `App::Ack::die`, or from Perl compiling a regex).
    pub fn create(kind: &[u8], args: &[Bytes]) -> Result<Filter, String> {
        let first = args.first().cloned().unwrap_or_default();
        match kind {
            b"ext" => {
                let alternatives: Vec<Bytes> = args.iter().map(|e| quotemeta(e)).collect();
                let pattern = [&b"(?i)[.](?:"[..], &alternatives.join(&b"|"[..]), b")$"].concat();
                let re = PcreMatcher::from_bytes(&pattern).map_err(|e| format!("{e}\n"))?;
                Ok(Filter::Ext {
                    extensions: args.to_vec(),
                    re,
                })
            }
            b"is" => Ok(Filter::Is(first)),
            b"match" => {
                let (source, re) = case_insensitive(&strip_slashes(&first))?;
                Ok(Filter::Match { source, re })
            }
            b"firstlinematch" => {
                let (source, re) = case_insensitive(&strip_slashes(&first))?;
                Ok(Filter::FirstLineMatch { source, re })
            }
            _ => Err(crate::output::die_text(&format!(
                "Unknown filter type '{}'.  Type must be one of: ext, firstlinematch, is, match.",
                lossy(kind)
            ))),
        }
    }

    pub fn matches(&self, file: &File, ctx: &FilterContext) -> bool {
        match self {
            Filter::Ext { re, .. } => re.is_match(&file.name),
            Filter::Is(name) => file.basename() == name.as_slice(),
            Filter::IsPath(name) => file.name == *name,
            Filter::Match { re, .. } => re.is_match(file.basename()),
            Filter::FirstLineMatch { re, .. } => {
                re.is_match(file.firstliney(ctx.report_bad_filenames))
            }
            Filter::Default => is_text(file),
        }
    }
}

/// What filters need to know about the run.
pub struct FilterContext {
    pub report_bad_filenames: bool,
}

/// A filter or its inverse (`App::Ack::Filter::Inverse`). Inverting an
/// inverse gives back the original filter, as in Perl.
#[derive(Clone)]
pub struct FilterRef {
    pub filter: FilterRc,
    pub inverted: bool,
}

/// `App::Ack::Filter::Collection`: matches if any of its filters match.
/// Filters with a group (ext, is, ispath, match) are merged into one lookup
/// per group, as in Perl.
#[derive(Default)]
pub struct Collection {
    extensions: HashSet<Bytes>,
    has_extensions: bool,
    names: HashSet<Bytes>,
    paths: HashSet<Bytes>,
    match_parts: Vec<Bytes>,
    big_re: Option<PcreMatcher>,
    ungrouped: Vec<FilterRc>,
}

impl Collection {
    pub fn add(&mut self, filter: FilterRc) {
        match &*filter {
            Filter::Ext { extensions, .. } => {
                self.has_extensions = true;
                for e in extensions {
                    self.extensions.insert(e.to_ascii_lowercase());
                }
            }
            Filter::Is(name) => {
                self.names.insert(name.clone());
            }
            Filter::IsPath(path) => {
                self.paths.insert(path.clone());
            }
            Filter::Match { source, .. } => {
                // MatchGroup: join('|', map { "(?:$_)" } @regexes)
                self.match_parts.push([&b"(?:"[..], source, b")"].concat());
                self.big_re = PcreMatcher::from_bytes(&self.match_parts.join(&b"|"[..])).ok();
            }
            Filter::FirstLineMatch { .. } | Filter::Default => self.ungrouped.push(filter),
        }
    }

    pub fn matches(&self, file: &File, ctx: &FilterContext) -> bool {
        if self.has_extensions {
            // ExtensionGroup: the text after the last '.' in the whole name.
            if let Some(i) = file.name.iter().rposition(|&c| c == b'.')
                && self
                    .extensions
                    .contains(&file.name[i + 1..].to_ascii_lowercase())
            {
                return true;
            }
        }
        if !self.names.is_empty() && self.names.contains(file.basename()) {
            return true;
        }
        if !self.paths.is_empty() && self.paths.contains(&file.name) {
            return true;
        }
        if let Some(re) = &self.big_re
            && re.is_match(file.basename())
        {
            return true;
        }
        self.ungrouped.iter().any(|f| f.matches(file, ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: FilterContext = FilterContext {
        report_bad_filenames: false,
    };

    fn file(name: &str) -> File {
        File::new(name.as_bytes().to_vec())
    }

    #[test]
    fn splitpath() {
        assert_eq!(splitpath_file(b"t/swamp/foo.pl"), b"foo.pl");
        assert_eq!(splitpath_file(b"foo.pl"), b"foo.pl");
        assert_eq!(splitpath_file(b"t/swamp/"), b"");
        assert_eq!(splitpath_file(b"t/.."), b"");
    }

    #[test]
    fn single_filters() {
        let ext = Filter::create(b"ext", &[b"pl".to_vec(), b"PM".to_vec()]).unwrap();
        assert!(ext.matches(&file("t/Foo.PL"), &CTX));
        assert!(ext.matches(&file("x.pm"), &CTX));
        assert!(!ext.matches(&file("x.pmx"), &CTX));
        let m = Filter::create(b"match", &[b"/[.]min[.]js$/".to_vec()]).unwrap();
        assert!(m.matches(&file("a/jquery.MIN.js"), &CTX));
        assert!(Filter::create(b"bogus", &[]).is_err());
    }

    #[test]
    fn collection_groups() {
        let mut c = Collection::default();
        c.add(Arc::new(
            Filter::create(b"ext", &[b"bak".to_vec()]).unwrap(),
        ));
        c.add(Arc::new(
            Filter::create(b"is", &[b".DS_Store".to_vec()]).unwrap(),
        ));
        c.add(Arc::new(
            Filter::create(b"match", &[b"/~$/".to_vec()]).unwrap(),
        ));
        c.add(Arc::new(
            Filter::create(b"match", &[b"/^#.+#$/".to_vec()]).unwrap(),
        ));
        assert!(c.matches(&file("x/y.BAK"), &CTX));
        assert!(c.matches(&file("x/.DS_Store"), &CTX));
        assert!(c.matches(&file("x/y.txt~"), &CTX));
        assert!(c.matches(&file("x/#y#"), &CTX));
        assert!(!c.matches(&file("x/y.txt"), &CTX));
    }
}
