# Refactor Phase 5 — Docs, Versioning, Polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the refactor: 0.9.0 with a documented versioning policy, docs matching the new dependency set and structure, rustdoc coverage on the new API, and the spec's acceptance checklist verified end to end.

**Architecture:** Documentation-and-release tasks only — no behavior, no structural code change (a rustdoc-comment sweep is the only src/ touch).

**Tech Stack:** Markdown docs + Cargo metadata; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` §9 (this phase), §11 (acceptance), §13 (dependency record).

## Global Constraints

- Same core rules as the Phase 1 plan (worktree, branch, gate, conventional commits). Documentation claims must match verified reality — every factual sentence added to README/lib docs is checkable against the tree.
- The printed tool version stays `"0.8.5 (beta) 2020"` (`JDIFF_VERSION`, byte-pinned); only `Cargo.toml`'s `version` moves to `0.9.0`.

---

### Task 1: Version 0.9.0 + README updates

**Files:**
- Modify: `Cargo.toml` (`version = "0.9.0"`)
- Modify: `README.md`

**Interfaces:** none.

- [ ] **Step 1: Bump the version**

`version = "0.8.5"` → `version = "0.9.0"` in `Cargo.toml`. Run `cargo check` to refresh `Cargo.lock`'s own-package version.

- [ ] **Step 2: README — versioning policy paragraph**

Add near the top (after the project description):

```markdown
## Versioning

The crate version tracks the **upstream compatibility lineage**, not this
project's age: `0.8.x` mirrors JojoDiff 0.8.5 byte-for-byte (the port-parity
era), and `0.9.x` is the same behavior on an idiomatic-Rust internals
refactor. `1.0.0` is reserved for a stable library API. The version the
tool prints (`0.8.5 (beta) 2020`) is JojoDiff's own banner, replicated
byte-exactly and never bumped by this project.
```

- [ ] **Step 3: README — dependency story**

Replace every "std-only"/"no dependencies" claim with the dependency record (spec §13): runtime `thiserror` (typed engine errors) + `anyhow` (binary wrapper only); dev `pretty_assertions`, `jojodiff` (cross-validation oracle), `tempfile`/`assert_cmd`/`predicates` (test harness). One short table or list with the why, linking the spec's §13 for the considered-and-rejected record (clap, log/tracing, primal, memmap2, insta).

- [ ] **Step 4: Full gate + commit**

```bash
git add Cargo.toml Cargo.lock README.md
git commit -m "docs: 0.9.0 with documented mirror-lineage versioning and dependency record"
```

---

### Task 2: Crate docs, CONTRIBUTING, and the spec carve-out

**Files:**
- Modify: `src/lib.rs` (crate `//!` docs)
- Modify: `CONTRIBUTING.md`
- Modify: `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` (§11 criterion 3)

**Interfaces:** none.

- [ ] **Step 1: lib.rs crate docs**

Update the `//!` block: module map (engine + `cli` + `error`), the error model in two sentences (`Result<_, JDiffError>` engine-wide; `cli::error::report` is the single text/exit-code boundary), and the dependency sentence from Task 1. Keep the byte-compatibility promise sentence unchanged.

- [ ] **Step 2: CONTRIBUTING**

Add three short sections:

```markdown
## Port anchors

`// port:<File>.cpp:<lines>` comments are verification metadata tying code
to the C++ source the goldens were generated from. They are optional —
new idiomatic code owes nothing — but when ported code moves, its anchors
travel with it (greppable via `rg '// port:'`).

## Do-not-fix markers

Behavioral quirks replicated on purpose are inventoried in the port spec
(`docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` §21). If a
piece of code looks wrong, check §21 before "fixing" it — the goldens will
fail otherwise, by design.

## Refactoring rule

Green-to-green only: the full gate (`cargo fmt --all && cargo clippy
--all-targets --all-features --locked -- -D warnings && cargo test
--all-features`) passes before and after every transformation, one
transformation per commit. Goldens are never regenerated.
```

- [ ] **Step 3: Spec carve-out amendment**

In the refactor spec's §11, criterion 3, append to "no `process::exit` in library code": "— except the two `cfg(debug)` oracle-parity asserts in `src/jfile/ahead.rs` (debug-build exit 6 is pinned by the debug oracle tests; a `Result` cannot reproduce it)". This records the Phase 3 ruling in the acceptance list itself.

- [ ] **Step 4: Full gate + commit**

```bash
git add src/lib.rs CONTRIBUTING.md docs/superpowers/specs/
git commit -m "docs: crate docs, contributing guide, spec acceptance carve-out"
```

---

### Task 3: Rustdoc sweep on the new surface

**Files:**
- Modify: `src/cli/*.rs`, `src/error.rs`, `src/jout/wire.rs` (doc comments), `src/jfile/mod.rs` (`ByteOrEof`)

**Interfaces:** none (comments only).

- [ ] **Step 1: Ensure every public item carries `///`**

Checklist (each item gets purpose + one usage line; `# Errors` sections on `Result`-returning public fns): `JDiffError` variants + `exit_code`, `ByteOrEof` variants, `cli::run`, `cli::config::{Function, Options, parse, size_buffers, Buffers, function_from_argv0}`, `cli::error::report`, `cli::report::{print_greeting, print_usage, print_notes}`. Module `//!` docs on `src/cli/mod.rs` (receiving the four CLI-deviation notes moved from the binary in Phase 4 Task 5), `src/cli/config.rs`, `src/cli/error.rs`, `src/error.rs`. Run `cargo doc --no-deps` — zero warnings; fix any broken intra-doc links it reports.

- [ ] **Step 2: Full gate + commit**

```bash
git add src/
git commit -m "docs: rustdoc coverage on the 0.9.0 public surface"
```

---

### Task 4: Acceptance verification + release notes

**Files:**
- Create: `docs/superpowers/notes/2026-10-03-0.9.0-release-notes.md` (release-notes draft)

**Interfaces:** none.

- [ ] **Step 1: Run the spec §11 acceptance checklist (all nine items)**

1. `cargo test --all-features` green (grew beyond 204 — count and record).
2. `git diff --stat main..HEAD -- tests/fixtures/` → empty.
3. `wc -l src/bin/jdiff.rs` ≤ 80; `rg -n 'exit\(' src/bin/jdiff.rs src/` shows only `main`'s final exit + the two `cfg(debug)` parity asserts.
4. `rg -n 'let Self \{' src/jdiff.rs` → zero destructures.
5. The Phase 1 spot-check greps (plan P1 Task 10 Step 2) all clean.
6. `rg -n 'fn get\(' src/jfile/mod.rs` returns the `ByteOrEof` signature; opcode casts: `rg -n 'as u8' src/jout/ src/jpatcht.rs` shows only wire-byte conversions not opcode casts; `rg -n '\-> i32' src/jdiff.rs src/jpatcht.rs src/jfileout.rs` shows accessors only.
7. fmt + clippy clean (part of every gate).
8. Quick-diagnostic re-score per the refactoring-patterns rubric (target ≥ 8/10; the i32-sentinel row stays by design — note it).
9. Version/docs state from Tasks 1-3.

- [ ] **Step 2: Write the release-notes draft**

```markdown
# jojodiff-cli-rs 0.9.0

Same JojoDiff 0.8.5 behavior — byte-identical patches, listings, verbose
output and exit codes — on an idiomatic-Rust internals refactor.

- **Breaking (library API):** `JFile::get` returns `ByteOrEof`; engine
  APIs return `Result<_, JDiffError>`; opcodes are the `Op` enum; the CLI
  lives in the `cli` module with `cli::run` as the testable entry point.
- **Dependencies:** `thiserror` + `anyhow` replace the zero-dependency
  property (documented decision record in the refactor spec §13).
- **Tests:** in-process unit tests for parsing/sizing/dispatch/error
  boundary; `tempfile`/`assert_cmd` harness with RAII scratch dirs.
- CLI behavior, the printed version banner, and goldens are unchanged;
  `--compat-081` patches and 0.8.1-era applicability are unaffected.
```

- [ ] **Step 3: Full gate + commit**

```bash
git add docs/superpowers/notes/
git commit -m "docs: 0.9.0 acceptance verification and release-notes draft"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** §9 bullet 1 → Task 1; bullets 2-3 → Tasks 1-2; bullet 4 → Task 3; §11 → Task 4. The Phase 3 cfg(debug) ruling is closed out by Task 2 Step 3.
- **Placeholder scan:** none — every step is a concrete edit or verification command with expected outcomes.
- **Type consistency:** references (`cli::run`, `JDiffError::exit_code`, `ByteOrEof`) match the names fixed by Phases 3-4.
- **Ordering note:** Task 4 legitimately runs last; Tasks 1-3 are order-independent among themselves.
