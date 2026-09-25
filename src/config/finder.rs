//! `App::Ack::ConfigFinder`: which ackrc files to read.
//!
//! In order: the global ackrc, the user's (`$ACKRC` or `~/.ackrc`), and the
//! project's (the nearest `.ackrc` or `_ackrc` from the current directory
//! up). The same file is only read once.

use std::collections::HashSet;

use crate::bytes::{Bytes, from_os, lossy, path_bytes, to_path};
use crate::walk::canonpath;

pub struct ConfigFile {
    pub path: Bytes,
    pub project: bool,
}

fn is_file(path: &[u8]) -> bool {
    std::fs::metadata(to_path(path))
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// The directory separator `File::Spec` joins with.
const SEP: u8 = if cfg!(windows) { b'\\' } else { b'/' };

fn is_sep(c: u8) -> bool {
    c == b'/' || (cfg!(windows) && c == b'\\')
}

/// `File::Spec->catdir( $dir )`: Unix canonpath, or on Windows the path
/// with forward slashes turned into backslashes and a trailing one removed
/// (except after a drive letter).
fn catdir(dir: &[u8]) -> Bytes {
    if !cfg!(windows) {
        return canonpath(dir);
    }
    let mut p: Bytes = dir
        .iter()
        .map(|&c| if c == b'/' { b'\\' } else { c })
        .collect();
    while p.len() > 1 && p.ends_with(b"\\") && !p.ends_with(b":\\") {
        p.pop();
    }
    p
}

/// `_check_for_ackrc`: the `.ackrc` or `_ackrc` in `dir`. `Err` (the
/// message for `App::Ack::die`) if both exist.
fn check_for_ackrc(dir: &[u8]) -> Result<Option<Bytes>, String> {
    let files: Vec<Bytes> = [&b".ackrc"[..], b"_ackrc"]
        .iter()
        .map(|name| {
            let mut p = catdir(dir);
            if !p.last().is_some_and(|&c| is_sep(c)) {
                p.push(SEP);
            }
            p.extend_from_slice(name);
            p
        })
        .filter(|p| is_file(p))
        .collect();
    if files.len() > 1 {
        return Err(format!(
            "{} contains both .ackrc and _ackrc. Please remove one of those files.",
            lossy(&catdir(dir))
        ));
    }
    Ok(files.into_iter().next())
}

/// On Unix, files are the same if they have the same inode.
#[cfg(unix)]
fn unique_key(path: Bytes) -> Bytes {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(to_path(&path)) {
        Ok(m) => format!("{}:{}", m.dev(), m.ino()).into_bytes(),
        Err(_) => path,
    }
}

#[cfg(not(unix))]
fn unique_key(path: Bytes) -> Bytes {
    path
}

/// `_remove_redundancies`: drop files already seen (by inode on Unix).
fn remove_redundancies(configs: Vec<ConfigFile>) -> Vec<ConfigFile> {
    let mut seen = HashSet::new();
    configs
        .into_iter()
        .filter(|c| {
            let path = to_path(&c.path);
            let key = match std::fs::canonicalize(&path) {
                Ok(real) => path_bytes(&real),
                Err(_) => c.path.clone(),
            };
            seen.insert(unique_key(key))
        })
        .collect()
}

/// `find_config_files`, from the current directory. Dies on an error.
pub fn find_config_files() -> Vec<ConfigFile> {
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    find_config_files_from(&cwd).unwrap_or_else(|e| crate::output::die(&e))
}

/// `find_config_files` as if the current directory were `cwd`. `Err` is the
/// message for `App::Ack::die`.
pub fn find_config_files_from(cwd: &std::path::Path) -> Result<Vec<ConfigFile>, String> {
    let mut configs = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramData", "APPDATA"] {
            if let Some(dir) = std::env::var_os(var) {
                let mut p = from_os(&dir);
                p.extend_from_slice(b"\\ackrc");
                configs.push(ConfigFile {
                    path: p,
                    project: false,
                });
            }
        }
    } else {
        configs.push(ConfigFile {
            path: b"/etc/ackrc".to_vec(),
            project: false,
        });
    }

    match crate::env::var("ACKRC").filter(|a| !a.is_empty() && a != b"0" && is_file(a)) {
        Some(ackrc) => configs.push(ConfigFile {
            path: ackrc,
            project: false,
        }),
        None => {
            if let Some(home) = crate::env::var("HOME")
                && let Some(f) = check_for_ackrc(&home)?
            {
                configs.push(ConfigFile {
                    path: f,
                    project: false,
                });
            }
        }
    }

    // File::Spec->splitdir, then look in each directory from cwd up to the root.
    let cwd = path_bytes(cwd);
    let mut dirs: Vec<&[u8]> = cwd.split(|&c| is_sep(c)).collect();
    while !dirs.is_empty() {
        let dir = dirs.join(&[SEP][..]);
        let dir = if dir.is_empty() { vec![SEP] } else { dir };
        if let Some(f) = check_for_ackrc(&dir)? {
            configs.push(ConfigFile {
                path: f,
                project: true,
            });
            break;
        }
        dirs.pop();
    }
    Ok(remove_redundancies(configs))
}
