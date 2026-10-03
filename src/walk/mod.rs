//! File enumeration: a port of `File::Next::files` and `from_file` (File::Next
//! 1.18), and the Unix `File::Spec` path functions they use.
//!
//! The walk is depth-first and pre-order. Each directory's entries are put at
//! the front of the queue in `readdir` order (or sorted by name with
//! `--sort-files`), so the output order matches Perl ack exactly.

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read};

use crate::bytes::{Bytes, errno_text, from_os, lossy, to_path};

/// `File::Spec::Unix->canonpath`
pub fn canonpath(path: &[u8]) -> Bytes {
    let mut p: Bytes = Vec::with_capacity(path.len());
    // xx////xx -> xx/xx
    for &c in path {
        if !(c == b'/' && p.last() == Some(&b'/')) {
            p.push(c);
        }
    }
    // xx/././xx -> xx/xx: s{(?:/\.)+(?:/|\z)}{/}g
    let mut out: Bytes = Vec::with_capacity(p.len());
    let mut i = 0;
    while i < p.len() {
        let mut j = i;
        while p.get(j) == Some(&b'/') && p.get(j + 1) == Some(&b'.') {
            j += 2;
        }
        if j > i && (j == p.len() || p[j] == b'/') {
            out.push(b'/');
            i = if j == p.len() { j } else { j + 1 };
            continue;
        }
        out.push(p[i]);
        i += 1;
    }
    let mut p = out;
    // ./xx -> xx
    if p != b"./" {
        while p.starts_with(b"./") {
            p.drain(..2);
        }
    }
    // /../../xx -> xx
    if p.starts_with(b"/../") {
        let mut k = 1;
        while p[k..].starts_with(b"../") {
            k += 3;
        }
        p.drain(1..k);
    }
    // /.. -> /
    if p == b"/.." {
        p = b"/".to_vec();
    }
    // xx/ -> xx
    if p.len() > 1 && p.ends_with(b"/") {
        p.pop();
    }
    p
}

/// `File::Spec::Unix->catdir( $dir, $file )`
pub fn catdir(dir: &[u8], file: &[u8]) -> Bytes {
    canonpath(&[dir, b"/", file, b"/"].concat())
}

/// `File::Spec::Unix->catfile( @parts )`
fn catfile(parts: &[&[u8]]) -> Bytes {
    let (file, dirs) = parts.split_last().expect("catfile needs a part");
    let file = canonpath(file);
    if dirs.is_empty() {
        return file;
    }
    let mut dir = canonpath(&[dirs.join(&b"/"[..]).as_slice(), b"/"].concat());
    if !dir.ends_with(b"/") {
        dir.push(b'/');
    }
    dir.extend_from_slice(&file);
    dir
}

/// `File::Next::reslash`
pub fn reslash(path: &[u8]) -> Bytes {
    // Perl's split drops trailing empty fields.
    let mut parts: Vec<&[u8]> = path.split(|&c| c == b'/').collect();
    while parts.last() == Some(&&b""[..]) {
        parts.pop();
    }
    if parts.len() < 2 {
        return path.to_vec();
    }
    catfile(&parts)
}

/// One candidate: `[ $dirname, $file, $fullpath, $is_dir, $is_file, $is_fifo ]`.
pub struct Entry {
    pub dirname: Option<Bytes>,
    pub fullpath: Bytes,
    is_dir: bool,
    is_file: bool,
    is_fifo: bool,
}

fn file_kind(meta: Option<fs::Metadata>, path: &[u8]) -> (bool, bool, bool) {
    let dev_fd = path.starts_with(b"/dev/fd");
    match meta {
        Some(m) => {
            let ft = m.file_type();
            (ft.is_dir(), ft.is_file(), is_fifo(&ft) || dev_fd)
        }
        None => (false, false, dev_fd),
    }
}

#[cfg(unix)]
fn is_fifo(ft: &fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt;
    ft.is_fifo()
}

#[cfg(not(unix))]
fn is_fifo(_: &fs::FileType) -> bool {
    false
}

pub type FileFilter<'a> = Box<dyn FnMut(&Entry) -> bool + 'a>;
pub type DescendFilter<'a> = Box<dyn Fn(&[u8]) -> bool + 'a>;
pub type ErrorHandler<'a> = Box<dyn Fn(&str) + 'a>;

pub struct WalkOptions<'a> {
    pub file_filter: Option<FileFilter<'a>>,
    pub descend_filter: Option<DescendFilter<'a>>,
    pub error_handler: ErrorHandler<'a>,
    pub sort_files: bool,
    pub follow_symlinks: bool,
}

/// The iterator returned by `File::Next::files`.
pub struct Files<'a> {
    opts: WalkOptions<'a>,
    queue: VecDeque<Entry>,
}

impl<'a> Files<'a> {
    pub fn new(opts: WalkOptions<'a>, starts: &[Bytes]) -> Files<'a> {
        let queue = starts
            .iter()
            .map(|s| {
                let start = reslash(s);
                let (is_dir, is_file, is_fifo) =
                    file_kind(fs::metadata(to_path(&start)).ok(), &start);
                Entry {
                    dirname: if is_dir { Some(start.clone()) } else { None },
                    fullpath: start,
                    is_dir,
                    is_file,
                    is_fifo,
                }
            })
            .collect();
        Files { opts, queue }
    }

    fn candidate_files(&self, dirname: &[u8]) -> Vec<Entry> {
        let reader = match fs::read_dir(to_path(dirname)) {
            Ok(r) => r,
            Err(e) => {
                (self.opts.error_handler)(&format!("{}: {}", lossy(dirname), errno_text(&e)));
                return Vec::new();
            }
        };
        let mut entries = Vec::new();
        for dent in reader.flatten() {
            let name = from_os(&dent.file_name());
            let fullpath = catdir(dirname, &name);
            let path = to_path(&fullpath);
            let meta = if self.opts.follow_symlinks {
                fs::metadata(&path).ok()
            } else {
                match fs::symlink_metadata(&path) {
                    Ok(m) if m.file_type().is_symlink() => continue,
                    m => m.ok(),
                }
            };
            let (is_dir, is_file, is_fifo) = file_kind(meta, &fullpath);
            if is_dir
                && let Some(descend) = &self.opts.descend_filter
                && !descend(&fullpath)
            {
                continue;
            }
            entries.push((
                name,
                Entry {
                    dirname: Some(dirname.to_vec()),
                    fullpath,
                    is_dir,
                    is_file,
                    is_fifo,
                },
            ));
        }
        if self.opts.sort_files {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
        }
        entries.into_iter().map(|(_, e)| e).collect()
    }
}

impl Iterator for Files<'_> {
    type Item = Bytes;

    fn next(&mut self) -> Option<Bytes> {
        while let Some(entry) = self.queue.pop_front() {
            if entry.is_file || entry.is_fifo {
                if let Some(filter) = &mut self.opts.file_filter
                    && !filter(&entry)
                {
                    continue;
                }
                return Some(entry.fullpath);
            }
            if entry.is_dir {
                let children = self.candidate_files(&entry.fullpath);
                for child in children.into_iter().rev() {
                    self.queue.push_front(child);
                }
            }
        }
        None
    }
}

/// `File::Next::from_file`: the list of files to search, one per line, from
/// a file or (for `-`) stdin. `Err` is the message for the error handler.
pub fn from_file<'a>(filename: &[u8], warn: ErrorHandler<'a>) -> Result<FromFile<'a>, String> {
    let reader: Box<dyn BufRead> = if filename == b"-" {
        Box::new(BufReader::new(std::io::stdin()))
    } else {
        match fs::File::open(to_path(filename)) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => {
                return Err(format!(
                    "Unable to open {}: {}",
                    lossy(filename),
                    errno_text(&e)
                ));
            }
        }
    };
    Ok(FromFile { reader, warn })
}

pub struct FromFile<'a> {
    reader: Box<dyn BufRead>,
    warn: ErrorHandler<'a>,
}

impl Iterator for FromFile<'_> {
    type Item = Bytes;

    fn next(&mut self) -> Option<Bytes> {
        loop {
            let mut line = Vec::new();
            match self
                .reader
                .by_ref()
                .take(u64::MAX)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            // next unless $fullpath =~ /./  (any character but a newline)
            if line.is_empty() {
                continue;
            }
            let (_, is_file, is_fifo) = file_kind(fs::metadata(to_path(&line)).ok(), b"");
            if !(is_file || is_fifo) {
                (self.warn)(&format!("{}: No such file", lossy(&line)));
                continue;
            }
            return Some(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(b: Bytes) -> String {
        String::from_utf8(b).unwrap()
    }

    #[test]
    fn file_spec() {
        assert_eq!(s(canonpath(b"a//b/./c/")), "a/b/c");
        assert_eq!(s(canonpath(b"./a")), "a");
        assert_eq!(s(canonpath(b"./")), ".");
        assert_eq!(s(canonpath(b"/../x")), "/x");
        assert_eq!(s(canonpath(b"a/.")), "a");
        assert_eq!(s(catdir(b".", b"foo")), "foo");
        assert_eq!(s(catdir(b"t/text", b"a.txt")), "t/text/a.txt");
        assert_eq!(s(catdir(b"/", b"x")), "/x");
        assert_eq!(s(reslash(b"t/text/")), "t/text");
        assert_eq!(s(reslash(b"./t")), "./t");
        assert_eq!(s(reslash(b"./t/x.txt")), "t/x.txt");
        assert_eq!(s(reslash(b"/abs/path")), "/abs/path");
        assert_eq!(s(reslash(b"t")), "t");
    }
}
