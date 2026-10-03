//! Perl's `-r` and `-R` file tests, as ack's file filter uses them:
//! `-r _` checks the mode bits from the last stat against the effective
//! uid and gids, and when that fails, `-R` under `use filetest 'access'`
//! asks the kernel with `access(2)` (which also knows about ACLs).

use std::fs::Metadata;
use std::path::Path;

/// `-r _`: Perl's `cando(S_IRUSR, effective)` on the stat buffer. False if
/// the stat failed.
#[cfg(unix)]
pub fn readable(meta: Option<&Metadata>) -> bool {
    use rustix::process::{getegid, geteuid, getgroups};
    use std::os::unix::fs::MetadataExt;

    let Some(meta) = meta else { return false };
    let euid = geteuid().as_raw();
    if euid == 0 {
        // Root can read anything.
        return true;
    }
    let mode = meta.mode();
    if meta.uid() == euid {
        return mode & 0o400 != 0;
    }
    let gid = meta.gid();
    let in_group = getegid().as_raw() == gid
        || getgroups().is_ok_and(|groups| groups.iter().any(|g| g.as_raw() == gid));
    if in_group {
        return mode & 0o040 != 0;
    }
    mode & 0o004 != 0
}

#[cfg(not(unix))]
pub fn readable(meta: Option<&Metadata>) -> bool {
    meta.is_some()
}

/// `-R $name` under `use filetest 'access'`: `access( $name, R_OK )`.
#[cfg(unix)]
pub fn access_readable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::READ_OK).is_ok()
}

#[cfg(not(unix))]
pub fn access_readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}
