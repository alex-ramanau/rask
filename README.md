# rask

**rask is [ack](https://beyondgrep.com/), the grep-like search tool for
programmers, rewritten in Rust.**

It behaves like ack 3.10.0: the same options, the same output, the same exit
codes and the same `.ackrc` files. If you know ack, you already know rask.
It's just faster: a median **8.5× faster** than Perl ack on the
[benchmarks](docs/benchmarks.md), with identical output.

```console
$ rask -w threads src
src/parallel.rs
3://! The walk, the per-file work and the printing run on different threads:
8://! - worker threads run the file filter and `Settings::prepare` (opening,
48:/// The number of worker threads: the CPU count, or `RASK_THREADS` (for
```

## Why ack (and rask)?

Compared with `grep -r`, ack is made for searching source code:

- **It searches the current directory tree by default.** No `-r`, no `.`.
- **It skips what you don't want to see:** version control directories
  (`.git`, `.svn`, ...), build output, backup files, minified JavaScript,
  core dumps and binary files.
- **It knows file types.** `--python`, `--js`, `--rust`, `--type=noxml`, and
  about 80 more (`rask --help-types`).
- **It uses Perl-compatible regular expressions** (PCRE2 in rask), with
  `\d`, `\b`, lookarounds and the rest.
- **Its output is easy to read:** matches are highlighted and grouped under
  each file name.

rask adds:

- **Speed.** Files are searched in parallel, and the output still comes
  out in the same order as ack's.
- **One self-contained binary.** No Perl and no CPAN modules to install.

## Installing

### Snap (Linux)

Once rask is published in the Snap Store:

```sh
sudo snap install rask --classic
```

Until then, build from source (below), or build the snap yourself with
`make snap` (see [docs/snap.md](docs/snap.md)).

The snap command is `rask`. To run it as `ack` too:

```sh
sudo snap alias rask ack
```

### From source

You need [Rust](https://rustup.rs/) 1.85 or newer and a C compiler (the
PCRE2 library is built from source and linked in).

```sh
git clone https://github.com/alex-ramanau/rask.git
cd rask
make install          # installs to ~/.local/bin/rask
```

To install system-wide instead, run `make` and then
`sudo install -m755 target/release/rask /usr/local/bin/`. Or use
`cargo install --path .` if you'd rather not use `make`.

rask is developed and tested on Linux. It builds and passes its own tests
on macOS and Windows too, but hasn't been checked against the whole ack test
suite there yet.

## Examples

Search the current directory tree for a pattern:

```sh
rask cache_size
```

Whole words only, ignoring case:

```sh
rask -w -i timeout
```

A literal string, with no regex meaning for `(`, `.`, `*` and so on:

```sh
rask -Q 'config.get('
```

### Choosing files

Only Python files, or everything except JavaScript:

```sh
rask --python import
rask --type=nojs TODO
```

Search only some directories or files:

```sh
rask handle_request src/ tests/
```

List the files rask would search, without searching them (`-f`), or only
those whose name matches a pattern (`-g`):

```sh
rask -f --rust
rask -g '_test\.go$'
```

See which file types rask knows and how it recognises them:

```sh
rask --help-types
```

Add your own type, for example for Jinja templates:

```sh
rask --type-add=jinja:ext:j2,jinja --jinja block
```

### Shaping the output

Only the names of matching files (`-l`), or of files with no match (`-L`):

```sh
rask -l deprecated
rask -L 'Copyright'
```

Count matches per file:

```sh
rask -c TODO
```

Show 2 lines of context around each match (`-C`), or 3 after (`-A 3`) or
before (`-B 3`):

```sh
rask -C parse_args
```

Print only the matched text (`-o`), or build each output line from the
match (`--output`, with `$1`..`$9`, `$&`, `$_`, `$.` for the line number and
`$f` for the file name):

```sh
rask -o 'https?://\S+'
rask --output='$1' 'version = "([^"]+)"' Cargo.toml
rask -h --output='$f:$.:$&' 'TODO.*'   # -h: no extra file:line prefix
```

### Combining patterns

Lines that contain `fn` and `pub`, but not `test`:

```sh
rask fn --and pub --not test
```

Search only between a start and an end pattern. Here `.` matches every
line, so this prints one section of a file:

```sh
rask --range-start='^\[dependencies\]' --range-end='^$' . Cargo.toml
```

### In pipelines

rask reads standard input when it's given no files, like grep:

```sh
git log --oneline | rask -i fix
```

Feed it a list of files with `-x` (or `--files-from=FILE`):

```sh
git diff --name-only | rask -x console.log
```

The exit code is 0 if something matched and 1 if nothing did, so rask
works in scripts:

```sh
if rask -l 'password\s*=' config/ > /dev/null; then echo 'found a password'; fi
```

## Configuration

Default options go in an `.ackrc` file, one option per line, exactly as for
ack. rask reads `/etc/ackrc`, then `~/.ackrc` (or the file named by `$ACKRC`),
then the nearest `.ackrc` in the current directory or above it. For example:

```
# ~/.ackrc
--smart-case
--ignore-dir=node_modules
--type-add=jinja:ext:j2,jinja
--pager=less -R
```

Useful commands:

- `rask --create-ackrc` prints ack's built-in defaults as a starting point.
- `rask --dump` shows which options are loaded, and from which file.
- `rask --noenv` ignores every `.ackrc` file and `ACK_*` environment variable.

The environment variables ack uses work too: `ACK_OPTIONS`, `ACK_PAGER`,
`ACK_PAGER_COLOR`, `ACK_COLOR_MATCH` and the other `ACK_COLOR_*` variables,
and `NO_COLOR`.

## Documentation

- `rask --help` for a summary of the options.
- `rask --man` for the full manual (it's ack's manual: everything in it
  applies to rask).
- The [ack website](https://beyondgrep.com/documentation/) has more, including
  how to [switch from grep](https://beyondgrep.com/why-ack/) and a
  [comparison with similar tools](https://beyondgrep.com/feature-comparison/).

## Differences from ack

rask aims to have none in what it prints. It passes ack's own test suite on
Linux (839 of 839 tests). The tests of ack's Perl internals don't apply; rask
has its own tests for those parts.

What's different:

- `rask --version` prints `ack v3.10.0 (rask 0.1.0)`, so tools that check
  ack's version still work.
- Matching uses PCRE2 rather than Perl's regex engine. The two agree on
  almost everything ack users write, and rask reproduces Perl's behaviour
  and error messages where they differ; the details are in
  [docs/regex-compat.md](docs/regex-compat.md).

If you find a case where rask and ack disagree, please
[open an issue](https://github.com/alex-ramanau/rask/issues) with the command
line and both outputs.

## Credits

ack was created by **[Andy Lester](https://github.com/petdance)** in 2005,
and is maintained by him with many contributors. Everything rask does, from
the options to the file types to the output, comes from ack's design, and its
help text and manual are ack's own. rask exists only because ack is worth
having faster. Thank you, Andy.

- ack's website: <https://beyondgrep.com/>
- ack's source code: <https://github.com/beyondgrep/ack3>

rask also relies on [PCRE2](https://github.com/PCRE2Project/pcre2) by Philip
Hazel and its maintainers, and on the Rust [`pcre2`](https://crates.io/crates/pcre2)
crate by Andrew Gallant.

## Contributing

See [docs/development.md](docs/development.md) for how the code is organised,
how to build it, and how to run ack's test suite against rask.

## Licence

Artistic License 2.0, the same as ack. ack is copyright 2005–2026 Andy Lester.
