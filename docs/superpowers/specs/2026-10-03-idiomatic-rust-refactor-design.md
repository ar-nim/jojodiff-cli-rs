# Idiomatic-Rust Refactor — Design Specification (0.8.5 → 0.9.0)

Status: approved design, revised 2026-10-03 (fidelity upgraded from
"layered" to "byte-contract only" by user ruling mid-design)
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

1. **Fidelity: byte-contract only.** The observable behavior — patch,
   listing and verbose **bytes**, CLI stdout/stderr **bytes**, **exit
   codes** — is the sole contract, pinned by the golden/oracle suites.
   Internals become freely idiomatic Rust wherever an idiomatic form can
   still pass every pinned test. The C++ implementation is reference
   material for *behavior*, not a template for *structure*. (Amended
   2026-10-03 from the original "layered" ruling; all breaking internal
   changes land in the single 0.9.0 release.)
2. **Dependencies: `thiserror` (lib, engine-wide) + `anyhow` (binary) +
   test trio (`tempfile`, `assert_cmd`, `predicates`) as dev-deps.** The
   std-only property is retired. `Getopt` stays hand-rolled — no arg-parsing
   crate can tokenize the pinned grammar (lossy `c_atoi`, byte-exact
   errors, argv[0] dispatch). See §13 for the full decision record.
3. **Version: 0.9.0** continuing the mirror lineage (0.8.x = port-parity
   era, 0.9.x = idiomatic-Rust era, 1.0.0 reserved for lib-API stability).
   The printed tool version `"0.8.5 (beta) 2020"` is a byte-pinned upstream
   constant and never moves.
4. **Sequencing: bottom-up, five phases.** P1 de-dup → P2 engine surgery →
   P3 engine type modernization → P4 CLI idiomatic treatment → P5
   docs/version. Each phase shrinks the surface the next touches; all
   breaking changes land once, in 0.9.0.

## 3. Invariants (what never changes)

Every phase is behavior-preserving, pinned by the existing suites:

- Patch/listing/verbose **bytes**, CLI stdout/stderr **bytes**, and **exit
  codes** are identical. Goldens are never regenerated.
- The port spec §21 rulings remain the behavioral authority, including the
  do-not-fix markers (§21.3 broken `-t`, §21.4 `-y` → exit 20, §21.10 stale
  text replicated verbatim, §21.14 stats quirks, §21.17 negative-position
  EOF gate, §21.16 `--compat-081`). Behavior follows the code, text
  follows the text.
- `Getopt` parsing behavior and `c_atoi` semantics are pinned (they parse
  the CLI contract).
- The write-error policy is oracle-pinned, not accidental: diff path
  silently ignores write errors (`jdiff A B /dev/full` → exit 0, matching
  the C++ unchecked `putc`s); patch path checks writes → `EXI_WRI`
  (exit 9). The refactor makes this explicit; it does not change it.
- **No longer invariant** (revoked by the byte-contract ruling): the
  engine's internal i32 sentinel protocol. It is modernized in Phase 3;
  the `EXI_*` constants survive as the exit-code vocabulary at the CLI
  boundary.

## 4. Anchor policy

C++ traceability comments (`JDiff.cpp:389`) are **verification metadata,
not a constraint**. They travel with code that moves when convenient, and
new idiomatic code does not owe them anything. Quirk/do-not-fix comments
(§21 markers, per-module quirk censuses) are behavioral documentation and
are relocated verbatim, never deleted. Phase 5 documents the `// port:`
tag convention in CONTRIBUTING.md as optional archaeology.

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
   (§21.14) remain a documented local deviation at its call site.
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
9. **Dead-code removal where provably unreachable** (permitted by the
   byte-contract ruling; §21.13 remains the inventory): `get_buf_sze`
   (zero non-test callers), the COLLISION_LOW branch, `MAXGLD`. Each
   removal requires a reachability argument in the commit message.

Rule-of-Three discipline applies in reverse: nothing here is extracted
"for symmetry"; every item above is a repeated *decision* (audit §5).

## 6. Phase 2 — engine structural surgery

1. **`SearchState` extraction (jdiff.rs).** The 13-field rolling search
   state (`az_org` … `hsh_err`) becomes a private sub-struct; the scan
   methods take `&mut SearchState` (+ `&mut dyn JFile` as needed). This
   ends the double `let Self {...} = self` destructure workaround in
   `search`. Field names unchanged.
2. **`JFileAhead::get_fromfile` arm split (ahead.rs:412-549).** The
   Reset/Append/Scrollback strategy blocks become private functions; the
   three-way dispatch stays a readable match.
3. **`JMatchTable::add` (224 L)** splits into join/allocate/evaluate steps
   where the boundaries are clean. `cleanup`'s duplicated cfg(debug)
   sanity walks move behind `#[cfg(feature = "debug")]` helpers.
4. **`build_full_index` slow/fast loops (jdiff.rs:982-1011 vs 1014-1022)**
   are unified **only after verification** that they encode the same
   decision; if they differ subtly (a ported quirk), they stay separate
   with a comment saying why.

## 7. Phase 3 — engine type modernization

The highest-leverage idiomatic change, enabled by the byte-contract ruling:

1. **`ByteOrEof` for `JFile::get`.** The i32 sentinel channel (0-255 /
   `EOF` / `EOB` / `EXI_SEK` / `EXI_RED`) becomes an enum —
   `Byte(u8) / Eof / Eob / Err(JDiffError)`. Every `< 0` / `<= EOF` /
   `< EOB` comparison site becomes a match. The §21.17 negative-position
   gate must survive the reshape as `Eof` (it is pinned by unit tests).
2. **`Op` opcode enum.** `ESC/MOD/INS/DEL/EQL/BKT` i32 consts become a
   `#[repr(u8)]` enum (byte values 0xA2-0xA7 stay the wire truth); the 23
   `as u8` casts in `JOutBin` and friends disappear. The decoder keeps
   accepting arbitrary bytes as "not an opcode" exactly as today.
3. **`Result` engine-wide.** The 27 `-> i32` status APIs become
   `Result<T, JDiffError>`; the thiserror enum is defined once in the lib
   (engine + CLI phases share it). The five pinned `eprintln!` error texts
   move into the error's Display, printed at the CLI boundary — byte
   order/content verified against the oracle goldens. cfg(debug)
   `process::exit` parity asserts (ahead.rs:391,403) are re-expressed as
   debug asserts or error returns with identical observable behavior.
4. **`Node.cmp` sentinel field** becomes an enum with payload
   (`Run(i32)` / the CMPINV/CMPSKP/CMPEOB sentinels / the negated EOB
   estimates `is_best` stores), making `is_old2_skip`/`is_old2_reuse`
   self-documenting. The negated-estimate encoding trick must be
   represented faithfully — it is algorithm, not accident.

## 8. Phase 4 — CLI idiomatic treatment

### 8.1 Module layout

```
src/bin/jdiff.rs    ≤80 lines: argv → cli::run(argv) → exit-code + stderr
src/cli/
  mod.rs            module re-exports (exists; gains the new items)
  run.rs            orchestrator (real_main's control flow, no exits)
  config.rs         pure option/buffer-sizing math
  diff_phase.rs     run_diff_phase(inputs, config) -> Result<i32, JDiffError>
  patch_phase.rs    run_patch_phase (also the -t test mode)
  report.rs         greeting/usage/notes/echo/statistics text fns
  error.rs          JDiffError boundary: error → (exit code, stderr bytes)
  opts.rs           Getopt (behavior pinned; internals may modernize)
```

`Function` (Diff/Patch/Dedup/Test) moves from the binary into `cli`.
`print_usage`/`print_notes` stay byte-pinned text blocks, relocated to
`report.rs`.

### 8.2 Error model

- Dependencies (verified against crates.io / Context7, 2026-10-03):
  `thiserror = "2"` (latest 2.0.21) and `anyhow = "1"` (latest 1.0.104);
  both MSRVs sit well below the crate's 1.85 floor.
- One lib-wide `#[derive(Debug, thiserror::Error)] pub enum JDiffError`
  (Phase 3 introduces it engine-side; Phase 4 the CLI phases use it):
  variants carry `std::io::Error` via `#[from]` plus which-file context,
  with `#[error("...")]` Display messages that reproduce the pinned texts.
- One boundary function maps `Result<i32, JDiffError>` → process exit
  code + byte-pinned stderr — the single place error text is printed.
- All 19 inline `exit()` sites become returned codes or `JDiffError`; the
  library never exits the process.
- `anyhow` is used in the binary wrapper only, for context around
  `cli::run`; no `anyhow` types cross into the lib.

### 8.3 Write-error policy (explicit)

`IgnoringWriter` is retained and documented as the diff-path policy
(oracle: `/dev/full` → exit 0). The patch path's checked writes → `EXI_WRI`
(oracle: exit 9). With Phase 3's `Result` plumbing, the JOut panicking
write helpers become properly checked-or-explicitly-ignored with a policy
comment citing the oracle rows.

### 8.4 Test modernization (dev-deps adopted)

- `tempfile` (3.x): replaces the hand-rolled `temp_dir`/`DirGuard` helpers
  (193 pattern matches across 5 test files, two competing conventions).
  RAII auto-delete structurally mitigates the tmpfs-leak failure mode.
- `assert_cmd` (2.x) + `predicates` (3.x): simplify the 13 `Command::new`
  sites — cargo-bin resolution, stdin writing, output assertions.
- Kept hand-rolled where behavior demands it: seekable-stdin tests keep
  `Stdio::from(File)` (a real file is the behavior under test); argv[0]
  dispatch tests keep the copy-binary-under-new-name step (no crate does
  this); the francisdb `jojodiff` cross-validation dev-dep stays.
- New unit tests land for: argv[0] dispatch, buffer-sizing math, each
  phase function, and the error→(code, stderr) mapping. The subprocess
  round-trip suite remains the byte-parity authority.

## 9. Phase 5 — docs, versioning, polish

- `version = "0.9.0"` + README versioning-policy paragraph (mirror
  lineage; printed version constant unaffected).
- lib.rs and README drop the "std-only" claim; document the dependency
  set and rationale (§13).
- CONTRIBUTING.md: the `// port:` anchor convention (optional metadata),
  the do-not-fix pointer to port spec §21, and the green-to-green
  refactoring rule.
- `//!` module docs on all new/changed modules; `///` on public items.
- MSRV stays 1.85 (all deps support it; dev-dep MSRVs cannot regress the
  gate because the CI msrv job runs plain `cargo check`).

## 10. Verification and safety rails

- Per-commit gate: `cargo fmt --all --check`,
  `cargo clippy --all-targets --all-features --locked -- -D warnings`,
  `cargo test --all-features` — all green before and after every
  transformation; one transformation per commit.
- Goldens byte-compare on every run; the CI oracle job guards live
  byte-parity; crossver guards 0.8.1 patch applicability.
- **Bounded loops only** in any test writing under temp dirs: this
  project's history includes a runaway `jdiff -u` test loop filling the
  7.8 GB tmpfs `/tmp` and killing the session via disk-quota cascade. No
  unbounded retries; every generated artifact has a size cap; `tempfile`'s
  drop-cleanup is the default, never `keep()`.
- Branch `refactor/idiomatic-rust` off main; task-granular conventional
  commits; merge to main when fully green.

## 11. Acceptance criteria

1. Full suite green (194 existing tests + new unit tests), CI matrix
   including the oracle job.
2. Goldens and all byte-parity checks unmodified and passing — no golden
   file changes in the diff.
3. `src/bin/jdiff.rs` ≤ 80 lines, no `exit()` outside `main`; no
   `process::exit` in library code.
4. `JDiff::search` contains zero `let Self {...} = self` destructures.
5. The audit's duplication inventory (§5) fully consolidated; §5.9 dead
   code removed with reachability arguments.
6. `JFile::get` returns `ByteOrEof`; opcode values flow as `Op` through
   writers and decoder (zero opcode `as u8` casts); engine public APIs are
   `Result`-based.
7. clippy clean at `-D warnings`; rustfmt clean.
8. Re-scored quick diagnostic ≥ 8/10.
9. Version 0.9.0, docs updated, dependency record (§13) reflected in
   README.

## 12. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Subtle engine behavior drift on extraction/re-typing | Goldens + oracle job pin bytes; §21 quirk markers relocate verbatim |
| Sentinel-reshape loses a pinned edge (EOB soft-ahead-only, §21.17 EOF gate, `is_best` negated estimates) | Each has a pinning test today; the enum arms are named after the sentinels so mismatches fail loudly |
| CLI text drift (usage/greeting/errors) | Byte-pinned blocks move whole into `report.rs`; round-trip suite asserts exact bytes |
| Wrong abstraction from over-dedup | Only audit-listed same-decision duplicates merge |
| Dead-code removal that was not actually dead | Reachability argument required per item (§5.9); full-suite gate |
| tmpfs fill from test loops | §10 bounded-loops constraint; tempfile RAII default |

## 13. Dependency decision record

Adopted:

| Crate | Role | Why |
|---|---|---|
| `thiserror = "2"` | lib, engine-wide error enum | Idiomatic typed errors; replaces 27 `-> i32` APIs |
| `anyhow = "1"` | binary wrapper only | Ergonomic context at the top level |
| `tempfile = "3"` | dev | Replaces 193 hand-rolled temp-dir sites; RAII cleanup |
| `assert_cmd = "2"`, `predicates = "3"` | dev | Standard CLI-test ergonomics for 13 subprocess sites |
| `proptest = "1"` (optional, post-refactor) | dev | Property-based round-trip fuzzing, size-capped |

Considered and rejected:

| Candidate | Ruling |
|---|---|
| `log` / `tracing` | Every diagnostic byte is golden-pinned (`.vv.stderr` files, debug oracle); a framework needs a custom formatter that writes the same hand-written strings — DRY-negative. Verbosity gates code paths, not levels. |
| `clap` / `pico-args` / any parser | Cannot express the pinned grammar (lossy `c_atoi`, byte-exact errors, argv[0] dispatch). Getopt internals may modernize; its behavior cannot. |
| `primal` | ~30 pinned, tested lines vs a dependency — cost/benefit fails. |
| `memmap2` | Perf change with pipe-semantics implications; excluded from this refactor. |
| `insta` | Binary goldens + exact stderr compare well with `fs::read` + `pretty_assertions`. |
| `exitcode`/`sysexits`, `serde`, bitflags | Exit codes upstream-pinned; no config surface; flag arrays are behavior. |

## 14. Out of scope

- Any behavioral change, including fixing upstream bugs already deviated
  from per §21 (the negative-position EOF gate stays).
- Replacing `Getopt` behavior; new features; performance work beyond
  removing the audited redundant clones.
- Fuzzing infrastructure beyond the optional `proptest` follow-up.
