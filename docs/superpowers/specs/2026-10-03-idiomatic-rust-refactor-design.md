# Idiomatic-Rust Refactor — Design Specification (0.8.5 → 0.9.0)

Status: approved design, 2026-10-03
Companion to: [`2026-10-01-jojodiff-1to1-port-spec.md`](2026-10-01-jojodiff-1to1-port-spec.md)
(the port spec remains the behavioral authority; this spec governs internal structure only)

## 1. Context and motivation

The 0.8.5 port is functionally complete and verified: 194 tests across four
layers (round-trip, C++-generated goldens, cross-version, live-oracle
compare), clippy-clean under `-D warnings`, CI on three OSes plus an
oracle-compare job. The port discipline (1:1 with upstream C++, per-function
line anchors, quirks ported as written) served verification — and is now the
remaining structural debt.

A refactoring audit (2026-10-03) scored the codebase **6.5/10** on the
refactoring-patterns quick diagnostic. The drivers:

| Smell | Evidence |
|---|---|
| Long Method | 31 functions ≥50 lines; worst: `real_main` 622 L, `JDiff::search` 428 L, `JMatchTable::add` 224 L |
| Untestable CLI | `real_main` calls `exit()` inline ×14; only subprocess-testable |
| Borrow-checker workaround | `JDiff::search` needs two field-disjoint `let Self {...} = self` destructures (jdiff.rs:551-562, 596-614) |
| No `Result` anywhere | 27 `-> i32` status APIs; `JFile::get` overloads byte/EOF/EOB/error sentinels in one channel |
| Duplication | char filter ×3, wire-length tier table ×4, output-open+exit ×3, verbatim list-merge ×2, near-identical `jpatch` opcode arms ×5 |

## 2. Ratified decisions

Four decisions were made interactively and bind this design:

1. **Fidelity: layered.** The CLI layer receives the full idiomatic
   treatment; the engine core receives conservative surgery only —
   de-duplication and state extraction with the i32 sentinel protocol and
   per-function C++ anchors intact.
2. **Dependencies: `thiserror` (lib) + `anyhow` (binary).** The std-only
   property is retired. `Getopt` stays hand-rolled: upstream's option
   grammar (lossy `c_atoi` parsing, stale usage text, argv[0] dispatch)
   cannot be expressed by clap without behavior drift.
3. **Version: 0.9.0** continuing the mirror lineage (0.8.x = port-parity
   era, 0.9.x = idiomatic-Rust era, 1.0.0 reserved for lib-API stability).
   The printed tool version `"0.8.5 (beta) 2020"` is a byte-pinned upstream
   constant and never moves.
4. **Sequencing: bottom-up.** Phase 1 de-dup → Phase 2 engine surgery →
   Phase 3 CLI idiomatic treatment → Phase 4 docs/version. Each phase
   shrinks the surface the next touches; the riskiest change (CLI) lands on
   a stabilized base.

## 3. Invariants (what never changes)

Every phase is behavior-preserving, pinned by the existing suites:

- Patch/listing/verbose **bytes**, CLI stdout/stderr **bytes**, and **exit
  codes** are identical. Goldens are never regenerated.
- The engine's i32 sentinel protocol (`EXI_*` codes, `EOF`/`EOB` from
  `JFile::get`) is untouched (layered ruling).
- `Getopt` behavior, `c_atoi` quirks, stale user-visible text (§21.10),
  dead code ported as dead (§21.13), stats quirks (§21.14), and every other
  §21 do-not-fix marker remain as-is.
- The write-error policy is oracle-pinned, not accidental: diff path
  silently ignores write errors (`jdiff A B /dev/full` → exit 0, matching
  the C++ unchecked `putc`s); patch path checks writes → `EXI_WRI`
  (exit 9). The refactor makes this explicit; it does not change it.
- Engine modules keep their upstream-mirroring names and ported identifier
  names.

## 4. Anchor policy

C++ traceability comments (`JDiff.cpp:389`) are load-bearing while the port
is the verification artifact. When code moves during extraction, its anchors
travel with it under a greppable tag convention:

```rust
// port:JDiff.cpp:389-718 — extracted into SearchState::run_scan
```

Quirk/do-not-fix comments (§21.13, §21.14, and the per-module quirk
censuses) are relocated verbatim, never deleted. Phase 4 documents the
convention in CONTRIBUTING.md.

## 5. Phase 1 — mechanical de-duplication (zero behavior risk)

Each item lands as its own green-to-green commit:

1. `defs::print_char(v: i32) -> char` — consolidates the printable-ASCII
   filter currently ×3 (jdebug.rs:107-113 `c_chr`, jout/asc.rs:59-65 `chr`,
   jpatcht.rs:144-149 inline).
2. `jout` shared wire-length tier module — the 252/508/65535/2^32 tier
   encoding is currently written ×4: writer `JOutBin::put_len`
   (jout/bin.rs:131-163), size calculators `JOutAsc::put_sze`
   (jout/asc.rs:71-83) and `JOutRgn::put_len` (jout/rgn.rs:75-87), decoder
   `JPatcht::uf_get_int` (jpatcht.rs:90-124). One module holds the tier
   constants and boundary math; JOutRgn's deliberate 4/8 return values
   (§21.14) remain a documented local deviation at its call site. Decoder
   and encoder now share one definition — closing the sync gap that only
   tests previously protected.
3. `open_output` helper — the identical output-open + error + `exit(-EXI_OUT)`
   blocks ×3 (bin/jdiff.rs:445-454, 658-667, 669-678).
4. `make_writer` helper — the `out_is_stdout && DBG_TO_STDOUT` raw-lock vs
   `BufWriter` decision ×2 (bin/jdiff.rs:498-503, 688-693).
5. `JMatchTable::merge_new_into_old` — the verbatim join-old-and-new-lists
   block (jmatchtable.rs:534-543 in `getbest`, 631-639 in `cleanup`);
   `nextold`'s third variant (1048-1052) is noted but unified only if it is
   the same decision, not coincidental similarity.
6. Equal-run fast loops in `JDiff::jdiff` (jdiff.rs:327-339 vs 342-351) —
   parameterize the one behavioral difference (`hash_add_org`).
7. `del_gld`/`del_col` (jmatchtable.rs:1207-1224, 1228-1243) — one
   parameterized unlink over the table/link pair.
8. `cli/opts.rs` short-cluster restructure — removes the `String` clone per
   short-option character (opts.rs:266, 295) via `mem::take` or
   index-based scanning.

Rule-of-Three discipline applies in reverse: nothing here is extracted
"for symmetry"; every item above is a repeated *decision* (audit §5).

## 6. Phase 2 — engine conservative surgery

1. **`SearchState` extraction (jdiff.rs).** The 13-field rolling search
   state (`az_org` … `hsh_err`) becomes a private sub-struct; the scan
   methods take `&mut SearchState` (+ `&mut dyn JFile` as needed). This
   ends the double `let Self {...} = self` destructure workaround in
   `search`. Field names unchanged (port discipline).
2. **`JFileAhead::get_fromfile` arm split (ahead.rs:412-549).** The
   Reset/Append/Scrollback strategy blocks become private functions; the
   three-way dispatch stays a readable match.
3. **`JMatchTable::add` (224 L)** splits into join/allocate/evaluate steps
   *only where anchor-clean* — if a step boundary would split a C++ line
   anchor's span, it stays inline. `cleanup`'s duplicated cfg(debug)
   sanity walks move behind `#[cfg(feature = "debug")]` helpers.
4. **`build_full_index` slow/fast loops (jdiff.rs:982-1011 vs 1014-1022)**
   are unified **only after verification** that they encode the same
   decision; if they differ subtly (a ported quirk), they stay separate
   with a comment saying why.
5. No protocol, signature-shape, or identifier renames in engine code.

## 7. Phase 3 — CLI idiomatic treatment

### 7.1 Module layout

```
src/bin/jdiff.rs    ~60 lines: argv → cli::run(argv) → exit-code + stderr
src/cli/
  mod.rs            module re-exports (exists; gains the new items)
  run.rs            orchestrator (real_main's control flow, no exits)
  config.rs         pure option/buffer-sizing math
  diff_phase.rs     run_diff_phase(inputs, config) -> i32
  patch_phase.rs    run_patch_phase (also the -t test mode)
  report.rs         greeting/usage/notes/echo/statistics text fns
  error.rs          CliError + the single error→(code, stderr) boundary
  opts.rs           Getopt (moved logic lands around it)
```

`Function` (Diff/Patch/Dedup/Test) moves from the binary into `cli`.
`print_usage`/`print_notes` stay byte-pinned text blocks, relocated to
`report.rs` with their anchors.

### 7.2 Error model

- `#[derive(Debug, thiserror::Error)] pub enum CliError` — variants carry
  `std::io::Error` plus which-file context (first/second/output/patch).
- Engine return codes stay `i32` (layered ruling); the CLI boundary wraps
  them. One function maps `Result<i32, CliError>` → process exit code +
  byte-pinned stderr text — the single place error text lives.
- All 19 inline `exit()` sites in the current binary become returned codes
  or `CliError`; the library's cli module never exits the process.
- `anyhow` is used in the binary wrapper only, for context around
  `cli::run`; no `anyhow` types cross into the lib.

### 7.3 Write-error policy (explicit)

`IgnoringWriter` is retained and renamed/documented as the diff-path
policy (oracle: `/dev/full` → exit 0). The patch path's checked writes →
`EXI_WRI` (oracle: exit 9). The JOut panicking write helpers become
infallible-by-policy with a policy comment citing the oracle rows — not
removed.

### 7.4 New unit tests

As phases become in-process callable, unit tests land for: argv[0]
dispatch, option parsing edge cases beyond Getopt's existing tests,
buffer-sizing math, each phase function's return codes, and the
error→(code, stderr) mapping. These are additions; the subprocess
round-trip suite remains the byte-parity authority.

## 8. Phase 4 — docs, versioning, polish

- `version = "0.9.0"` + README versioning-policy paragraph (mirror
  lineage; printed version constant unaffected).
- lib.rs and README drop the "std-only" claim; state the two dependencies
  and why.
- CONTRIBUTING.md: the `// port:` anchor convention, the do-not-fix
  pointer to port spec §21, and the green-to-green refactoring rule.
- `//!` module docs on all new cli modules; `///` on new public items
  (Errors/Panics sections where relevant).
- MSRV stays 1.85 (thiserror/anyhow both support it; CI msrv job guards).

## 9. Verification and safety rails

- Per-commit gate: `cargo fmt --all --check`,
  `cargo clippy --all-targets --all-features --locked -- -D warnings`,
  `cargo test --all-features` — all green before and after every
  transformation; one transformation per commit (refactoring-patterns
  green-to-green discipline).
- Goldens byte-compare on every run; the CI oracle job (ubuntu) guards
  live byte-parity; crossver guards 0.8.1 patch applicability.
- **Bounded loops only** in any test writing under temp dirs: this
  project's history includes a runaway `jdiff -u` test loop filling the
  7.8 GB tmpfs `/tmp` and killing the session via disk-quota cascade. No
  unbounded retries; every generated artifact has a size cap.
- Branch `refactor/idiomatic-rust` off main; task-granular conventional
  commits (`refactor:`, `test:`, `docs:`); merge to main when fully green.

## 10. Acceptance criteria

1. Full suite green (194 existing tests + new CLI unit tests), CI matrix
   including the oracle job.
2. Goldens and all byte-parity checks unmodified and passing — no golden
   file changes in the diff.
3. `src/bin/jdiff.rs` ≤ 80 lines, no `exit()` outside `main`; no
   `process::exit` in library code except existing cfg(debug) oracle-parity
   asserts.
4. `JDiff::search` contains zero `let Self {...} = self` destructures.
5. The audit's duplication inventory (§5 above) fully consolidated.
6. clippy clean at `-D warnings`; rustfmt clean.
7. Re-scored quick diagnostic ≥ 8/10 (methods <10 lines and duplication
   rows are the expected movers; the i32 sentinel row stays by design).
8. Version 0.9.0, docs updated, anchors greppable via `// port:`.

## 11. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Subtle engine behavior drift on extraction | Goldens + oracle job pin bytes; anchors travel; quirk comments relocate verbatim |
| Extraction splits a C++ anchor span | Step boundaries yield to anchor spans (Phase 2 rule 3) |
| CLI text drift (usage/greeting/errors) | Byte-pinned blocks move whole into `report.rs`; round-trip suite asserts exact bytes |
| Wrong abstraction from over-dedup | Rule-of-Three in reverse: only audit-listed same-decision duplicates merge |
| tmpfs fill from test loops | §9 bounded-loops constraint carried into the implementation plan |

## 12. Out of scope

- Any behavioral change, including fixing upstream bugs already deviated
  from per §21 (the negative-position EOF gate stays).
- `ByteOrEof`/`Op` enum type overhaul of the engine (deferred; would
  revisit at 1.0 planning).
- Replacing `Getopt` with clap; new features; performance work beyond
  removing the audited redundant clones.
