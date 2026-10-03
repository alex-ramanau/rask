//! Perl's `-T` file test (`pp_fttext` in Perl 5.38's pp_sys.c), which
//! `App::Ack::Filter::Default` uses to skip binary files. `DESIGN.md` makes
//! this heuristic part of ack's behaviour, so it's ported exactly.

use std::io::Read;

use super::File;

/// Perl reads this much of the file.
pub const BLOCK: usize = 512;

/// Reads up to `BLOCK` bytes from the start of `f`.
pub fn read_block(f: &mut std::fs::File) -> Result<Vec<u8>, std::io::Error> {
    let mut buf = vec![0u8; BLOCK];
    let mut len = 0;
    while len < BLOCK {
        match f.read(&mut buf[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(len);
    Ok(buf)
}

/// `-T $file->name`: true if the file looks like text. Files that can't be
/// opened or read aren't. An empty file is anything. (Perl also says a
/// directory isn't text, but only files get here.)
pub fn is_text(file: &File) -> bool {
    match file.head() {
        Ok([]) => true,
        Ok(head) => looks_like_text(head),
        Err(_) => false,
    }
}

/// The textiness decision on the first block of a file.
pub fn looks_like_text(buf: &[u8]) -> bool {
    let mut len = buf.len();
    // Ignore a trailing ^Z on short files (DOSISH builds only).
    if cfg!(windows) && len < BLOCK && buf[len - 1] == 26 {
        len -= 1;
    }
    let buf = &buf[..len];

    if !buf.is_ascii() && is_utf8_fixed_width(buf) {
        return true;
    }

    let mut odd = 0usize;
    for &c in buf {
        if c == 0 {
            // NUL never allowed in text.
            odd += len;
            break;
        }
        let printable = (0x20..=0x7e).contains(&c);
        // VT occurs so rarely in text that it's considered odd.
        let space = matches!(c, b'\t' | b'\n' | 0x0c | b'\r');
        // But there are a fair number of backspaces and escapes in some text.
        if printable || space || c == 0x08 || c == 0x1b {
            continue;
        }
        odd += 1;
    }
    // Allow 1/3 odd.
    odd * 3 <= len
}

/// Perl's `is_utf8_fixed_width_buf_flags(s, len, 0)`: is the buffer valid
/// (Perl extended, so surrogates and code points above Unicode are fine)
/// UTF-8, allowing the buffer to end partway through a character?
fn is_utf8_fixed_width(buf: &[u8]) -> bool {
    let mut i = 0;
    while i < buf.len() {
        let c = buf[i];
        if c < 0x80 {
            i += 1;
            continue;
        }
        // Continuation count, and the lowest valid second byte (to reject
        // overlong forms).
        let (extra, min2) = match c {
            0xc2..=0xdf => (1, 0x80),
            0xe0 => (2, 0xa0),
            0xe1..=0xef => (2, 0x80),
            0xf0 => (3, 0x90),
            0xf1..=0xf7 => (3, 0x80),
            0xf8 => (4, 0x88),
            0xf9..=0xfb => (4, 0x80),
            0xfc => (5, 0x84),
            0xfd => (5, 0x80),
            0xfe => (6, 0x82),
            // Perl's 13-byte extended form, for code points from 2**36 up to
            // the largest 64-bit value: the first continuation byte must be
            // 0x80 and the second at most 0x8f (or the value overflows), and
            // not all of the first six can be 0x80 (or it's overlong).
            0xff => {
                let conts = &buf[i + 1..buf.len().min(i + 13)];
                if conts.iter().any(|b| !(0x80..=0xbf).contains(b))
                    || conts.first().is_some_and(|&b| b != 0x80)
                    || conts.get(1).is_some_and(|&b| b > 0x8f)
                    || (conts.len() >= 6 && conts[..6].iter().all(|&b| b == 0x80))
                {
                    return false;
                }
                if conts.len() < 12 {
                    return true;
                }
                i += 13;
                continue;
            }
            _ => return false,
        };
        for k in 1..=extra {
            let Some(&b) = buf.get(i + k) else {
                // A partial character at the end of the buffer is fine.
                return true;
            };
            let min = if k == 1 { min2 } else { 0x80 };
            if !(min..=0xbf).contains(&b) {
                return false;
            }
        }
        i += extra + 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_binary() {
        assert!(looks_like_text(b"hello, world\n"));
        assert!(!looks_like_text(b"hello\0world"));
        assert!(looks_like_text("caf\u{e9} cr\u{e8}me\n".as_bytes()));
        // Latin-1, not UTF-8: 2 odd bytes out of 12 is under a third.
        assert!(looks_like_text(b"caf\xe9 cr\xe8me\n"));
        assert!(!looks_like_text(b"\xe9\xe8\xe7\xe0ab"));
        // Valid UTF-8 wins even with NULs.
        assert!(looks_like_text("\u{e9}\0\0\0".as_bytes()));
        // Cut off in the middle of a character.
        assert!(looks_like_text(&"abc\u{20ac}".as_bytes()[..5]));
        assert!(!looks_like_text(b"\x0b\x0b\x0bab"));
        // A lone 0xFF is the start of Perl's 13-byte extended UTF-8.
        assert!(looks_like_text(b"\xff"));
        assert!(!looks_like_text(b"\xff\x93"));
    }
}
