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

/// `_check_for_ackrc`: the `.ackrc` or `_ackrc` in `dir`. Dies if both exist.
fn check_for_ackrc(dir: &[u8]) -> Option<Bytes> {
    let files: Vec<Bytes> = [&b".ackrc"[..], b"_ackrc"]
        .iter()
        .map(|name| {
            let mut p = canonpath(dir);
            if !p.ends_with(b"/") {
                p.push(b'/');
            }
            p.extend_from_slice(name);
            p
        })
        .filter(|p| is_file(p))
        .collect();
    if files.len() > 1 {
        crate::output::die(&format!(
            "{} contains both .ackrc and _ackrc. Please remove one of those files.",
            lossy(&canonpath(dir))
        ));
    }
    files.into_iter().next()
}

/// `_remove_redundancies`: drop files already seen (by inode on Unix).
fn remove_redundancies(configs: Vec<ConfigFile>) -> Vec<ConfigFile> {
    let mut seen = HashSet::new();
    configs
        .into_iter()
        .filter(|c| {
            let path = to_path(&c.path);
            let mut key = match std::fs::canonicalize(&path) {
                Ok(real) => path_bytes(&real),
                Err(_) => c.path.clone(),
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(m) = std::fs::metadata(to_path(&key)) {
                    key = format!("{}:{}", m.dev(), m.ino()).into_bytes();
                }
            }
            seen.insert(key)
        })
        .collect()
}

pub fn find_config_files() -> Vec<ConfigFile> {
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
            if let Some(home) = std::env::var_os("HOME").map(|h| from_os(&h))
                && let Some(f) = check_for_ackrc(&home)
            {
                configs.push(ConfigFile {
                    path: f,
                    project: false,
                });
            }
        }
    }

    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    // File::Spec->splitdir, then look in each directory from cwd up to /.
    let cwd = path_bytes(&cwd);
    let mut dirs: Vec<&[u8]> = cwd.split(|&c| c == b'/').collect();
    while !dirs.is_empty() {
        let dir = dirs.join(&b"/"[..]);
        let dir = if dir.is_empty() { b"/".to_vec() } else { dir };
        if let Some(f) = check_for_ackrc(&dir) {
            configs.push(ConfigFile {
                path: f,
                project: true,
            });
            break;
        }
        dirs.pop();
    }
    remove_redundancies(configs)
}
