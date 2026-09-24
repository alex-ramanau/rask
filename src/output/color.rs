//! The parts of Term::ANSIColor 5.01 that ack uses: `colored( $string, $spec )`.

use crate::bytes::{Bytes, lossy};

fn attribute(code: &str) -> Option<String> {
    let basic = |name: &str| -> Option<u32> {
        Some(match name {
            "clear" | "reset" => 0,
            "bold" => 1,
            "dark" | "faint" => 2,
            "italic" => 3,
            "underline" | "underscore" => 4,
            "blink" => 5,
            "reverse" => 7,
            "concealed" => 8,
            _ => return None,
        })
    };
    const COLORS: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    if let Some(n) = basic(code) {
        return Some(n.to_string());
    }
    let (on, name) = match code.strip_prefix("on_") {
        Some(rest) => (true, rest),
        None => (false, code),
    };
    let (bright, color) = match name.strip_prefix("bright_") {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    if let Some(i) = COLORS.iter().position(|c| *c == color) {
        let base = match (on, bright) {
            (false, false) => 30,
            (true, false) => 40,
            (false, true) => 90,
            (true, true) => 100,
        };
        return Some((base + i).to_string());
    }
    if bright {
        return None;
    }
    let prefix = if on { "48" } else { "38" };
    let number = |s: &str| -> Option<u32> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    if let Some(n) = name
        .strip_prefix("ansi")
        .and_then(number)
        .filter(|n| *n <= 255)
    {
        return Some(format!("{prefix};5;{n}"));
    }
    if let Some(n) = name
        .strip_prefix("grey")
        .and_then(number)
        .filter(|n| *n <= 23)
    {
        return Some(format!("{prefix};5;{}", n + 232));
    }
    if let Some(rgb) = name.strip_prefix("rgb")
        && rgb.len() == 3
        && rgb.bytes().all(|b| (b'0'..=b'5').contains(&b))
    {
        let d: Vec<u32> = rgb.bytes().map(|b| u32::from(b - b'0')).collect();
        return Some(format!("{prefix};5;{}", 16 + 36 * d[0] + 6 * d[1] + d[2]));
    }
    // rNNNgNNNbNNN: 24-bit colour.
    let rest = name.strip_prefix('r')?;
    let g = rest.find('g')?;
    let b = rest[g..].find('b')? + g;
    let (r, gv, bv) = (
        number(&rest[..g])?,
        number(&rest[g + 1..b])?,
        number(&rest[b + 1..])?,
    );
    if r > 255 || gv > 255 || bv > 255 {
        return None;
    }
    Some(format!("{prefix};2;{r};{gv};{bv}"))
}

/// `Term::ANSIColor::color( $spec )`: the escape sequence for a spec such as
/// `"black on_yellow"`. `Err` is Term::ANSIColor's croak message.
pub fn color(spec: &[u8]) -> Result<Bytes, String> {
    if colors_disabled() {
        return Ok(Vec::new());
    }
    let spec = lossy(spec);
    let mut codes = Vec::new();
    for code in spec.split_ascii_whitespace() {
        let code = code.to_ascii_lowercase();
        match attribute(&code) {
            Some(a) => codes.push(a),
            None => return Err(format!("Invalid attribute name {code}")),
        }
    }
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(format!("\x1b[{}m", codes.join(";")).into_bytes())
}

fn colors_disabled() -> bool {
    std::env::var_os("ANSI_COLORS_DISABLED").is_some_and(|v| !v.is_empty() && v != "0")
        || std::env::var_os("NO_COLOR").is_some()
}

/// A compiled colour: `colored( $s, $spec )` is `start . $s . "\e[0m"`.
#[derive(Clone, Debug)]
pub struct Color {
    start: Bytes,
    disabled: bool,
}

impl Color {
    pub fn new(spec: &[u8]) -> Result<Color, String> {
        Ok(Color {
            start: color(spec)?,
            disabled: colors_disabled(),
        })
    }

    pub fn colored(&self, s: &[u8]) -> Bytes {
        if self.disabled {
            return s.to_vec();
        }
        [&self.start[..], s, b"\x1b[0m"].concat()
    }

    /// How many bytes colouring adds to a string.
    pub fn overhead(&self) -> usize {
        self.colored(b"x").len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs() {
        assert_eq!(color(b"black on_yellow").unwrap(), b"\x1b[30;43m");
        assert_eq!(color(b"bold green").unwrap(), b"\x1b[1;32m");
        assert_eq!(
            color(b"BRIGHT_RED on_bright_blue").unwrap(),
            b"\x1b[91;104m"
        );
        assert_eq!(
            color(b"rgb500 on_grey10 ansi3").unwrap(),
            b"\x1b[38;5;196;48;5;242;38;5;3m"
        );
        assert_eq!(color(b"r255g0b10").unwrap(), b"\x1b[38;2;255;0;10m");
        assert!(color(b"sparkly").is_err());
    }
}
