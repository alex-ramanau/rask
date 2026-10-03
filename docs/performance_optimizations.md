# Performance optimizations (Phase 5)

What was changed to make rask faster, and what each change did. Every
change kept the output byte-identical to Perl ack: the ack3 test suite,
the differential case files and `scripts/bench`'s own output check passed
after each step.

All timings are wall time on `~/go` (1.2 GB of Go modules, 33,550 files)
with a warm page cache, in a 4-vCPU VM. Full tables are in
[`benchmarks.md`](benchmarks.md).

## Summary

| Step | No-match search | Literal (`main`) | Median speed-up over Perl ack |
|---|---:|---:|---:|
| Phase 1 build (baseline) | 0.34 s | 0.50 s | 3.1× |
| 1. Fewer syscalls per file | 0.19 s | 0.36 s | not benchmarked |
| 2. Ordered parallel search | 0.09 s | 0.20 s | 8.0× |
| 3. Matching lines found by the workers | 0.09 s | 0.20 s | 8.0× |
| Final run (5 runs per scenario) | 0.09 s | 0.20 s | **8.5×** |

## Finding the bottleneck

`perf` isn't allowed on this VM, so the time was split up with `time` and
`strace -c`. Kernel time was two thirds of the total, and per file rask made
about 9 `read` calls, 5 `stat` calls, 2 `open` calls and a `geteuid`. A
microbenchmark showed that the regex wasn't the problem: rask scans a 44 MB
file in about 30 ms, about 40 ns per line.

## 1. Fewer syscalls per file

- **File types from `readdir`.** The walker takes each entry's type from the
  directory listing instead of calling `stat` on it. Only symlinks under
  `--follow` still need a `stat`.
- **One open per file.** The file filter opens each file once. A successful
  open is the readability check: Perl's exact `-r`/`-R` logic only runs if
  the open fails. The handle is kept for the binary check and the search.
- **Read the start once.** The first 512 bytes read for the binary check
  (Perl's `-T`) are reused by `firstlinematch:` and by the search. If the
  file is shorter than that, it has already been read completely.
- **Reads sized by `fstat`.** The prescan reads a file in one go at its known
  size, instead of in growing chunks with an extra read to find the end.
- **User and group IDs looked up once**, not once per file.
- **No `stat` for the starting-set check** when every starting point is a
  directory, since a file can't then be one of them.

| syscall (no-match search) | before | after |
|---|---:|---:|
| `read` | 308k | 101k |
| `stat` | 171k | 24k |
| `open` | 71k | 40k |
| `geteuid` | 33k | 0 |

**Impact:** no-match search 0.34 s → 0.19 s, literal search 0.50 s → 0.36 s.
Removing the extra end-of-file read had no measurable effect on its own.

## 2. Ordered parallel search

- A **walker thread** reads directories in File::Next order and numbers each
  entry. Directory errors are numbered too.
- **Worker threads** apply the file filter and do the per-file work: open,
  binary check, prescan, and counting for `-c`/`-l`/`-L`. They capture any
  warnings instead of printing them.
- The **main thread** puts the results back in walk order, prints each
  file's warnings just before it, and runs the unchanged printing code.
  Everything that depends on earlier output (headings, blank lines between
  files, `--` separators, `-1`, `-m`) therefore stays single-threaded.
- The walk can get at most 256 entries ahead of the printing, which bounds
  memory use.

**Impact** (compared with the Phase 1 build):

| Scenario | Before | After |
|---|---:|---:|
| no match | 0.35 s | 0.10 s |
| literal | 0.51 s | 0.20 s |
| `-c` | 0.64 s | 0.23 s |
| `-w` | 0.77 s | 0.47 s |

The median speed-up over Perl ack went from 3.1× to 8.0×.

**Thread count:** speed levels off at 2–3 workers on 4 CPUs (1 thread: 0.42 s,
2 threads: 0.20 s, 3 to 12 threads: 0.18–0.20 s for `main`). The default
is one worker per CPU. `RASK_THREADS` overrides it for testing.

## 3. Matching lines found by the workers

After step 2, files that passed the prescan were still matched line by
line on the main thread. Now the worker also records where each line's
first match is, and the printing code looks that up instead of running the
regex. Which lines match doesn't depend on earlier output, so it's safe to
do this in parallel.

**Impact:** `-w` 0.47 s → 0.37 s, `regex` 0.28 s → 0.23 s.

**A regression it caused, and the fix:** at first the table was also built
in count mode (`-c`, `-l`, `-L`), where the worker then matched every line
a second time. `-c` got slower (0.23 s → 0.38 s), and so did `-v -c`
(0.28 s → 0.48 s). The benchmark caught it. Building the table only when
matches are printed brought both back (0.23 s and 0.28 s).

## CPU time versus wall time

The parallel search finishes sooner but uses more CPU time in total: on
the same searches, 1.2–1.9× the CPU of the single-threaded search, for
2.1–2.4× less wall time. Measured on `~/go`, median of 3 runs; "CPU" is
user plus system time across all threads.

| Search | Mode | Wall time | CPU time (user + sys) |
|---|---|---:|---:|
| no match | Phase 1 build | 0.35 s | 0.35 s (0.10 + 0.25) |
| | now, `RASK_THREADS=1` | 0.19 s | 0.19 s (0.07 + 0.12) |
| | now, parallel | **0.08 s** | 0.25 s (0.09 + 0.16) |
| `main` | Phase 1 build | 0.50 s | 0.49 s (0.25 + 0.24) |
| | now, `RASK_THREADS=1` | 0.42 s | 0.42 s (0.30 + 0.12) |
| | now, parallel | **0.20 s** | 0.57 s (0.42 + 0.15) |
| `-c return` | Phase 1 build | 0.64 s | 0.63 s (0.39 + 0.24) |
| | now, `RASK_THREADS=1` | 0.48 s | 0.48 s (0.36 + 0.12) |
| | now, parallel | **0.23 s** | 0.79 s (0.57 + 0.22) |
| `-w int` | Phase 1 build | 0.77 s | 0.75 s (0.52 + 0.23) |
| | now, `RASK_THREADS=1` | 0.79 s | 0.79 s (0.67 + 0.12) |
| | now, parallel | **0.37 s** | 1.19 s (1.01 + 0.18) |

- **Fewer syscalls (step 1) saves CPU as well as wall time.** Single-threaded,
  system time roughly halved, and total CPU is below the Phase 1 build in
  three of the four searches.
- **Parallelism (step 2) adds CPU time.** Likely causes are handing files
  between threads, putting results back in order, and threads competing
  for caches and memory in the VM. This hasn't been profiled, because
  `perf` isn't allowed here.
- **`-w` is the exception:** even single-threaded it uses a little more CPU
  than the Phase 1 build (0.79 s vs 0.75 s of user and system time). The
  per-line match table seems to cost something there; the cause hasn't
  been found.
- For a no-match search, the parallel search uses less CPU in total than
  the Phase 1 build did (0.25 s vs 0.35 s).

Where total CPU matters more than latency, such as on a busy shared
machine, `RASK_THREADS=1` gives the single-threaded search.

## Not done

- **A fast path for literal patterns, or through Rust's `regex` crate.** The
  regex engine isn't where the time goes; opening, reading and printing are.
- **Memory-mapped files.** Not tried. Most files are already read with a
  single `read` call, so the gain is probably small, but it hasn't been
  measured.
