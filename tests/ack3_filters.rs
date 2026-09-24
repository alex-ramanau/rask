//! Ports of the ack3 tests that exercise Perl internals directly (see
//! rust-skip.txt): t/filter.t, t/ext-filter.t, t/is-filter.t,
//! t/match-filter.t, t/firstlinematch-filter.t, t/default-filter.t and
//! t/file-iterator.t. They use the fixtures in an ack3 checkout, found
//! through ACK3_DIR or at ../ack3, and are skipped if there isn't one.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rask::bytes::{lossy, path_bytes};
use rask::filter::{File, Filter, FilterContext};
use rask::walk::{Files, WalkOptions};

fn ack3() -> Option<PathBuf> {
    let dir = std::env::var_os("ACK3_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ack3"));
    if dir.join("t/swamp").is_dir() {
        Some(dir)
    } else {
        eprintln!(
            "skipping: no ack3 checkout at {} (set ACK3_DIR)",
            dir.display()
        );
        None
    }
}

/// `File::Next::files( 't/swamp' )` with File::Next's defaults (no filters,
/// following symlinks), with names relative to the ack3 checkout.
fn swamp_files(ack3: &std::path::Path) -> Vec<(String, File)> {
    let root = path_bytes(ack3);
    let start = [root.as_slice(), b"/t/swamp"].concat();
    let opts = WalkOptions {
        file_filter: None,
        descend_filter: None,
        error_handler: Box::new(|msg| panic!("{msg}")),
        sort_files: false,
        follow_symlinks: true,
    };
    Files::new(opts, &[start])
        .map(|name| {
            let relative = lossy(&name[root.len() + 1..]);
            (relative, File::new(name))
        })
        .collect()
}

/// `FilterTest::filter_test`: the swamp files the filter accepts.
fn filter_test(kind: &str, args: &[&str], expected: &[&str]) {
    let Some(ack3) = ack3() else { return };
    let args: Vec<Vec<u8>> = args.iter().map(|a| a.as_bytes().to_vec()).collect();
    let filter = Filter::create(kind.as_bytes(), &args).expect("filter");
    let ctx = FilterContext {
        report_bad_filenames: true,
    };
    let got: BTreeSet<String> = swamp_files(&ack3)
        .into_iter()
        .filter(|(_, file)| filter.matches(file, &ctx))
        .map(|(name, _)| name)
        .collect();
    let want: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "{kind} filter");
}

#[test]
fn unknown_filter_fails() {
    // t/filter.t. (Its register_filter half has no Rust counterpart: the
    // filter types are a closed set.)
    let err = Filter::create(b"test", &[]).err().expect("should fail");
    assert!(err.to_lowercase().contains("unknown filter"), "{err}");
}

#[test]
fn ext_filter() {
    filter_test(
        "ext",
        &["pl", "pod", "pm", "t"],
        &[
            "t/swamp/Makefile.PL",
            "t/swamp/blib/ignore.pm",
            "t/swamp/blib/ignore.pod",
            "t/swamp/constitution-100k.pl",
            "t/swamp/options-crlf.pl",
            "t/swamp/options.pl",
            "t/swamp/perl-test.t",
            "t/swamp/perl.handler.pod",
            "t/swamp/perl.pl",
            "t/swamp/perl.pm",
            "t/swamp/perl.pod",
        ],
    );
}

#[test]
fn is_filter() {
    filter_test("is", &["Makefile"], &["t/swamp/Makefile"]);
}

#[test]
fn match_filter() {
    filter_test(
        "match",
        &["/^.akefile/"],
        &[
            "t/swamp/Makefile",
            "t/swamp/Makefile.PL",
            "t/swamp/Rakefile",
        ],
    );
}

#[test]
fn firstlinematch_filter() {
    filter_test(
        "firstlinematch",
        &["/^#!.*perl/"],
        &[
            "t/swamp/#emacs-workfile.pl#",
            "t/swamp/0",
            "t/swamp/Makefile.PL",
            "t/swamp/options-crlf.pl",
            "t/swamp/options.pl",
            "t/swamp/options.pl.bak",
            "t/swamp/perl-test.t",
            "t/swamp/perl-without-extension",
            "t/swamp/perl.cgi",
            "t/swamp/perl.pl",
            "t/swamp/perl.pm",
            "t/swamp/blib/ignore.pm",
            "t/swamp/blib/ignore.pod",
        ],
    );
}

#[test]
fn default_filter() {
    let Some(ack3) = ack3() else { return };
    let ctx = FilterContext {
        report_bad_filenames: true,
    };
    let got: BTreeSet<String> = swamp_files(&ack3)
        .into_iter()
        .filter(|(_, file)| Filter::Default.matches(file, &ctx))
        .map(|(name, _)| name)
        .collect();
    let want: BTreeSet<String> = DEFAULT_FILTER_EXPECTED
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(got, want);
}

#[test]
fn file_iterator_unfiltered() {
    let Some(ack3) = ack3() else { return };
    let got: BTreeSet<String> = swamp_files(&ack3)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let mut want: BTreeSet<String> = DEFAULT_FILTER_EXPECTED
        .iter()
        .map(|s| s.to_string())
        .collect();
    // The binary files the default filter leaves out.
    for binary in [
        "t/swamp/bad-ascii.tar.GZ",
        "t/swamp/favicon.ico",
        "t/swamp/moose-andy.jpg",
        "t/swamp/perl.tar.gz",
        "t/swamp/perltoot.jpg",
        "t/swamp/solution8.tar",
    ] {
        want.insert(binary.to_string());
    }
    assert_eq!(got, want);
}

/// From t/default-filter.t: the swamp files that pass Perl's `-T`.
const DEFAULT_FILTER_EXPECTED: &[&str] = &[
    "t/swamp/#emacs-workfile.pl#",
    "t/swamp/0",
    "t/swamp/constitution-100k.pl",
    "t/swamp/c-header.h",
    "t/swamp/c-source.c",
    "t/swamp/crystallography-weenies.f",
    "t/swamp/example.R",
    "t/swamp/file.bar",
    "t/swamp/file.foo",
    "t/swamp/foo_test.py",
    "t/swamp/fresh.css",
    "t/swamp/fresh.min.css",
    "t/swamp/fresh.css.min",
    "t/swamp/html.htm",
    "t/swamp/html.html",
    "t/swamp/incomplete-last-line.txt",
    "t/swamp/javascript.js",
    "t/swamp/lua-shebang-test",
    "t/swamp/Makefile",
    "t/swamp/Makefile.PL",
    "t/swamp/MasterPage.master",
    "t/swamp/minified.js.min",
    "t/swamp/minified.min.js",
    "t/swamp/not-an-#emacs-workfile#",
    "t/swamp/notaMakefile",
    "t/swamp/notaRakefile",
    "t/swamp/notes.md",
    "t/swamp/options-crlf.pl",
    "t/swamp/options.pl",
    "t/swamp/options.pl.bak",
    "t/swamp/perl-test.t",
    "t/swamp/perl-without-extension",
    "t/swamp/perl.cgi",
    "t/swamp/perl.handler.pod",
    "t/swamp/perl.pl",
    "t/swamp/perl.pm",
    "t/swamp/perl.pod",
    "t/swamp/pipe-stress-freaks.F",
    "t/swamp/Rakefile",
    "t/swamp/Sample.ascx",
    "t/swamp/Sample.asmx",
    "t/swamp/sample.asp",
    "t/swamp/sample.aspx",
    "t/swamp/sample.rake",
    "t/swamp/service.svc",
    "t/swamp/test.py",
    "t/swamp/test_foo.py",
    "t/swamp/blib/ignore.pm",
    "t/swamp/blib/ignore.pod",
    "t/swamp/groceries/fruit",
    "t/swamp/groceries/junk",
    "t/swamp/groceries/meat",
    "t/swamp/groceries/another_subdir/fruit",
    "t/swamp/groceries/another_subdir/junk",
    "t/swamp/groceries/another_subdir/meat",
    "t/swamp/groceries/another_subdir/CVS/fruit",
    "t/swamp/groceries/another_subdir/CVS/junk",
    "t/swamp/groceries/another_subdir/CVS/meat",
    "t/swamp/groceries/another_subdir/RCS/fruit",
    "t/swamp/groceries/another_subdir/RCS/junk",
    "t/swamp/groceries/another_subdir/RCS/meat",
    "t/swamp/groceries/dir.d/fruit",
    "t/swamp/groceries/dir.d/junk",
    "t/swamp/groceries/dir.d/meat",
    "t/swamp/groceries/dir.d/CVS/fruit",
    "t/swamp/groceries/dir.d/CVS/junk",
    "t/swamp/groceries/dir.d/CVS/meat",
    "t/swamp/groceries/dir.d/RCS/fruit",
    "t/swamp/groceries/dir.d/RCS/junk",
    "t/swamp/groceries/dir.d/RCS/meat",
    "t/swamp/groceries/CVS/fruit",
    "t/swamp/groceries/CVS/junk",
    "t/swamp/groceries/CVS/meat",
    "t/swamp/groceries/RCS/fruit",
    "t/swamp/groceries/RCS/junk",
    "t/swamp/groceries/RCS/meat",
    "t/swamp/groceries/subdir/fruit",
    "t/swamp/groceries/subdir/junk",
    "t/swamp/groceries/subdir/meat",
    "t/swamp/stuff.cmake",
    "t/swamp/CMakeLists.txt",
    "t/swamp/swamp/ignoreme.txt",
];
