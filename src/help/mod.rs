//! `--help`, `--help-types` and `--version` (`App::Ack::show_help`,
//! `show_help_types`, `get_version_statement`). `--help-colors` and `--man`
//! come in Phase 4.

use crate::bytes::lossy;
use crate::config::Types;
use crate::filter::Filter;
use crate::output;

/// `App::Ack::show_help`'s text, synced from ack3 by `scripts/sync-from-ack3`.
const HELP: &str = include_str!("help.txt");

pub fn show_help() {
    output::print(&[HELP.as_bytes()]);
}

/// `--version`. `t/ack-version.t` checks that the first line has the ack version.
pub fn version_text() -> String {
    format!(
        "ack v{} (rask {})\n\n\
         Copyright 2005-2026 Andy Lester.\n\n\
         This program is free software.  You may modify or distribute it\n\
         under the terms of the Artistic License v2.0.\n",
        crate::ACK_VERSION,
        env!("CARGO_PKG_VERSION")
    )
}

/// `$filter->to_string`
pub fn filter_to_string(f: &Filter) -> String {
    match f {
        Filter::Ext { extensions, .. } => extensions
            .iter()
            .map(|e| format!(".{}", lossy(e)))
            .collect::<Vec<_>>()
            .join(" "),
        Filter::Is(name) | Filter::IsPath(name) => lossy(name),
        Filter::Match { source, .. } => format!("Filename matches {}", lossy(source)),
        Filter::FirstLineMatch { source, .. } => {
            // s{\([^:]*:(.*)\)$}{$1}
            let s = lossy(source);
            let inner = s
                .strip_prefix('(')
                .and_then(|rest| rest.find(':').map(|i| &rest[i + 1..]))
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(&s);
            format!("First line matches /{inner}/")
        }
        Filter::Default => "(unimplemented to_string)".into(),
    }
}

pub fn show_help_types(types: &Types) {
    output::print(&[b"Usage: ack [OPTION]... PATTERN [FILES OR DIRECTORIES]

The following is the list of filetypes supported by ack.  You can specify a
filetype to include with -t TYPE or --type=TYPE.  You can exclude a
filetype with -T TYPE or --type=noTYPE.

Note that some files may appear in multiple types.  For example, a file
called Rakefile is both Ruby (--type=ruby) and Rakefile (--type=rakefile).

"]);
    let mut names: Vec<&String> = types.mappings.keys().collect();
    names.sort();
    let maxlen = names.iter().map(|n| n.len()).max().unwrap_or(0);
    for name in names {
        if name.starts_with('-') {
            continue;
        }
        let list: Vec<String> = types.mappings[name]
            .iter()
            .map(|f| filter_to_string(f))
            .collect();
        output::print(&[format!("    {name:<maxlen$} {}\n", list.join("; ")).as_bytes()]);
    }
}

/// `--man`: ack's POD as text (see `scripts/sync-from-ack3`).
pub fn show_man() {
    output::print(&[include_str!("man.txt").as_bytes()]);
}

/// `--thpppt`
pub fn thpppt(arg: &[u8]) -> ! {
    let y = r"_   /|,\'!.x',=(www)=,   U   "
        .replace(',', "\n")
        .replace('x', "O")
        .replace('!', "o")
        .replace('w', "_");
    output::print(&[y.as_bytes(), b" ack ", arg, b"!\n"]);
    output::exit(0)
}

/// `--bar`
pub fn ackbar() -> ! {
    output::print(&[include_str!("bar.txt").as_bytes()]);
    output::exit(0)
}

/// `--cathy`
pub fn cathy() -> ! {
    output::print(&[include_str!("cathy.txt").as_bytes()]);
    output::exit(0)
}
