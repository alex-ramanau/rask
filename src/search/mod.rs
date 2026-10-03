//! The search: a port of ack's file loops (`file_loop_fg`, `file_loop_c`,
//! `file_loop_lL`, `file_loop_normal`), `App::Ack::File::may_be_present`,
//! and the per-file printers (`pmif_*`, `print_line_with_options`,
//! `print_line_with_context`).

use std::io::{BufRead, BufReader, Read};

use crate::bytes::{Bytes, errno_text, lossy, to_path};
use crate::matcher::{Matcher, PcreMatcher};
use crate::output::{self, color::Color};

/// Replaces control characters in a filename with `?` (`_safe_filename`).
pub fn safe_filename(name: &[u8]) -> Bytes {
    name.iter()
        .map(|&c| {
            if (0x20..=0x7e).contains(&c) || c == b'\t' {
                c
            } else {
                b'?'
            }
        })
        .collect()
}

/// Settings for a run, resolved from the options (ack's `$opt_*` globals).
pub struct Settings {
    pub re_match: Option<PcreMatcher>,
    pub re_not: Option<PcreMatcher>,
    pub re_hilite: Option<PcreMatcher>,
    pub re_scan: Option<PcreMatcher>,
    pub range_start: Option<PcreMatcher>,
    pub range_end: Option<PcreMatcher>,
    pub one: bool,
    pub a: usize,
    pub b: usize,
    pub break_: bool,
    pub c: bool,
    pub color: bool,
    pub column: bool,
    pub f: bool,
    pub g: bool,
    pub heading: bool,
    pub l: bool,
    pub big_l: bool,
    pub m: Option<i64>,
    pub output: Option<OutputTemplate>,
    pub passthru: bool,
    pub p: Option<i64>,
    pub show_filename: bool,
    /// `--show-types`, with the type definitions it needs.
    pub show_types: Option<crate::config::Types>,
    pub underline: bool,
    pub v: bool,
    pub report_bad_filenames: bool,
    /// Output record separator: `"\n"`, or `"\0"` with `--print0`.
    pub ors: &'static [u8],
    pub color_match: Color,
    pub color_filename: Color,
    pub color_lineno: Color,
    pub color_colno: Color,
}

/// `--output`: the template, and which special variables it uses.
pub struct OutputTemplate {
    template: Bytes,
    vars: Vec<u8>,
}

impl OutputTemplate {
    pub fn new(expr: &[u8]) -> OutputTemplate {
        // Expand out \t, \n and \r.
        let mut template = Vec::with_capacity(expr.len());
        let mut i = 0;
        while i < expr.len() {
            if expr[i] == b'\\' && i + 1 < expr.len() {
                let expanded = match expr[i + 1] {
                    b'n' => Some(b'\n'),
                    b'r' => Some(b'\r'),
                    b't' => Some(b'\t'),
                    _ => None,
                };
                if let Some(c) = expanded {
                    template.push(c);
                    i += 2;
                    continue;
                }
            }
            template.push(expr[i]);
            i += 1;
        }
        let supported = b"123456789_.`&'+f";
        let vars = supported
            .iter()
            .copied()
            .filter(|&v| template.windows(2).any(|w| w[0] == b'$' && w[1] == v))
            .collect();
        OutputTemplate { template, vars }
    }
}

/// Counters for `--debug`.
#[derive(Default)]
pub struct Stats {
    pub prescans: u64,
    pub linescans: u64,
    pub filematches: u64,
    pub linematches: u64,
}

/// Reads lines the way `<$fh>` does, with the trailing newline removed (`chomp`).
enum Lines {
    Buffer { data: Bytes, pos: usize },
    Stream(Box<dyn BufRead>),
}

impl Lines {
    fn next_line(&mut self, line: &mut Bytes) -> bool {
        line.clear();
        match self {
            Lines::Buffer { data, pos } => {
                if *pos >= data.len() {
                    return false;
                }
                let rest = &data[*pos..];
                let end = rest
                    .iter()
                    .position(|&c| c == b'\n')
                    .map_or(rest.len(), |i| i + 1);
                line.extend_from_slice(&rest[..end]);
                *pos += end;
            }
            Lines::Stream(r) => match r.read_until(b'\n', line) {
                Ok(0) | Err(_) => return false,
                Ok(_) => {}
            },
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        true
    }
}

/// An opened file: what `$file->open` gives, plus what the prescan read.
struct Opened {
    lines: Lines,
}

const PRESCAN_SIZE: usize = 10_000_000;

/// Strips trailing `\r` and `\n` characters (`s/[\r\n]+$//`).
fn strip_line_ending(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|&c| c != b'\r' && c != b'\n')
        .map_or(0, |i| i + 1);
    &line[..end]
}

/// Perl's `m//g` over a line: calls `f` with each match.
fn for_each_match(re: &PcreMatcher, line: &[u8], mut f: impl FnMut(usize, usize) -> bool) {
    let mut at = 0;
    while at <= line.len() {
        let Some(m) = re.find_at(line, at) else { break };
        if !f(m.start, m.end) {
            break;
        }
        at = if m.end == m.start { m.end + 1 } else { m.end };
    }
}

pub struct Searcher {
    pub s: Settings,
    pub stats: Stats,
    has_printed_from_any_file: bool,
    // Context tracking (the closure variables in ack's main loop).
    before_context_buf: Vec<Option<Bytes>>,
    before_context_pos: usize,
    after_context_pending: usize,
    printed_lineno: u64,
    is_first_match: bool,
    is_tracking_context: bool,
    match_colno: Option<usize>,
    /// The column the current line's colouring adds, for `--underline`.
    chars_used_by_coloring: Option<usize>,
}

impl Searcher {
    pub fn new(s: Settings) -> Searcher {
        Searcher {
            s,
            stats: Stats::default(),
            has_printed_from_any_file: false,
            before_context_buf: Vec::new(),
            before_context_pos: 0,
            after_context_pending: 0,
            printed_lineno: 0,
            is_first_match: true,
            is_tracking_context: false,
            match_colno: None,
            chars_used_by_coloring: None,
        }
    }

    fn say(&self, parts: &[&[u8]]) {
        output::print(parts);
        output::write(self.s.ors);
    }

    fn print_blank_line(&self) {
        output::write(b"\n");
    }

    fn warn_bad(&self, msg: String) {
        if self.s.report_bad_filenames {
            output::warn(&msg);
        }
    }

    fn open(&self, name: &[u8]) -> std::io::Result<Box<dyn Read>> {
        if name == b"-" {
            return Ok(Box::new(std::io::stdin()));
        }
        Ok(Box::new(std::fs::File::open(to_path(name))?))
    }

    fn is_regular(name: &[u8]) -> bool {
        if name == b"-" {
            return stdin_is_regular_file();
        }
        std::fs::metadata(to_path(name))
            .map(|m| m.is_file())
            .unwrap_or(false)
    }

    /// `$file->may_be_present( $re_scan )`, which also opens the file.
    /// Returns None if the file can't contain a match, or the opened file.
    fn open_and_prescan(
        &mut self,
        name: &[u8],
        prescan: bool,
    ) -> Result<Option<Opened>, std::io::Error> {
        let mut reader = self.open(name)?;
        if prescan
            && let Some(scan) = &self.s.re_scan
            && Self::is_regular(name)
        {
            let mut buf = Vec::new();
            match reader
                .by_ref()
                .take(PRESCAN_SIZE as u64)
                .read_to_end(&mut buf)
            {
                Err(e) => {
                    self.warn_bad(format!("{}: {}", lossy(name), errno_text(&e)));
                    return Ok(None);
                }
                Ok(n) => {
                    // If we read all 10M, or there are carriage returns, scan it all.
                    if n == PRESCAN_SIZE {
                        let rest: Box<dyn Read> = Box::new(std::io::Cursor::new(buf).chain(reader));
                        return Ok(Some(Opened {
                            lines: Lines::Stream(Box::new(BufReader::new(rest))),
                        }));
                    }
                    if !buf.contains(&b'\r') && !scan.is_match(&buf) {
                        return Ok(None);
                    }
                    return Ok(Some(Opened {
                        lines: Lines::Buffer { data: buf, pos: 0 },
                    }));
                }
            }
        }
        Ok(Some(Opened {
            lines: Lines::Stream(Box::new(BufReader::with_capacity(64 * 1024, reader))),
        }))
    }

    fn matches(&self, line: &[u8]) -> Option<usize> {
        let m = self.s.re_match.as_ref()?.find_at(line, 0)?;
        if let Some(not) = &self.s.re_not
            && not.is_match(line)
        {
            return None;
        }
        Some(m.start)
    }

    fn range_setup(&self) -> bool {
        let using_ranges = self.s.range_start.is_some() || self.s.range_end.is_some();
        !using_ranges || (self.s.range_start.is_none() && self.s.range_end.is_some())
    }

    fn using_ranges(&self) -> bool {
        self.s.range_start.is_some() || self.s.range_end.is_some()
    }

    fn range_starts(&self, in_range: bool, line: &[u8]) -> bool {
        in_range
            || (self.using_ranges()
                && self
                    .s
                    .range_start
                    .as_ref()
                    .is_some_and(|r| r.is_match(line)))
    }

    fn range_ends(&self, in_range: bool, line: &[u8]) -> bool {
        in_range
            && !(self.using_ranges() && self.s.range_end.as_ref().is_some_and(|r| r.is_match(line)))
    }

    // ---- -f and -g ----

    pub fn file_loop_fg(&mut self, files: &mut dyn Iterator<Item = Bytes>) -> u64 {
        let mut nmatches = 0;
        for name in files {
            if let Some(types) = &self.s.show_types {
                // App::Ack::show_types
                let file = crate::filter::File::new(name.clone());
                let ctx = crate::filter::FilterContext {
                    report_bad_filenames: self.s.report_bad_filenames,
                };
                let mut matched: Vec<&String> = types
                    .mappings
                    .iter()
                    .filter(|(_, filters)| filters.iter().any(|f| f.matches(&file, &ctx)))
                    .map(|(k, _)| k)
                    .collect();
                matched.sort();
                let list: Vec<&str> = matched.iter().map(|s| s.as_str()).collect();
                let arrow: &[u8] = if list.is_empty() { b" =>" } else { b" => " };
                self.say(&[&name, arrow, list.join(",").as_bytes()]);
            } else if self.s.g {
                self.print_line_with_options(None, &safe_filename(&name), 0, self.s.ors, false);
            } else {
                self.say(&[&safe_filename(&name)]);
            }
            nmatches += 1;
            if self.s.m.is_some_and(|m| nmatches >= m) {
                break;
            }
        }
        nmatches as u64
    }

    // ---- -c ----

    pub fn file_loop_c(&mut self, files: &mut dyn Iterator<Item = Bytes>) -> u64 {
        let mut nmatched_files = 0;
        let mut total_count = 0;
        for name in files {
            let count = self.count_matches_in_file(&name, false);
            if count > 0 {
                nmatched_files += 1;
            }
            if !self.s.show_filename {
                total_count += count;
                continue;
            }
            if !self.s.l || count > 0 {
                let shown = if self.s.color {
                    self.s.color_filename.colored(&name)
                } else {
                    name.clone()
                };
                self.say(&[&shown, b":", count.to_string().as_bytes()]);
            }
        }
        if !self.s.show_filename {
            self.say(&[total_count.to_string().as_bytes()]);
        }
        nmatched_files
    }

    // ---- -l and -L ----

    pub fn file_loop_l(&mut self, files: &mut dyn Iterator<Item = Bytes>) -> u64 {
        let mut nmatches = 0;
        for name in files {
            let is_match = self.count_matches_in_file(&name, true) > 0;
            if if self.s.big_l { !is_match } else { is_match } {
                self.say(&[&name]);
                nmatches += 1;
                if self.s.one || self.s.m.is_some_and(|m| nmatches >= m) {
                    break;
                }
            }
        }
        nmatches as u64
    }

    fn count_matches_in_file(&mut self, name: &[u8], bail: bool) -> u64 {
        let opened = match self.open_and_prescan(name, !self.s.v) {
            Err(e) => {
                self.warn_bad(format!("{}: {}", lossy(name), errno_text(&e)));
                return 0;
            }
            Ok(None) => return 0,
            Ok(Some(o)) => o,
        };
        let mut lines = opened.lines;
        let mut nmatches = 0;
        let mut in_range = self.range_setup();
        let mut line = Vec::new();
        while lines.next_line(&mut line) {
            if self.using_ranges() {
                in_range = self.range_starts(in_range, &line);
                if in_range && (self.matches(&line).is_some() != self.s.v) {
                    nmatches += 1;
                    if bail {
                        break;
                    }
                }
                in_range = self.range_ends(in_range, &line);
            } else if self.matches(&line).is_some() != self.s.v {
                nmatches += 1;
                if bail {
                    break;
                }
            }
        }
        nmatches
    }

    // ---- normal searching ----

    pub fn file_loop_normal(&mut self, files: &mut dyn Iterator<Item = Bytes>) -> u64 {
        let tracking_off = self.s.output.is_some();
        let (nb, na) = if tracking_off {
            (0, 0)
        } else {
            (self.s.b, self.s.a)
        };
        self.s.b = nb;
        self.s.a = na;
        self.before_context_buf = vec![None; nb];
        self.before_context_pos = 0;
        self.is_tracking_context = nb > 0 || na > 0;
        self.is_first_match = true;

        let mut nmatches = 0;
        for name in files {
            if self.is_tracking_context {
                self.printed_lineno = 0;
                self.after_context_pending = 0;
                if self.s.heading {
                    self.is_first_match = true;
                }
            }
            let prescan = !self.s.passthru && !self.s.v;
            if prescan {
                self.stats.prescans += 1;
            }
            let opened = match self.open_and_prescan(&name, prescan) {
                Ok(None) => {
                    if self.s.one && nmatches > 0 {
                        break;
                    }
                    continue;
                }
                Ok(Some(o)) => Some(o),
                // print_matches_in_file reports the error.
                Err(e) => {
                    self.warn_bad(format!(
                        "{}: {}",
                        lossy(&safe_filename(&name)),
                        errno_text(&e)
                    ));
                    None
                }
            };
            self.stats.linescans += 1;
            if let Some(o) = opened {
                nmatches += self.print_matches_in_file(&name, o);
            }
            if self.s.one && nmatches > 0 {
                break;
            }
        }
        nmatches
    }

    fn print_matches_in_file(&mut self, name: &[u8], opened: Opened) -> u64 {
        let filename = safe_filename(name);
        let display_filename = if self.s.show_filename && self.s.heading && self.s.color {
            self.s.color_filename.colored(&filename)
        } else {
            filename.clone()
        };
        // Go negative for no limit so it can never reduce to 0.
        let max_count = match self.s.m {
            Some(m) if m != 0 => m,
            _ => -1,
        };
        let mut lines = opened.lines;
        if self.is_tracking_context {
            self.pmif_context(&mut lines, &filename, &display_filename, max_count)
        } else if self.s.passthru {
            self.pmif_passthru(&mut lines, &filename, &display_filename, max_count)
        } else if self.s.v {
            self.pmif_opt_v(&mut lines, &filename, &display_filename, max_count)
        } else {
            self.pmif_normal(&mut lines, &filename, &display_filename, max_count)
        }
    }

    fn start_file_output(&mut self, has_printed_from_this_file: bool, display_filename: &[u8]) {
        if !has_printed_from_this_file {
            self.stats.filematches += 1;
            if self.s.break_ && self.has_printed_from_any_file {
                self.print_blank_line();
            }
            if self.s.show_filename && self.s.heading {
                self.say(&[display_filename]);
            }
        }
    }

    fn pmif_context(
        &mut self,
        lines: &mut Lines,
        filename: &[u8],
        display: &[u8],
        mut max_count: i64,
    ) -> u64 {
        let mut in_range = self.range_setup();
        let mut has_printed_from_this_file = false;
        let mut nmatches = 0;
        self.after_context_pending = 0;
        let mut line = Vec::new();
        let mut lineno = 0u64;
        while lines.next_line(&mut line) {
            lineno += 1;
            self.match_colno = None;
            in_range = self.range_starts(in_range, &line);
            let mut does_match = false;
            if in_range {
                let m = self.matches(&line);
                does_match = m.is_some() != self.s.v;
                if !self.s.v
                    && let Some(start) = m
                {
                    self.match_colno = Some(start + 1);
                }
            }
            if does_match && max_count != 0 {
                self.start_file_output(has_printed_from_this_file, display);
                self.print_line_with_context(filename, &line, lineno);
                has_printed_from_this_file = true;
                self.stats.linematches += 1;
                nmatches += 1;
                max_count -= 1;
            } else if self.after_context_pending > 0 {
                // No column for context lines, since they have no matches.
                let column = std::mem::replace(&mut self.s.column, false);
                self.print_line_with_options(Some(filename), &line, lineno, b"-", false);
                self.s.column = column;
                self.after_context_pending -= 1;
            } else if self.s.b > 0 {
                // Save the line for "before" context.
                self.before_context_buf[self.before_context_pos] = Some(line.clone());
                self.before_context_pos = (self.before_context_pos + 1) % self.s.b;
            }
            in_range = self.range_ends(in_range, &line);
            if max_count == 0 && self.after_context_pending == 0 {
                break;
            }
        }
        nmatches
    }

    fn pmif_passthru(
        &mut self,
        lines: &mut Lines,
        filename: &[u8],
        display: &[u8],
        mut max_count: i64,
    ) -> u64 {
        let mut in_range = self.range_setup();
        let mut has_printed_from_this_file = false;
        let mut nmatches = 0;
        let mut line = Vec::new();
        let mut lineno = 0u64;
        while lines.next_line(&mut line) {
            lineno += 1;
            in_range = self.range_starts(in_range, &line);
            self.match_colno = None;
            let m = self.matches(&line);
            if in_range && let Some(start) = m {
                self.match_colno = Some(start + 1);
                if !has_printed_from_this_file {
                    if self.s.break_ && self.has_printed_from_any_file {
                        self.print_blank_line();
                    }
                    if self.s.show_filename && self.s.heading {
                        self.say(&[display]);
                    }
                }
                self.print_line_with_options(Some(filename), &line, lineno, b":", false);
                has_printed_from_this_file = true;
                nmatches += 1;
                max_count -= 1;
            } else {
                if self.s.break_ && !has_printed_from_this_file && self.has_printed_from_any_file {
                    self.print_blank_line();
                }
                self.print_line_with_options(Some(filename), &line, lineno, b"-", true);
                has_printed_from_this_file = true;
            }
            in_range = self.range_ends(in_range, &line);
            if max_count == 0 {
                break;
            }
        }
        nmatches
    }

    fn pmif_opt_v(
        &mut self,
        lines: &mut Lines,
        filename: &[u8],
        display: &[u8],
        mut max_count: i64,
    ) -> u64 {
        let mut in_range = self.range_setup();
        let mut has_printed_from_this_file = false;
        let mut nmatches = 0;
        self.match_colno = None;
        let mut line = Vec::new();
        let mut lineno = 0u64;
        while lines.next_line(&mut line) {
            lineno += 1;
            in_range = self.range_starts(in_range, &line);
            if in_range && self.matches(&line).is_none() {
                if !has_printed_from_this_file {
                    if self.s.break_ && self.has_printed_from_any_file {
                        self.print_blank_line();
                    }
                    if self.s.show_filename && self.s.heading {
                        self.say(&[display]);
                    }
                }
                self.print_line_with_context(filename, &line, lineno);
                has_printed_from_this_file = true;
                nmatches += 1;
                max_count -= 1;
            }
            in_range = self.range_ends(in_range, &line);
            if max_count == 0 {
                break;
            }
        }
        nmatches
    }

    fn pmif_normal(
        &mut self,
        lines: &mut Lines,
        filename: &[u8],
        display: &[u8],
        mut max_count: i64,
    ) -> u64 {
        let mut in_range = self.range_setup();
        let mut has_printed_from_this_file = false;
        let mut nmatches = 0;
        let mut last_match_lineno: Option<u64> = None;
        let mut line = Vec::new();
        let mut lineno = 0u64;
        while lines.next_line(&mut line) {
            lineno += 1;
            in_range = self.range_starts(in_range, &line);
            if in_range {
                self.match_colno = None;
                if let Some(start) = self.matches(&line) {
                    self.match_colno = Some(start + 1);
                    self.start_file_output(has_printed_from_this_file, display);
                    if let Some(p) = self.s.p.filter(|p| *p != 0) {
                        match last_match_lineno {
                            Some(last) => {
                                if lineno as i64 > last as i64 + p {
                                    self.print_blank_line();
                                }
                            }
                            None => {
                                if !self.s.break_ && self.has_printed_from_any_file {
                                    self.print_blank_line();
                                }
                            }
                        }
                    }
                    let stripped = strip_line_ending(&line).to_vec();
                    self.print_line_with_options(Some(filename), &stripped, lineno, b":", false);
                    has_printed_from_this_file = true;
                    nmatches += 1;
                    self.stats.linematches += 1;
                    max_count -= 1;
                    last_match_lineno = Some(lineno);
                }
            }
            in_range = self.range_ends(in_range, &line);
            if max_count == 0 {
                break;
            }
        }
        nmatches
    }

    fn print_line_with_options(
        &mut self,
        filename: Option<&[u8]>,
        line: &[u8],
        lineno: u64,
        separator: &[u8],
        skip_coloring: bool,
    ) {
        self.has_printed_from_any_file = true;
        self.printed_lineno = lineno;

        let mut parts: Vec<Bytes> = Vec::new();
        if self.s.show_filename
            && let Some(filename) = filename
        {
            let lineno = lineno.to_string().into_bytes();
            let (disp_filename, disp_lineno) = if self.s.color {
                (
                    self.s.color_filename.colored(filename),
                    self.s.color_lineno.colored(&lineno),
                )
            } else {
                (filename.to_vec(), lineno)
            };
            if !self.s.heading {
                parts.push(disp_filename);
            }
            parts.push(disp_lineno);
            if self.s.column {
                let colno = self
                    .match_colno
                    .map(|c| c.to_string().into_bytes())
                    .unwrap_or_default();
                parts.push(if self.s.color {
                    self.s.color_colno.colored(&colno)
                } else {
                    colno
                });
            }
        }

        if let Some(tmpl) = &self.s.output {
            let re = self.s.re_match.as_ref().expect("--output needs a regex");
            let mut outputs = Vec::new();
            for_each_match(re, line, |start, end| {
                outputs.push(expand_output(tmpl, re, line, start, end, lineno, filename));
                true
            });
            for out in outputs {
                let mut all = parts.clone();
                all.push(out);
                self.say(&[&all.join(separator)]);
            }
            return;
        }

        let mut underline: Bytes = Vec::new();
        let hilite = self.s.re_hilite.as_ref();
        // Underlining comes first, because highlighting changes the line length.
        if self.s.underline
            && !skip_coloring
            && let Some(re) = hilite
        {
            for_each_match(re, line, |start, end| {
                if end <= start {
                    return false;
                }
                underline.resize(start.max(underline.len()), b' ');
                underline.extend(std::iter::repeat_n(b'^', end - start));
                true
            });
        }
        let mut shown = line.to_vec();
        if self.s.color
            && !skip_coloring
            && let Some(re) = hilite
        {
            // As in Perl, each match is replaced in the line itself, and the
            // search goes on in the modified line.
            let mut pos = 0;
            let mut highlighted = false;
            while pos <= shown.len() {
                let Some(m) = re.find_at(&shown, pos) else {
                    break;
                };
                if m.end <= m.start {
                    break;
                }
                let substitution = self.s.color_match.colored(&shown[m.start..m.end]);
                let grown = substitution.len() - (m.end - m.start);
                shown.splice(m.start..m.end, substitution);
                pos = m.end + grown;
                highlighted = true;
            }
            if highlighted {
                // Reset formatting and delete everything to the end of the line.
                shown.extend_from_slice(b"\x1b[0m\x1b[K");
            }
        }
        parts.push(shown);
        self.say(&[&parts.join(separator)]);

        if !underline.is_empty() {
            let chars_used = *self.chars_used_by_coloring.get_or_insert_with(|| {
                let mut n = 0;
                if self.s.color {
                    if !self.s.heading {
                        n += self.s.color_filename.overhead();
                    }
                    n += self.s.color_lineno.overhead();
                    if self.s.column {
                        n += self.s.color_colno.overhead();
                    }
                }
                n
            });
            parts.pop();
            if !parts.is_empty() {
                let left = parts.join(separator);
                let spaces = (left.len() + 1).saturating_sub(chars_used);
                output::write(&vec![b' '; spaces]);
            }
            self.say(&[&underline]);
        }
    }

    fn print_line_with_context(&mut self, filename: &[u8], matching_line: &[u8], lineno: u64) {
        let matching_line = strip_line_ending(matching_line).to_vec();
        let nb = self.s.b;
        if self.s.a > 0 || nb > 0 {
            let mut before_unprinted = lineno as i64 - self.printed_lineno as i64 - 1;
            if !self.is_first_match && (self.printed_lineno == 0 || before_unprinted > nb as i64) {
                self.say(&[b"--"]);
            }
            // We want at most $n_before_ctx_lines of context.
            if before_unprinted > nb as i64 {
                before_unprinted = nb as i64;
            }
            while before_unprinted > 0 {
                let idx =
                    (self.before_context_pos as i64 - before_unprinted + nb as i64) as usize % nb;
                let line = self.before_context_buf[idx].clone().unwrap_or_default();
                let column = std::mem::replace(&mut self.s.column, false);
                self.print_line_with_options(
                    Some(filename),
                    &line,
                    lineno - before_unprinted as u64,
                    b"-",
                    false,
                );
                self.s.column = column;
                before_unprinted -= 1;
            }
        }
        self.print_line_with_options(Some(filename), &matching_line, lineno, b":", false);
        // We want to get the next $n_after_ctx_lines printed.
        self.after_context_pending = self.s.a;
        self.is_first_match = false;
    }
}

/// One `--output` expansion: `s/\$([$vars])/$keep{$1}/eg` over the template.
fn expand_output(
    tmpl: &OutputTemplate,
    re: &PcreMatcher,
    line: &[u8],
    start: usize,
    end: usize,
    lineno: u64,
    filename: Option<&[u8]>,
) -> Bytes {
    if tmpl.vars.is_empty() {
        return tmpl.template.clone();
    }
    let groups = re.captures_at(line, start);
    let value = |v: u8| -> Bytes {
        match v {
            b'&' => line[start..end].to_vec(),
            b'`' => line[..start].to_vec(),
            b'\'' => line[end..].to_vec(),
            b'_' => line.to_vec(),
            b'.' => lineno.to_string().into_bytes(),
            b'f' => filename.map(<[u8]>::to_vec).unwrap_or_default(),
            b'+' => groups
                .iter()
                .skip(1)
                .rev()
                .find_map(|g| g.map(|(s, e)| line[s..e].to_vec()))
                .unwrap_or_default(),
            d @ b'1'..=b'9' => groups
                .get(usize::from(d - b'0'))
                .copied()
                .flatten()
                .map(|(s, e)| line[s..e].to_vec())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    let t = &tmpl.template;
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if t[i] == b'$' && i + 1 < t.len() && tmpl.vars.contains(&t[i + 1]) {
            out.extend(value(t[i + 1]));
            i += 2;
        } else {
            out.push(t[i]);
            i += 1;
        }
    }
    out
}

#[cfg(unix)]
fn stdin_is_regular_file() -> bool {
    std::fs::metadata("/dev/stdin")
        .map(|m| m.is_file())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn stdin_is_regular_file() -> bool {
    false
}
