# Regex compatibility with Perl ack

Result of plan item 0.3. The raw numbers are in
[`regex-spike-results.md`](regex-spike-results.md), produced by
`scripts/regex-spike`.

## Decision (D1): PCRE2

rask matches with **PCRE2 10.46**, bundled with `pcre2-sys` 0.2.10 and
linked statically (`PCRE2_SYS_STATIC=1` in `.cargo/config.toml`). It runs in
**non-UTF mode** with JIT enabled. All matching goes through the
`matcher::Matcher` trait, so a faster engine can be added later (see
"Fast path" below).

## Evidence

We ran the whole ack3 test suite against Perl ack with every regex it built
logged. That gave 578 distinct patterns: search regexes, `-w`/`-i` wrappings,
`--and`/`--or`/`--not` combinations, highlight and pre-scan regexes, and
`match:` and `firstlinematch:` filter regexes. We ran each one over the 6192
lines of `t/` fixtures (356 of them not valid UTF-8) with Perl and each
candidate engine.

| Engine | Compile verdict = Perl | Matches = Perl (where both compile) |
|---|---:|---:|
| **pcre2** | **577 / 578** | **566 / 566** |
| fancy-regex | 577 / 578 | 485 / 566 |
| regex::bytes | 530 / 578 | 508 / 518 |

- **fancy-regex** only searches `&str`, so it can't search the non-UTF-8
  lines at all. In ack that means binary-ish and Latin-1 files. That rules
  it out (D2).
- **regex::bytes** can't compile the patterns ack itself generates. `-w`
  gives `(?:^|\b|\s)\K(?:…)(?=\s|\b|$)`, `--and` gives `(?=.*…)` lookaheads,
  and backreferences come from users. Its `$` also only matches at the
  very end, while Perl's `$` also matches before a final `\n`. That's why
  `foo$`, `pl$` etc. differ.
- **pcre2** agrees with Perl on every match of every pattern that both
  accept, including `m//g` iteration offsets. Perl's `(?^i:…)` syntax, which
  shows up when ack interpolates one `qr//` into another, works as-is.

## Things rask must handle itself

PCRE2 gives the right matches. What it doesn't give us is Perl's *error
behaviour*.

### 1. Perl warnings are errors in ack
`App::Ack::build_regex` compiles with `$SIG{__WARN__}` promoted to `die`
(`lib/App/Ack.pm:757`). So a pattern Perl merely *warns* about is rejected
by ack. The one PCRE2 divergence in the spike is this:
`foo{` ("Unescaped left brace in regex is passed through"), which PCRE2
accepts as a literal. Phase 1 needs a small lint pass for the regex
warnings Perl 5.22+ emits that ack users can hit. Start with unescaped `{`,
`\` followed by an unrecognised letter, and quantifiers on zero-length
assertions, and grow the list from the differential tests (Phase 6).

### 2. Error messages in Perl's format
`t/illegal-regex.t` pins the exact text:

```
ack: Invalid regex '(set|get)_user_(id|(username)'
Regex: (set|get)_user_(id|(username)
                      ^---HERE Unmatched ( in regex
```

The `^---HERE` column comes from Perl's `<-- HERE` marker. PCRE2 reports an
error code and an offset instead. Map the codes that the tests and common
typos produce to Perl's wording and marker position: `Unmatched (`,
`Unmatched )`, `Unmatched [`, `Quantifier follows nothing`. Fall back to
PCRE2's own text for the rest.

### 3. Code blocks must be refused
`(?{ … })` and `(??{ … })` must never run (`t/no-re-eval.t`, a security
fix). PCRE2 has no code blocks and rejects them as syntax errors (unit test
`rejects_code_blocks`). rask should detect them up front and use Perl's
"Eval-group not allowed at runtime" wording, so the output matches.

### 4. `m//g` after an empty match
After an empty match at position *p*, Perl retries at *p* and only accepts a
non-empty match there, before moving to *p+1*. The simple "advance one byte"
loop agreed with Perl on every pattern in the corpus. It would still differ on
patterns like `|a`. When highlighting and `-o` are implemented (Phase 4), do
it Perl's way: retry at *p* with `PCRE2_NOTEMPTY_ATSTART | PCRE2_ANCHORED`.
The high-level `pcre2` crate doesn't expose those flags, so we may need
`pcre2-sys` directly for this one call.

### 5. Byte semantics
Perl ack reads files without an encoding layer, so Perl matches bytes: `\w`,
`\d`, `\s` and case-insensitive matching are ASCII-only, and `.` matches any
byte except `\n`. PCRE2 in non-UTF mode without `PCRE2_UCP`, using its
built-in C-locale tables, does the same. Don't turn on UTF or UCP.

Patterns are bytes too. A pattern with `\x{100}` or higher is an error in
PCRE2's non-UTF mode, but Perl upgrades the string. Rare. Record it in
`divergences.md` if it comes up.

## Fast path (Phase 5)

`regex::bytes` was ~11× faster than PCRE2 on the spike's scan. Once the
full suite passes, add a second `Matcher` implementation that is used only
when the pattern:

- has no lookaround, `\K`, backreferences, possessive or atomic groups, or
  recursion (check with `regex-syntax` parsing, not string sniffing);
- has `$` handled by searching the line without its terminator, or by
  rewriting `$` as `(?:\n?\z)` (with `(?m)` handled separately);
- gives identical results to PCRE2 across the spike corpus. Keep the spike
  as a regression gate for this.
