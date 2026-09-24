//! Regex engine spike (plan item 0.3).
//!
//!     regex-spike patterns.jsonl haystack.bin perl-results.jsonl > report.md
//!
//! Runs every pattern from the corpus through each candidate engine on the
//! same haystack lines as `perl/perl-matches.pl`. It then reports, for each
//! engine, how often the results are identical to Perl's.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Deserialize)]
struct Pattern {
    id: usize,
    pattern: String,
    #[serde(default)]
    mods: String,
    sites: Vec<String>,
}

#[derive(Deserialize)]
struct PerlResult {
    id: usize,
    error: Option<String>,
    #[serde(default)]
    first: Vec<[usize; 3]>,
    #[serde(default)]
    all: Vec<[usize; 3]>,
}

type Found = Option<(usize, usize)>;

trait Engine {
    /// First match in `line` starting at or after `start`. `Err` means the
    /// engine can't search this line at all (e.g. the line isn't UTF-8).
    fn find_at(&self, line: &[u8], start: usize) -> Result<Found, ()>;
    /// Where to restart after an empty match at `at`.
    fn step(&self, line: &[u8], at: usize) -> usize {
        let _ = line;
        at + 1
    }
}

struct Pcre(pcre2::bytes::Regex);
impl Engine for Pcre {
    fn find_at(&self, line: &[u8], start: usize) -> Result<Found, ()> {
        self.0
            .find_at(line, start)
            .map(|m| m.map(|m| (m.start(), m.end())))
            .map_err(|_| ())
    }
}

struct RustRegex(regex::bytes::Regex);
impl Engine for RustRegex {
    fn find_at(&self, line: &[u8], start: usize) -> Result<Found, ()> {
        Ok(self.0.find_at(line, start).map(|m| (m.start(), m.end())))
    }
}

struct Fancy(fancy_regex::Regex);
impl Engine for Fancy {
    fn find_at(&self, line: &[u8], start: usize) -> Result<Found, ()> {
        let s = std::str::from_utf8(line).map_err(|_| ())?;
        match self.0.find_from_pos(s, start) {
            Ok(m) => Ok(m.map(|m| (m.start(), m.end()))),
            Err(_) => Err(()),
        }
    }
    fn step(&self, line: &[u8], at: usize) -> usize {
        let mut next = at + 1;
        while next < line.len() && (line[next] & 0xC0) == 0x80 {
            next += 1;
        }
        next
    }
}

const ENGINES: [&str; 3] = ["pcre2", "fancy-regex", "regex::bytes"];

/// Perl's `(?^flags:...)` means "reset to defaults, then apply flags".
/// Neither Rust engine knows `^`, so spell out the flags to turn off.
fn expand_caret(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len());
    let mut rest = pat;
    while let Some(i) = rest.find("(?^") {
        out.push_str(&rest[..i]);
        rest = &rest[i + 3..];
        let flags: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        rest = &rest[flags.len()..];
        let off: String = "imsx".chars().filter(|c| !flags.contains(*c)).collect();
        out.push_str("(?");
        out.push_str(&flags);
        if !off.is_empty() {
            out.push('-');
            out.push_str(&off);
        }
        if !rest.starts_with(':') && !rest.starts_with(')') {
            out.push(':');
        }
    }
    out.push_str(rest);
    out
}

fn build(engine: &str, pat: &Pattern) -> Result<Box<dyn Engine>, String> {
    // Only the modifiers that mean something to a matcher; charset modifiers
    // (a, d, l, u) come from Perl's internals.
    let mods: String = pat.mods.chars().filter(|c| "imsxn".contains(*c)).collect();
    let full = if mods.is_empty() {
        pat.pattern.clone()
    } else {
        format!("(?{mods}){}", pat.pattern)
    };
    match engine {
        "pcre2" => pcre2::bytes::RegexBuilder::new()
            .jit_if_available(true)
            .build(&full)
            .map(|r| Box::new(Pcre(r)) as Box<dyn Engine>)
            .map_err(|e| e.to_string()),
        "fancy-regex" => fancy_regex::Regex::new(&expand_caret(&full))
            .map(|r| Box::new(Fancy(r)) as Box<dyn Engine>)
            .map_err(|e| e.to_string()),
        "regex::bytes" => regex::bytes::RegexBuilder::new(&expand_caret(&full))
            .unicode(false)
            .build()
            .map(|r| Box::new(RustRegex(r)) as Box<dyn Engine>)
            .map_err(|e| e.to_string()),
        _ => unreachable!(),
    }
}

struct Run {
    first: Vec<[usize; 3]>,
    all: Vec<[usize; 3]>,
    unsearchable_lines: usize,
    first_time: Duration,
}

fn run(engine: &dyn Engine, lines: &[&[u8]]) -> Run {
    let mut first = Vec::new();
    let t = Instant::now();
    let mut unsearchable_lines = 0;
    for (n, line) in lines.iter().enumerate() {
        match engine.find_at(line, 0) {
            Ok(Some((s, e))) => first.push([n, s, e]),
            Ok(None) => {}
            Err(()) => unsearchable_lines += 1,
        }
    }
    let first_time = t.elapsed();

    let mut all = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let mut start = 0;
        while start <= line.len() {
            let Ok(Some((s, e))) = engine.find_at(line, start) else {
                break;
            };
            all.push([n, s, e]);
            start = if e == s { engine.step(line, e) } else { e };
        }
    }
    Run {
        first,
        all,
        unsearchable_lines,
        first_time,
    }
}

#[derive(Default)]
struct Tally {
    compiled: usize,
    compile_agrees: usize,
    first_same: usize,
    all_same: usize,
    compared: usize,
    time: Duration,
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &str) -> Vec<T> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{path}: {e}: {l}")))
        .collect()
}

fn show(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim_end_matches('\n');
    let s: String = s.chars().take(60).collect();
    format!("{s:?}")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [patterns_file, haystack_file, perl_file] = args.as_slice() else {
        eprintln!("usage: regex-spike patterns.jsonl haystack.bin perl-results.jsonl");
        std::process::exit(2);
    };

    let patterns: Vec<Pattern> = read_jsonl(patterns_file);
    let perl: BTreeMap<usize, PerlResult> = read_jsonl::<PerlResult>(perl_file)
        .into_iter()
        .map(|r| (r.id, r))
        .collect();
    let haystack = std::fs::read(haystack_file).expect("haystack");
    let lines: Vec<&[u8]> = haystack.split_inclusive(|&b| b == b'\n').collect();
    let non_utf8 = lines
        .iter()
        .filter(|l| std::str::from_utf8(l).is_err())
        .count();

    let mut tallies: BTreeMap<&str, Tally> =
        ENGINES.iter().map(|e| (*e, Tally::default())).collect();
    let mut divergences: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut sites: BTreeMap<&str, usize> = BTreeMap::new();

    for pat in &patterns {
        for s in &pat.sites {
            *sites.entry(s.as_str()).or_default() += 1;
        }
        let oracle = &perl[&pat.id];
        let shown = format!(
            "`{}`{}",
            pat.pattern.replace('`', "\\`"),
            if pat.mods.is_empty() {
                String::new()
            } else {
                format!(" /{}", pat.mods)
            }
        );

        for engine in ENGINES {
            let tally = tallies.get_mut(engine).unwrap();
            let notes = divergences.entry(engine).or_default();
            match (build(engine, pat), &oracle.error) {
                (Err(_), Some(_)) => tally.compile_agrees += 1,
                (Err(e), None) => {
                    let e = e.lines().last().unwrap_or("").trim().to_string();
                    notes.push(format!("{shown}: rejected, Perl accepts it ({e})"));
                }
                (Ok(_), Some(_)) => {
                    tally.compiled += 1;
                    notes.push(format!("{shown}: accepted, Perl rejects it"));
                }
                (Ok(re), None) => {
                    tally.compiled += 1;
                    tally.compile_agrees += 1;
                    tally.compared += 1;
                    let got = run(re.as_ref(), &lines);
                    tally.time += got.first_time;
                    if got.first == oracle.first {
                        tally.first_same += 1;
                    }
                    if got.all == oracle.all {
                        tally.all_same += 1;
                    } else {
                        let (want, have) = if got.first != oracle.first {
                            (&oracle.first, &got.first)
                        } else {
                            (&oracle.all, &got.all)
                        };
                        let kind = if got.first != oracle.first {
                            "first match"
                        } else {
                            "m//g iteration"
                        };
                        let diff = want
                            .iter()
                            .zip(have.iter())
                            .find(|(a, b)| a != b)
                            .map(|(a, _)| *a)
                            .or_else(|| want.get(have.len()).copied())
                            .or_else(|| have.get(want.len()).copied());
                        let example = diff
                            .map(|[n, _, _]| format!(", e.g. line {}", show(lines[n])))
                            .unwrap_or_default();
                        let skipped = if got.unsearchable_lines > 0 {
                            format!("; {} non-UTF-8 lines unsearchable", got.unsearchable_lines)
                        } else {
                            String::new()
                        };
                        notes.push(format!(
                            "{shown}: {kind} differs (Perl {} vs {}){example}{skipped}",
                            want.len(),
                            have.len()
                        ));
                    }
                }
            }
        }
    }

    let perl_rejects = perl.values().filter(|r| r.error.is_some()).count();
    let mut out = String::new();
    let _ = writeln!(out, "## Corpus\n");
    let _ = writeln!(
        out,
        "- {} distinct patterns ({} rejected by Perl)",
        patterns.len(),
        perl_rejects
    );
    for (site, n) in &sites {
        let _ = writeln!(out, "  - `{site}`: {n}");
    }
    let _ = writeln!(
        out,
        "- {} haystack lines, {} of them not valid UTF-8\n",
        lines.len(),
        non_utf8
    );
    let _ = writeln!(out, "## Results\n");
    let _ = writeln!(
        out,
        "| Engine | Compiles | Compile verdict = Perl | First match = Perl | All matches = Perl | Scan time |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|");
    for engine in ENGINES {
        let t = &tallies[engine];
        let _ = writeln!(
            out,
            "| {engine} | {} / {} | {} / {} | {} / {} | {} / {} | {:.1} ms |",
            t.compiled,
            patterns.len(),
            t.compile_agrees,
            patterns.len(),
            t.first_same,
            t.compared,
            t.all_same,
            t.compared,
            t.time.as_secs_f64() * 1000.0
        );
    }
    let _ = writeln!(
        out,
        "\nScan time is the first-match pass over every line, summed over the patterns that engine compiled, so the columns only compare like with like when the compile counts are equal."
    );
    for engine in ENGINES {
        let notes = &divergences[engine];
        let _ = writeln!(out, "\n### {engine}: {} divergences\n", notes.len());
        for n in notes.iter().take(40) {
            let _ = writeln!(out, "- {n}");
        }
        if notes.len() > 40 {
            let _ = writeln!(out, "- ... and {} more", notes.len() - 40);
        }
    }
    print!("{out}");
}
