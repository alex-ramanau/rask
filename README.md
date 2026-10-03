# rask

A Rust reimplementation of [ack 3](https://beyondgrep.com/), the grep-like
source code search tool. The target is byte-for-byte parity with Perl ack
3.10.0: same options, same output, same exit codes, same `.ackrc` handling.

**Status: Phases 0–2 done.** rask passes the whole ack3 test suite on Linux
(87/87 test files, 839/839 tests; 12 tests of Perl internals are skipped,
see `rust-skip.txt`). Known gaps are listed in the plan
(`ack3/ack_rewrite_in_rust/phases/`).

## Layout

| Path | What |
|---|---|
| `src/` | the `rask` binary; one module per area (`cli`, `config`, `walk`, `filter`, `matcher`, `search`, `output`, `help`) |
| `scripts/ack-suite` | runs ack3's own test suite against the rask binary and reports pass counts |
| `scripts/diff-ack`, `tests/diff/` | runs Perl ack and rask on the same command lines and compares stdout, stderr and exit code |
| `scripts/texttest-vs-perl` | property test: the port of Perl's `-T` against Perl, on thousands of random files |
| `tests/ack3_filters.rs` | Rust ports of ack3's Perl-internals filter and iterator tests (need `ACK3_DIR`) |
| `scripts/sync-from-ack3` | regenerates the text rask embeds from ack3: default ackrc, `--help`, `--man`, `--bar`, `--cathy` |
| `rust-skip.txt` | ack3 tests that only exercise Perl internals, with their planned Rust counterparts |
| `scripts/regex-spike`, `tools/regex-spike/` | the regex engine comparison behind decision D1 |
| `docs/regex-compat.md` | regex decisions and the Perl behaviours rask must emulate |
| `ci/ack3-test-binary.patch` | the `ACK_TEST_BINARY` harness change for ack3, applied in CI until merged upstream |

## Running the ack3 test suite

Perl ack is the spec, and its test suite is the conformance test. You need
an ack3 checkout next to this one (or set `ACK3_DIR`), with the
`ACK_TEST_BINARY` harness change and a built `blib/`:

```sh
cd ../ack3 && perl Makefile.PL && make && cd -   # needs File::Next and YAML::PP
cargo build
scripts/ack-suite                      # summary
scripts/ack-suite --markdown out.md    # plus a per-file table
scripts/ack-suite t/ack-w.t            # selected tests
scripts/diff-ack -v -w foo t/text      # compare one command line with Perl ack
scripts/diff-ack --cases tests/diff/phase1.txt
scripts/diff-ack --cases tests/diff/phase2.txt
ACK3_DIR=../ack3 cargo test            # includes the ports of ack3's filter tests
```

## How the code maps to Perl ack

| rask | Perl ack |
|---|---|
| `src/cli/getopt.rs` | Getopt::Long 2.54 (the parts ack configures) |
| `src/config/` | `App::Ack::ConfigLoader`, `ConfigFinder`, `ConfigDefault` |
| `src/filter/` | `App::Ack::File`, `App::Ack::Filter::*`, Perl's `-T` |
| `src/walk/` | File::Next 1.18, `File::Spec::Unix` |
| `src/matcher/` | `App::Ack::build_regex` and friends; Perl regex diagnostics |
| `src/search/` | ack's file loops and `pmif_*` printers |
| `src/output/` | `App::Ack::say`/`warn`/`die`, pager, Term::ANSIColor |
| `src/help/` | `--help`, `--help-types`, `--man`, `--version`, easter eggs |
| `src/filetest.rs` | Perl's `-r` and `-R` |
| `src/app.rs` | ack's `MAIN` block and file filters (`src/main.rs` just calls it) |

## Building on a VirtualBox shared folder

The linker writes corrupt binaries on `vboxsf` mounts (build scripts fail
with "Exec format error"). Put the target directory on a local disk:

```sh
export CARGO_TARGET_DIR=$HOME/.cache/rask-target
```

The scripts honour `CARGO_TARGET_DIR`.

## Licence

Artistic License 2.0, like ack.
