//! `_check_for_mutex_options`: options that can't be used together.

use std::collections::HashMap;

use crate::bytes::Bytes;
use crate::cli::getopt::{self, Config, Spec};

/// `mutex_options()` from App::Ack::ConfigLoader (machine-generated there by
/// dev/crank-mutex), as (option, option) pairs.
const MUTEX: &[(&str, &str); 261] = &[
    ("1", "m"),
    ("1", "passthru"),
    ("A", "L"),
    ("A", "c"),
    ("A", "f"),
    ("A", "g"),
    ("A", "l"),
    ("A", "o"),
    ("A", "output"),
    ("A", "p"),
    ("A", "passthru"),
    ("B", "L"),
    ("B", "c"),
    ("B", "f"),
    ("B", "g"),
    ("B", "l"),
    ("B", "o"),
    ("B", "output"),
    ("B", "p"),
    ("B", "passthru"),
    ("C", "L"),
    ("C", "c"),
    ("C", "f"),
    ("C", "g"),
    ("C", "l"),
    ("C", "o"),
    ("C", "output"),
    ("C", "p"),
    ("C", "passthru"),
    ("H", "L"),
    ("H", "f"),
    ("H", "g"),
    ("H", "l"),
    ("I", "f"),
    ("L", "A"),
    ("L", "B"),
    ("L", "C"),
    ("L", "H"),
    ("L", "L"),
    ("L", "break"),
    ("L", "c"),
    ("L", "column"),
    ("L", "f"),
    ("L", "g"),
    ("L", "group"),
    ("L", "h"),
    ("L", "heading"),
    ("L", "l"),
    ("L", "no-filename"),
    ("L", "o"),
    ("L", "output"),
    ("L", "p"),
    ("L", "passthru"),
    ("L", "show-types"),
    ("L", "v"),
    ("L", "with-filename"),
    ("and", "or"),
    ("break", "L"),
    ("break", "c"),
    ("break", "f"),
    ("break", "g"),
    ("break", "l"),
    ("c", "A"),
    ("c", "B"),
    ("c", "C"),
    ("c", "L"),
    ("c", "break"),
    ("c", "column"),
    ("c", "f"),
    ("c", "g"),
    ("c", "group"),
    ("c", "heading"),
    ("c", "m"),
    ("c", "o"),
    ("c", "output"),
    ("c", "p"),
    ("c", "passthru"),
    ("column", "L"),
    ("column", "c"),
    ("column", "f"),
    ("column", "g"),
    ("column", "l"),
    ("column", "o"),
    ("column", "output"),
    ("column", "passthru"),
    ("column", "v"),
    ("f", "A"),
    ("f", "B"),
    ("f", "C"),
    ("f", "H"),
    ("f", "I"),
    ("f", "L"),
    ("f", "break"),
    ("f", "c"),
    ("f", "column"),
    ("f", "f"),
    ("f", "files-from"),
    ("f", "g"),
    ("f", "group"),
    ("f", "h"),
    ("f", "heading"),
    ("f", "i"),
    ("f", "l"),
    ("f", "m"),
    ("f", "match"),
    ("f", "o"),
    ("f", "output"),
    ("f", "p"),
    ("f", "passthru"),
    ("f", "smart-case"),
    ("f", "u"),
    ("f", "v"),
    ("f", "x"),
    ("files-from", "f"),
    ("files-from", "g"),
    ("files-from", "x"),
    ("g", "A"),
    ("g", "B"),
    ("g", "C"),
    ("g", "H"),
    ("g", "L"),
    ("g", "break"),
    ("g", "c"),
    ("g", "column"),
    ("g", "f"),
    ("g", "files-from"),
    ("g", "g"),
    ("g", "group"),
    ("g", "h"),
    ("g", "heading"),
    ("g", "l"),
    ("g", "m"),
    ("g", "match"),
    ("g", "o"),
    ("g", "output"),
    ("g", "p"),
    ("g", "passthru"),
    ("g", "u"),
    ("g", "x"),
    ("group", "L"),
    ("group", "c"),
    ("group", "f"),
    ("group", "g"),
    ("group", "l"),
    ("h", "L"),
    ("h", "f"),
    ("h", "g"),
    ("h", "l"),
    ("heading", "L"),
    ("heading", "c"),
    ("heading", "f"),
    ("heading", "g"),
    ("heading", "l"),
    ("i", "f"),
    ("l", "A"),
    ("l", "B"),
    ("l", "C"),
    ("l", "H"),
    ("l", "L"),
    ("l", "break"),
    ("l", "column"),
    ("l", "f"),
    ("l", "g"),
    ("l", "group"),
    ("l", "h"),
    ("l", "heading"),
    ("l", "l"),
    ("l", "no-filename"),
    ("l", "o"),
    ("l", "output"),
    ("l", "p"),
    ("l", "passthru"),
    ("l", "show-types"),
    ("l", "with-filename"),
    ("m", "1"),
    ("m", "c"),
    ("m", "f"),
    ("m", "g"),
    ("m", "passthru"),
    ("match", "f"),
    ("match", "g"),
    ("no-filename", "L"),
    ("no-filename", "l"),
    ("o", "A"),
    ("o", "B"),
    ("o", "C"),
    ("o", "L"),
    ("o", "c"),
    ("o", "column"),
    ("o", "f"),
    ("o", "g"),
    ("o", "l"),
    ("o", "o"),
    ("o", "output"),
    ("o", "p"),
    ("o", "passthru"),
    ("o", "show-types"),
    ("o", "v"),
    ("or", "and"),
    ("output", "A"),
    ("output", "B"),
    ("output", "C"),
    ("output", "L"),
    ("output", "c"),
    ("output", "column"),
    ("output", "f"),
    ("output", "g"),
    ("output", "l"),
    ("output", "o"),
    ("output", "output"),
    ("output", "p"),
    ("output", "passthru"),
    ("output", "show-types"),
    ("output", "u"),
    ("output", "v"),
    ("p", "A"),
    ("p", "B"),
    ("p", "C"),
    ("p", "L"),
    ("p", "c"),
    ("p", "f"),
    ("p", "g"),
    ("p", "l"),
    ("p", "o"),
    ("p", "output"),
    ("p", "p"),
    ("p", "passthru"),
    ("passthru", "1"),
    ("passthru", "A"),
    ("passthru", "B"),
    ("passthru", "C"),
    ("passthru", "L"),
    ("passthru", "c"),
    ("passthru", "column"),
    ("passthru", "f"),
    ("passthru", "g"),
    ("passthru", "l"),
    ("passthru", "m"),
    ("passthru", "o"),
    ("passthru", "output"),
    ("passthru", "p"),
    ("passthru", "v"),
    ("show-types", "L"),
    ("show-types", "l"),
    ("show-types", "o"),
    ("show-types", "output"),
    ("smart-case", "f"),
    ("u", "f"),
    ("u", "g"),
    ("u", "output"),
    ("v", "L"),
    ("v", "column"),
    ("v", "f"),
    ("v", "o"),
    ("v", "output"),
    ("v", "passthru"),
    ("with-filename", "L"),
    ("with-filename", "l"),
    ("x", "f"),
    ("x", "files-from"),
    ("x", "g"),
];

fn is_mutex(a: &str, b: &str) -> bool {
    MUTEX.binary_search(&(a, b)).is_ok()
}

/// Splits a Getopt::Long spec into its names and its type suffix.
fn split_spec(spec: &str) -> (Vec<&str>, String) {
    let negatable = spec.ends_with('!');
    let spec = spec.trim_end_matches('!');
    let (names, suffix) = match spec.find(['=', ':']) {
        Some(i) => (&spec[..i], &spec[i..]),
        None => (spec, ""),
    };
    let mut suffix = suffix.to_string();
    if negatable {
        suffix.push('!');
    }
    (names.split('|').collect(), suffix)
}

/// Dies if the command line uses two options that can't go together.
///
/// Like Perl ack, this parses the command line twice: once recording the
/// option names as the user wrote them (`@raw`), once recording each
/// option's canonical name and position (`%parsed`).
pub fn check(argv: &[Bytes], specs: &[String]) {
    let mut all: Vec<String> = specs.to_vec();
    all.extend(
        [
            "type-add=s",
            "type-del=s",
            "type-set=s",
            "ignore-ack-defaults",
        ]
        .map(String::from),
    );

    // Capture the @raw array: one spec per alias, so the callback sees the alias.
    let mut raw: HashMap<usize, String> = HashMap::new();
    {
        let mut raw_specs: Vec<Spec<String>> = Vec::new();
        for spec in &all {
            let (names, suffix) = split_spec(spec);
            for alias in names {
                raw_specs.push(Spec::new(format!("{alias}{suffix}"), alias.to_string()));
            }
        }
        let pos = std::cell::Cell::new(0usize);
        let mut bump = |_: &[u8]| pos.set(pos.get() + 1);
        getopt::get_options_with_nonoptions(
            &mut argv.to_vec(),
            &raw_specs,
            Config::ack(false, true),
            |alias, _, _| {
                raw.insert(
                    pos.get(),
                    if alias.len() == 1 {
                        format!("-{alias}")
                    } else {
                        format!("--{alias}")
                    },
                );
                pos.set(pos.get() + 1);
                Ok(())
            },
            Some(&mut bump),
        );
    }

    // Capture the %parsed hash: canonical name => position.
    let mut parsed: HashMap<String, usize> = HashMap::new();
    {
        let parsed_specs: Vec<Spec<()>> = all.iter().map(|s| Spec::new(s.clone(), ())).collect();
        let pos_cell = std::cell::Cell::new(0usize);
        let mut bump = |_: &[u8]| pos_cell.set(pos_cell.get() + 1);
        getopt::get_options_with_nonoptions(
            &mut argv.to_vec(),
            &parsed_specs,
            Config::ack(false, true),
            |_, cname, _| {
                parsed.insert(cname.to_string(), pos_cell.get());
                pos_cell.set(pos_cell.get() + 1);
                Ok(())
            },
            Some(&mut bump),
        );
    }

    let mut used: Vec<&String> = parsed.keys().collect();
    used.sort_by_key(|k| k.to_lowercase());
    for i in &used {
        for j in &used {
            if i == j || !is_mutex(i, j) {
                continue;
            }
            let x = raw.get(&parsed[*i]).cloned().unwrap_or_default();
            let y = raw.get(&parsed[*j]).cloned().unwrap_or_default();
            crate::output::die(&format!("Options '{x}' and '{y}' can't be used together."));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted() {
        assert!(MUTEX.windows(2).all(|w| w[0] < w[1]));
        assert!(is_mutex("f", "o") && is_mutex("o", "f"));
        assert!(!is_mutex("i", "w"));
    }
}
