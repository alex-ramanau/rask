//! Byte strings. Perl ack works on bytes throughout: arguments, filenames and
//! file contents are never decoded, so rask doesn't decode them either.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub type Bytes = Vec<u8>;

#[cfg(unix)]
pub fn from_os(s: &OsStr) -> Bytes {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
pub fn from_os(s: &OsStr) -> Bytes {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
pub fn to_os(b: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    OsStr::from_bytes(b).to_os_string()
}

#[cfg(not(unix))]
pub fn to_os(b: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(b).into_owned())
}

pub fn to_path(b: &[u8]) -> PathBuf {
    PathBuf::from(to_os(b))
}

pub fn path_bytes(p: &Path) -> Bytes {
    from_os(p.as_os_str())
}

pub fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Perl's `$!` text for an I/O error: the strerror message, without Rust's
/// " (os error N)" suffix.
pub fn errno_text(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}
