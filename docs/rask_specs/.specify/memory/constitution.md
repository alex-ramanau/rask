<!--
Sync Impact Report
==================
Version change: (template, unversioned) → 1.0.0
Bump rationale: initial ratification; every placeholder replaced with project-specific content.

Modified principles (template slot → new title):
  - [PRINCIPLE_1_NAME] → I. Byte-for-Byte Parity with ack 3.10.0 (NON-NEGOTIABLE)
  - [PRINCIPLE_2_NAME] → II. Perl ack Is the Oracle: Conformance Testing
  - [PRINCIPLE_3_NAME] → III. Speed Without Output Changes
  - [PRINCIPLE_4_NAME] → IV. Safe, Self-Contained Rust
  - [PRINCIPLE_5_NAME] → V. Faithful Attribution and Licensing

Added sections:
  - Platform and Distribution Constraints (was [SECTION_2_NAME])
  - Development Workflow and Quality Gates (was [SECTION_3_NAME])
  - Governance (filled in)

Removed sections: none

Follow-up TODOs: none. Ratification date set to the date of first adoption (2026-10-12).
-->

# rask Constitution

## Core Principles

### I. Byte-for-Byte Parity with ack 3.10.0 (NON-NEGOTIABLE)

Perl ack 3.10.0 is the specification. For any command line, environment and file tree, rask
MUST produce the same stdout, the same stderr and the same exit code as Perl ack, and MUST
read the same `.ackrc` files and `ACK_*` environment variables with the same precedence.

- The only permitted differences are those listed under "Differences from ack" in `README.md`
  (currently: the `--version` string, and PCRE2 in place of Perl's regex engine, with Perl's
  behaviour and diagnostics emulated as recorded in `docs/regex-compat.md`).
- A new intentional difference MUST be justified in its spec or plan, added to that README
  section, and covered by a test that pins the new behaviour.
- rask MUST NOT add options, output formats or defaults that ack does not have.
- Parallelism MUST be invisible: output order, warnings and their position, headings,
  separators, `-1` and `-m` MUST match a serial run and Perl ack.

Rationale: rask's whole value is "if you know ack, you already know rask". Any divergence,
however small, breaks scripts and the user's trust.

### II. Perl ack Is the Oracle: Conformance Testing

Behaviour is verified against Perl ack itself, not against our reading of its documentation.

- The ack3 test suite (`scripts/ack-suite`) MUST stay fully green on Linux; a change that
  lowers the pass count MUST NOT be merged.
- Every change to observable behaviour MUST add or update differential cases under
  `tests/diff/` (run by `scripts/diff-ack`), and those cases MUST pass both in parallel and
  with `RASK_THREADS=1`.
- ack3 tests that exercise only Perl internals are listed in `rust-skip.txt`, and each MUST
  have a Rust counterpart (in `tests/` or unit tests) or an explicit "n/a" justification.
- Bug fixes MUST start with a failing test or differential case that reproduces the
  disagreement with Perl ack.

Rationale: an executable oracle catches the edge cases (encodings, binary detection, regex
diagnostics, config precedence) that prose specifications miss.

### III. Speed Without Output Changes

Speed is the reason rask exists, and it MUST never be bought with a change in output.

- Every performance change MUST keep the ack3 suite, the differential cases and the output
  check in `scripts/bench` passing.
- Performance claims (README, `docs/benchmarks.md`) MUST come from `scripts/bench` runs whose
  method, machine and date are recorded; numbers MUST NOT be quoted from memory or estimated.
- A change that measurably slows a benchmark scenario MUST state the cost and the reason in
  its PR; optimizations MUST record what they changed and their measured effect in
  `docs/performance_optimizations.md`.
- Memory use of parallel search MUST stay bounded (the walker may only run a fixed number of
  entries ahead of printing).

Rationale: "8× faster with identical output" is the product promise; both halves are tested.

### IV. Safe, Self-Contained Rust

- `unsafe_code` is forbidden crate-wide; this lint MUST NOT be relaxed. Platform calls go
  through safe wrappers (`rustix`).
- rask MUST ship as one self-contained binary with no runtime dependency on Perl, CPAN or
  other interpreters. Perl is a test-time oracle only.
- New crate dependencies MUST be justified in the plan (what it does, why the standard library
  or an existing dependency is insufficient); the dependency set stays minimal.
- Code MUST pass `cargo fmt --check` (`max_width = 100`) and `cargo clippy --all-targets
  -- -D warnings`, and MUST build on the declared MSRV (Rust 1.85, edition 2024).
- Modules mirror the Perl components they reimplement (see the mapping in
  `docs/development.md`); new code MUST name the ack/Perl behaviour it ports when the mapping
  is not obvious.

Rationale: a memory-safe, dependency-light single binary is easy to trust, audit and package.

### V. Faithful Attribution and Licensing

- rask is licensed GPL-3.0-or-later. New files and dependencies MUST be compatible with it.
- Text embedded from ack (default ackrc, `--help`, `--man`, easter eggs) MUST be regenerated
  with `scripts/sync-from-ack3`, MUST NOT be hand-edited, and keeps Andy Lester's copyright.
- Credits to ack, PCRE2 and the `pcre2` crate MUST be kept in the README.

Rationale: rask exists because of ack's design; honouring its author and licence is a
precondition of distributing it.

## Platform and Distribution Constraints

- **Linux is the reference platform.** Full conformance (Principles I–II) is required on
  Linux. macOS and Windows MUST build and pass `cargo test --all` in CI; the ack3 suite runs
  there and reports pass counts, and regressions on those platforms MUST be investigated
  before merge.
- **Release builds** MUST use `--locked`. The snap ships a static musl binary (with mimalloc)
  for amd64 and arm64.
- **The snap is strictly confined.** Any change to plugs, interfaces or confinement MUST update
  `snap/snapcraft.yaml`, `docs/snap.md` and the README's install section together, and MUST
  state what the user can and cannot search as a result.
- The upstream harness change for ack3 (`ci/ack3-test-binary.patch`) MUST be kept applicable
  to the ack3 ref pinned in CI until it is merged upstream.

## Development Workflow and Quality Gates

- **Local gate:** `make check` (fmt, clippy, tests) MUST pass before a change is proposed;
  changes to behaviour also run `scripts/ack-suite` and the relevant `scripts/diff-ack` cases
  against a local ack3 checkout (`ACK3_DIR`).
- **CI gate:** the `cargo` and `ack3 suite` jobs in `.github/workflows/ci.yml` MUST be green on
  every pull request to `main`; the snap workflow MUST be green for changes under `snap/`.
- **Documentation:** a change MUST update the user docs (`README.md`) when user-visible, and
  `docs/development.md` when layout, scripts or the build process change.
- **Spec Kit artifacts:** each feature's `plan.md` MUST include a Constitution Check against
  Principles I–V; any violation MUST be recorded with its justification in the plan's
  Complexity Tracking table, or the design changed.
- **Environment note:** on VirtualBox shared folders, `CARGO_TARGET_DIR` MUST point to a local
  disk; scripts and the Makefile honour it.

## Governance

This constitution supersedes other development practices for rask. Where it conflicts with
another document, the constitution wins until it is amended.

- **Amendments** are made by a pull request that edits `.specify/memory/constitution.md`,
  explains the change and its motivation, and updates any dependent docs in the same PR. A
  temporary Sync Impact Report is used during review and removed before merge.
- **Versioning** follows semantic versioning: MAJOR for removing or redefining a principle
  incompatibly, MINOR for a new principle or section or materially expanded guidance, PATCH for
  clarifications and wording.
- **Compliance review:** every PR review and every Spec Kit plan MUST check compliance with
  this constitution; deviations require explicit justification. Runtime development guidance
  lives in `docs/development.md`.

**Version**: 1.0.0 | **Ratified**: 2026-10-12 | **Last Amended**: 2026-10-12
