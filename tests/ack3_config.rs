//! Ports of t/config-loader.t and t/config-finder.t, which test
//! App::Ack::ConfigLoader::process_args and App::Ack::ConfigFinder directly
//! (see rust-skip.txt). They make their own fixtures in temporary
//! directories, so they don't need an ack3 checkout.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rask::bytes::{lossy, path_bytes};
use rask::config::finder::{ConfigFile, find_config_files_from};
use rask::config::{Opt, Source, process_args};
use rask::env::with_overrides;
use rask::filter::Filter;

// ---- t/config-loader.t ----

/// `test_loader`: runs process_args on an ARGV source alone.
fn load(argv: &[&str], env: &[(&str, Option<&str>)]) -> Opt {
    let mut vars = vec![
        ("PAGER", None),
        ("ACK_PAGER", None),
        ("ACK_PAGER_COLOR", None),
    ];
    for (k, v) in env {
        vars.retain(|(name, _)| name != k);
        vars.push((k, *v));
    }
    with_overrides(&vars, || {
        let source = Source {
            name: "ARGV".into(),
            contents: argv.iter().map(|a| a.as_bytes().to_vec()).collect(),
            is_ackrc: false,
            project: false,
        };
        let processed = process_args(vec![source]);
        assert!(processed.argv.is_empty(), "got targets for {argv:?}");
        processed.opt
    })
}

/// The options t/config-loader.t checks, as text, so a mismatch shows them all.
fn summary(o: &Opt) -> String {
    let filters: Vec<&str> = o
        .filters
        .iter()
        .map(|f| match (&*f.filter, f.inverted) {
            (Filter::Default, false) => "Default",
            _ => "other",
        })
        .collect();
    format!(
        "A={:?} B={:?} pager={:?} filters={filters:?} and={} or={} not={} break={:?} c={} color={:?} \
         column={} debug={} f={} files_from={:?} follow={} g={} h={:?} H={:?} heading={:?} l={} L={} \
         m={:?} n={} output={:?} p={:?} passthru={} print0={} Q={} range_start={:?} range_end={:?} \
         range_invert={} regex={:?} s={} show_types={} sort_files={} underline={} v={} w={}",
        o.a,
        o.b,
        o.pager.as_ref().map(|p| lossy(p)),
        o.and.len(),
        o.or.len(),
        o.not.len(),
        o.break_,
        o.c,
        o.color,
        o.column,
        o.debug,
        o.f,
        o.files_from,
        o.follow,
        o.g,
        o.h,
        o.big_h,
        o.heading,
        o.l,
        o.big_l,
        o.m,
        o.n,
        o.output,
        o.p,
        o.passthru,
        o.print0,
        o.regex_opts.literal,
        o.range_start,
        o.range_end,
        o.range_invert,
        o.regex,
        o.s,
        o.show_types,
        o.sort_files,
        o.underline,
        o.v,
        o.regex_opts.w,
    )
}

/// The defaults, with A, B and the pager changed.
fn expected(a: Option<i64>, b: Option<i64>, pager: Option<&str>) -> String {
    let o = Opt {
        a,
        b,
        pager: pager.map(|p| p.as_bytes().to_vec()),
        filters: vec![rask::filter::FilterRef {
            filter: std::sync::Arc::new(Filter::Default),
            inverted: false,
        }],
        ..Opt::default()
    };
    summary(&o)
}

fn check(argv: &[&str], env: &[(&str, Option<&str>)], want: String) {
    assert_eq!(summary(&load(argv, env)), want, "ack {argv:?} with {env:?}");
}

#[test]
fn empty_inputs_give_defaults() {
    check(&[], &[], expected(None, None, None));
}

#[test]
fn after_and_before_context() {
    for (long, short, is_a) in [
        ("after-context", "-A", true),
        ("before-context", "-B", false),
    ] {
        let want = |n: i64| {
            if is_a {
                expected(Some(n), None, None)
            } else {
                expected(None, Some(n), None)
            }
        };
        check(&[&format!("--{long}=15")], &[], want(15));
        check(&[&format!("--{long}=0")], &[], want(0));
        check(&[&format!("--{long}")], &[], want(2));
        check(&[&format!("--{long}=-43")], &[], want(2));
        check(&[short, "15"], &[], want(15));
        check(&[short, "0"], &[], want(0));
        check(&[short], &[], want(2));
        check(&[short, "-43"], &[], want(2));
    }
}

#[test]
fn context() {
    let both = |n| expected(Some(n), Some(n), None);
    check(&["-C", "5"], &[], both(5));
    check(&["-C"], &[], both(2));
    check(&["-C", "0"], &[], both(0));
    check(&["-C", "-43"], &[], both(2));
    check(&["--context=5"], &[], both(5));
    check(&["--context"], &[], both(2));
    check(&["--context=0"], &[], both(0));
    check(&["--context=-43"], &[], both(2));
}

#[test]
fn ack_pager() {
    let env = [("ACK_PAGER", Some("t/test-pager --skip=2"))];
    check(
        &[],
        &env,
        expected(None, None, Some("t/test-pager --skip=2")),
    );
    check(
        &["--pager=t/test-pager"],
        &env,
        expected(None, None, Some("t/test-pager")),
    );
    check(&["--nopager"], &env, expected(None, None, None));
}

#[test]
fn ack_pager_color() {
    let env = [("ACK_PAGER_COLOR", Some("t/test-pager --skip=2"))];
    check(
        &[],
        &env,
        expected(None, None, Some("t/test-pager --skip=2")),
    );
    check(
        &["--pager=t/test-pager"],
        &env,
        expected(None, None, Some("t/test-pager")),
    );
    check(&["--nopager"], &env, expected(None, None, None));

    let env = [
        ("ACK_PAGER_COLOR", Some("t/test-pager --skip=2")),
        ("ACK_PAGER", Some("t/test-pager --skip=3")),
    ];
    check(
        &[],
        &env,
        expected(None, None, Some("t/test-pager --skip=2")),
    );
    check(
        &["--pager=t/test-pager"],
        &env,
        expected(None, None, Some("t/test-pager")),
    );
    check(&["--nopager"], &env, expected(None, None, None));
}

#[test]
fn pager_variable() {
    let env = [("PAGER", Some("t/test-pager"))];
    check(&[], &env, expected(None, None, None));
    check(
        &["--pager"],
        &env,
        expected(None, None, Some("t/test-pager")),
    );
    check(
        &["--pager=t/test-pager --skip=2"],
        &env,
        expected(None, None, Some("t/test-pager --skip=2")),
    );
}

// ---- t/config-finder.t ----

/// A fresh temporary directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rask-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Resolve symlinks (macOS's /tmp), so paths compare cleanly.
        TempDir(std::fs::canonicalize(&dir).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn touch(path: &Path) {
    std::fs::write(path, "").unwrap();
}

fn rm(path: &Path) {
    std::fs::remove_file(path).unwrap();
}

/// `realpath`, or the path itself if it doesn't exist.
fn real(path: &[u8]) -> String {
    let p = rask::bytes::to_path(path);
    lossy(
        &std::fs::canonicalize(&p)
            .map(|r| path_bytes(&r))
            .unwrap_or_else(|_| path.to_vec()),
    )
}

/// The global ackrc files, which are always in the list.
fn global_files() -> Vec<(String, bool)> {
    if cfg!(windows) {
        ["ProgramData", "APPDATA"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|dir| (real(format!("{dir}\\ackrc").as_bytes()), false))
            .collect()
    } else {
        vec![(real(b"/etc/ackrc"), false)]
    }
}

/// `expect_ackrcs`: find_config_files from `cwd` with `home` as HOME and
/// `ackrc` as ACKRC, compared after realpath, as in the Perl test.
fn expect(
    cwd: &Path,
    home: Option<&Path>,
    ackrc: Option<&Path>,
    want: &[(&Path, bool)],
    name: &str,
) {
    let home = home.map(|h| h.to_string_lossy().into_owned());
    let ackrc = ackrc.map(|a| a.to_string_lossy().into_owned());
    let got: Vec<ConfigFile> = with_overrides(
        &[("HOME", home.as_deref()), ("ACKRC", ackrc.as_deref())],
        || find_config_files_from(cwd).unwrap_or_else(|e| panic!("{name}: {e}")),
    );
    let got: Vec<(String, bool)> = got.iter().map(|c| (real(&c.path), c.project)).collect();
    let mut expected = global_files();
    expected.extend(
        want.iter()
            .map(|(p, project)| (real(&path_bytes(p)), *project)),
    );
    assert_eq!(got, expected, "{name}");
}

#[test]
fn config_finder() {
    let home_dir = TempDir::new("home");
    let home = home_dir.0.as_path();
    let user = home.join(".ackrc");
    touch(&user);
    let tmp = TempDir::new("finder");
    let t = tmp.0.as_path();

    expect(
        t,
        Some(home),
        None,
        &[(&user, false)],
        "having no project file should return only the top level files",
    );
    expect(
        t,
        None,
        None,
        &[],
        "only system-wide ackrc is returned if HOME is not defined",
    );

    let baz = t.join("foo/bar/baz");
    std::fs::create_dir_all(&baz).unwrap();

    // For each place a project file can be, with HOME set and not.
    let cases: [(PathBuf, &str); 6] = [
        (
            baz.join(".ackrc"),
            "a project file in the same directory should be detected",
        ),
        (
            t.join("foo/bar/.ackrc"),
            "a project file in the parent directory should be detected",
        ),
        (
            t.join("foo/.ackrc"),
            "a project file in the grandparent directory should be detected",
        ),
        (
            baz.join("_ackrc"),
            "a project _ackrc in the same directory should be detected",
        ),
        (
            t.join("foo/bar/_ackrc"),
            "a project _ackrc in the parent directory should be detected",
        ),
        (
            t.join("foo/_ackrc"),
            "a project _ackrc in the grandparent directory should be detected",
        ),
    ];
    for (project, name) in &cases {
        touch(project);
        expect(
            &baz,
            Some(home),
            None,
            &[(&user, false), (project, true)],
            name,
        );
        expect(&baz, None, None, &[(project, true)], name);
        rm(project);
    }

    // The nearest project file wins.
    for (near, far) in [
        (baz.join(".ackrc"), t.join("foo/.ackrc")),
        (baz.join("_ackrc"), t.join("foo/_ackrc")),
    ] {
        touch(&near);
        touch(&far);
        let name = "a project file in the same directory should be detected, even with another one above it";
        expect(
            &baz,
            Some(home),
            None,
            &[(&user, false), (&near, true)],
            name,
        );
        expect(&baz, None, None, &[(&near, true)], name);
        rm(&near);
        rm(&far);
    }

    // .ackrc and _ackrc in the same directory is an error.
    touch(&baz.join(".ackrc"));
    touch(&baz.join("_ackrc"));
    for h in [Some(home), None] {
        let h = h.map(|h| h.to_string_lossy().into_owned());
        let err = with_overrides(&[("HOME", h.as_deref()), ("ACKRC", None)], || {
            find_config_files_from(&baz)
        })
        .err()
        .expect(".ackrc + _ackrc is an error");
        assert!(err.contains("contains both .ackrc and _ackrc"), "{err}");
    }
    rm(&baz.join(".ackrc"));
    let far = t.join("foo/.ackrc");
    touch(&far);
    let name = "a lower-level _ackrc should be preferred to a higher-level .ackrc";
    expect(
        &baz,
        Some(home),
        None,
        &[(&user, false), (&baz.join("_ackrc"), true)],
        name,
    );
    expect(&baz, None, None, &[(&baz.join("_ackrc"), true)], name);
    rm(&baz.join("_ackrc"));

    // HOME is an ancestor of cwd: its .ackrc is the user file and the project file.
    let foo = t.join("foo");
    expect(
        &baz,
        Some(&foo),
        None,
        &[(&far, false)],
        "Don't load the same ackrc file twice",
    );

    // ACKRC overrides HOME's ackrc, but only if it exists.
    let ackrc = t.join("custom-ackrc");
    touch(&ackrc);
    expect(
        t,
        Some(&foo),
        Some(&ackrc),
        &[(&ackrc, false)],
        "ACKRC overrides user's HOME ackrc",
    );
    rm(&ackrc);
    expect(
        t,
        Some(&foo),
        Some(&ackrc),
        &[(&far, false)],
        "ACKRC doesn't override if it doesn't exist",
    );
    touch(&ackrc);
    expect(
        &foo,
        Some(&foo),
        Some(&ackrc),
        &[(&ackrc, false), (&far, true)],
        "~/.ackrc should still be found as a project ackrc",
    );
}
