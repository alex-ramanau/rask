# Benchmarks

Produced by `scripts/bench` (plan item 5.1). Before timing anything, the
script checks that rask's stdout, stderr and exit code are the same as Perl
ack's for every scenario. The last column records that check.

**Result: rask is a median 8.5× faster than Perl ack 3.10.0** (5.4× at
worst), up from 3.1× for the Phase 1 build. Output is the same in every
scenario.

## Method

- Machine: a VirtualBox VM with 4 vCPUs (AMD Ryzen AI MAX+ 395 host), Linux 6.8,
  ext4, warm page cache. Perl 5.38.2.
- Each scenario runs once untimed, then 5 timed runs; the median wall time
  is shown.
- Corpora, all local:
  - **go**: `~/go`, 1.2 GB of Go modules, 33,550 files (31,504 selected)
  - **include**: `/usr/include`, 4,893 C headers
  - **share**: `/usr/share`, 27,068 files, many small and many not text
- `grep -r` is for reference only. It searches every file, binary files
  included, and has no type filters.

## Results

| Corpus | Scenario | Output lines | Perl ack | rask (Phase 1) | rask (now) | Speed-up | grep -r | Same output as Perl |
|---|---|---:|---:|---:|---:|---:|---:|---|
| go | `literal` | 27245 | 1.581s | 0.506s | 0.198s | 8.0× | 0.416s | yes |
| go | `literal -i` | 29664 | 1.763s | 0.523s | 0.221s | 8.0× | 0.571s | yes |
| go | `no match` | 0 | 0.758s | 0.352s | 0.092s | 8.2× | 0.432s | yes |
| go | `alternation` | 25923 | 1.539s | 0.470s | 0.166s | 9.3× | 0.589s | yes |
| go | `regex` | 36851 | 1.637s | 0.757s | 0.245s | 6.7× | 0.559s | yes |
| go | `-w` | 153540 | 3.003s | 0.767s | 0.375s | 8.0× | 0.700s | yes |
| go | `-v -c` | 31504 | 2.727s | 0.744s | 0.273s | 10.0× | 0.778s | yes |
| go | `-l` | 631 | 0.797s | 0.358s | 0.094s | 8.5× | 0.432s | yes |
| go | `-c` | 31504 | 1.864s | 0.639s | 0.226s | 8.2× | 0.487s | yes |
| go | `-f` | 31504 | 0.574s | 0.200s | 0.061s | 9.4× | - | yes |
| go | `--type=cc` | 259 | 0.452s | 0.123s | 0.052s | 8.7× | - | yes |
| include | `literal` | 4571 | 0.287s | 0.078s | 0.031s | 9.3× | 0.059s | yes |
| include | `literal -i` | 4932 | 0.298s | 0.081s | 0.031s | 9.6× | 0.057s | yes |
| include | `no match` | 0 | 0.136s | 0.050s | 0.014s | 9.7× | 0.031s | yes |
| include | `alternation` | 686 | 0.239s | 0.066s | 0.021s | 11.4× | 0.057s | yes |
| include | `regex` | 260 | 0.178s | 0.124s | 0.033s | 5.4× | 0.044s | yes |
| include | `-w` | 137117 | 0.651s | 0.152s | 0.074s | 8.8× | 0.111s | yes |
| include | `-v -c` | 4893 | 0.430s | 0.126s | 0.041s | 10.5× | 0.048s | yes |
| include | `-l` | 478 | 0.144s | 0.054s | 0.015s | 9.6× | 0.032s | yes |
| include | `-c` | 4893 | 0.310s | 0.102s | 0.033s | 9.4× | 0.047s | yes |
| include | `-f` | 4893 | 0.102s | 0.025s | 0.008s | 12.7× | - | yes |
| include | `--type=cc` | 36849 | 0.443s | 0.108s | 0.052s | 8.5× | - | yes |
| share | `literal` | 13733 | 0.869s | 0.302s | 0.128s | 6.8× | 0.302s | yes |
| share | `literal -i` | 16694 | 0.987s | 0.323s | 0.145s | 6.8× | 0.419s | yes |
| share | `no match` | 0 | 0.475s | 0.213s | 0.061s | 7.8× | 0.254s | yes |
| share | `alternation` | 11424 | 1.000s | 0.300s | 0.129s | 7.8× | 0.374s | yes |
| share | `regex` | 4106 | 0.699s | 0.355s | 0.121s | 5.8× | 0.304s | yes |
| share | `-w` | 3492 | 1.149s | 0.360s | 0.164s | 7.0× | 0.366s | yes |
| share | `-v -c` | 15022 | 1.142s | 0.348s | 0.129s | 8.9× | 0.429s | yes |
| share | `-l` | 1217 | 0.517s | 0.234s | 0.066s | 7.8× | 0.256s | yes |
| share | `-c` | 15022 | 0.791s | 0.304s | 0.108s | 7.3× | 0.283s | yes |
| share | `-f` | 15022 | 0.376s | 0.135s | 0.042s | 9.0× | - | yes |
| share | `--type=cc` | 110 | 0.302s | 0.097s | 0.035s | 8.6× | - | yes |

## What made the difference

Measured on `~/go`, one change at a time:

| Change | no-match search | literal search |
|---|---:|---:|
| Phase 1 build | 0.34s | 0.50s |
| Fewer syscalls: file types from `readdir`, one `open` per file shared by the `-T` test and the search, reads sized by `fstat`, uid/groups looked up once | 0.19s | 0.36s |
| Ordered parallel search (`src/parallel.rs`) | 0.09s | 0.20s |
| Workers also find the matching lines, so printing only looks them up | 0.09s | 0.20s (`-w`: 0.47s → 0.37s) |

The syscall counts for a no-match search of `~/go` went from 308k reads,
171k stats and 71k opens to 101k, 24k and 40k.

Not done, and why:

- **A literal or `regex::bytes` fast path.** The regex engine isn't the
  bottleneck. PCRE2's JIT scans a 44 MB file in about 30 ms (`-c` with no
  matches). The time goes to opening, reading and printing files.
- **More threads.** Speed levels off at 2–3 workers on this 4-CPU machine.
  The default is one worker per CPU. `RASK_THREADS` overrides it; it's
  meant for testing, and `RASK_THREADS=1` gives the serial search.
