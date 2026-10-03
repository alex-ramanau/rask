# Benchmarks

Produced by `scripts/bench` (plan item 5.1). Before timing anything, the
script checks that rask's stdout, stderr and exit code are the same as Perl
ack's for every scenario. The last column records that check.

**Result: rask is a median 8.1× faster than Perl ack 3.10.0** (4.9× at
worst), with the same output in every scenario. Measured on 2026-10-03.

## Method

- Machine: a VirtualBox VM with 4 vCPUs (AMD Ryzen AI MAX+ 395 host), Linux 6.8,
  ext4, warm page cache. Perl 5.38.2.
- Each scenario runs once untimed, then 5 timed runs (`scripts/bench --runs 5`);
  the median wall time is shown.
- Corpora, all local:
  - **go**: `~/go`, 1.2 GB of Go modules, 33,550 files (31,504 selected)
  - **include**: `/usr/include`, 5,111 files, mostly C headers
  - **share**: `/usr/share`, 27,106 files (15,051 selected), many small and many not text
- `grep -r` is for reference only. It searches every file, binary files
  included, and has no type filters.

## Results

| Corpus | Scenario | Output lines | Perl ack | rask (2026-10-03) | Speed-up | grep -r | Same output as Perl |
|---|---|---:|---:|---:|---:|---:|---|
| go | `literal` | 27245 | 1.595s | 0.197s | 8.1× | 0.432s | yes |
| go | `literal -i` | 29664 | 1.742s | 0.214s | 8.1× | 0.584s | yes |
| go | `no match` | 0 | 0.756s | 0.089s | 8.5× | 0.439s | yes |
| go | `alternation` | 25923 | 1.547s | 0.163s | 9.5× | 0.602s | yes |
| go | `regex` | 36851 | 1.693s | 0.236s | 7.2× | 0.568s | yes |
| go | `-w` | 153540 | 3.019s | 0.392s | 7.7× | 0.704s | yes |
| go | `-v -c` | 31504 | 2.726s | 0.291s | 9.4× | 0.791s | yes |
| go | `-l` | 631 | 0.821s | 0.095s | 8.6× | 0.447s | yes |
| go | `-c` | 31504 | 1.904s | 0.236s | 8.1× | 0.501s | yes |
| go | `-f` | 31504 | 0.578s | 0.066s | 8.8× | - | yes |
| go | `--type=cc` | 259 | 0.454s | 0.056s | 8.1× | - | yes |
| include | `literal` | 4590 | 0.282s | 0.032s | 8.7× | 0.068s | yes |
| include | `literal -i` | 4954 | 0.296s | 0.033s | 9.0× | 0.063s | yes |
| include | `no match` | 0 | 0.141s | 0.016s | 8.7× | 0.034s | yes |
| include | `alternation` | 708 | 0.262s | 0.024s | 10.9× | 0.064s | yes |
| include | `regex` | 260 | 0.195s | 0.039s | 4.9× | 0.063s | yes |
| include | `-w` | 138322 | 0.667s | 0.075s | 8.9× | 0.111s | yes |
| include | `-v -c` | 5111 | 0.434s | 0.045s | 9.7× | 0.051s | yes |
| include | `-l` | 479 | 0.151s | 0.015s | 10.3× | 0.033s | yes |
| include | `-c` | 5111 | 0.316s | 0.039s | 8.1× | 0.052s | yes |
| include | `-f` | 5111 | 0.111s | 0.012s | 9.3× | - | yes |
| include | `--type=cc` | 37605 | 0.452s | 0.053s | 8.5× | - | yes |
| share | `literal` | 13761 | 0.888s | 0.126s | 7.1× | 0.385s | yes |
| share | `literal -i` | 16722 | 1.018s | 0.148s | 6.9× | 0.422s | yes |
| share | `no match` | 0 | 0.478s | 0.060s | 8.0× | 0.257s | yes |
| share | `alternation` | 11443 | 0.994s | 0.126s | 7.9× | 0.371s | yes |
| share | `regex` | 4106 | 0.713s | 0.124s | 5.7× | 0.309s | yes |
| share | `-w` | 3493 | 1.144s | 0.165s | 6.9× | 0.371s | yes |
| share | `-v -c` | 15051 | 1.139s | 0.133s | 8.6× | 0.442s | yes |
| share | `-l` | 1223 | 0.533s | 0.067s | 7.9× | 0.256s | yes |
| share | `-c` | 15051 | 0.791s | 0.107s | 7.4× | 0.288s | yes |
| share | `-f` | 15051 | 0.395s | 0.042s | 9.5× | - | yes |
| share | `--type=cc` | 110 | 0.295s | 0.044s | 6.8× | - | yes |

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
