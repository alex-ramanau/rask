# rask

A Rust reimplementation of [ack 3](https://beyondgrep.com/), the grep-like
source code search tool. The target is byte-for-byte parity with Perl ack
3.10.0: same options, same output, same exit codes, same `.ackrc` handling.

**Status: Phase 0 (groundwork).** The binary doesn't search yet.

## Layout

| Path | What |
|---|---|
| `src/` | the `rask` binary; one module per area (`cli`, `config`, `walk`, `filter`, `matcher`, `search`, `output`, `help`) |
| `scripts/ack-suite` | runs ack3's own test suite against the rask binary and reports pass counts |
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
```

## Building on a VirtualBox shared folder

The linker writes corrupt binaries on `vboxsf` mounts (build scripts fail
with "Exec format error"). Put the target directory on a local disk:

```sh
export CARGO_TARGET_DIR=$HOME/.cache/rask-target
```

The scripts honour `CARGO_TARGET_DIR`.

## Licence

Artistic License 2.0, like ack.
