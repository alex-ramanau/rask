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

    /// (euid, egid, supplementary groups), looked up once.
    static IDS: std::sync::OnceLock<(u32, u32, Vec<u32>)> = std::sync::OnceLock::new();

    let Some(meta) = meta else { return false };
    let (euid, egid, groups) = IDS.get_or_init(|| {
        let groups = getgroups()
            .map(|g| g.iter().map(|g| g.as_raw()).collect())
            .unwrap_or_default();
        (geteuid().as_raw(), getegid().as_raw(), groups)
    });
    let euid = *euid;
    if euid == 0 {
        // Root can read anything.
        return true;
    }
    let mode = meta.mode();
    if meta.uid() == euid {
        return mode & 0o400 != 0;
    }
    let gid = meta.gid();
    let in_group = *egid == gid || groups.contains(&gid);
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
