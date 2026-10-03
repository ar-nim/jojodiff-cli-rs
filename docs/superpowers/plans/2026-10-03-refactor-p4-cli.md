# Refactor Phase 4 — CLI Idiomatic Treatment — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A ≤80-line binary, a unit-testable `cli` module in the library, one error boundary mapping `JDiffError` → pinned stderr + exit code, and the modern test trio (`tempfile`, `assert_cmd`, `predicates`) in the harness.

**Architecture:** Strangler migration: every task moves one block out of `src/bin/jdiff.rs` into `src/cli/` and flips the binary's calls to it — the tree compiles and passes the full gate after every task, no transitional dead code. Function dispatch, parsing, sizing, text blocks, phases, and the boundary land in `cli::{config, report, error, diff_phase, patch_phase, run}`; the binary ends as argv → `cli::run` → exit code.

**Tech Stack:** Rust (edition 2024, MSRV 1.85); `thiserror` (present since Phase 3) + `anyhow = "1"` (added in Task 5); dev-deps `tempfile = "3"`, `assert_cmd = "2"`, `predicates = "3"` (Task 6).

**Spec:** `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` §8 (this phase), §3, §10, §13.

## Global Constraints

- Same core rules as the Phase 1 plan: worktree root, branch, pinned bytes (all CLI text is byte-pinned — every moved block moves **verbatim**), no `tests/fixtures/` edits, per-commit gate, conventional commits.
- Line numbers reference the pre-refactor tree; locate by identifier, verify with `rg`. Quoted code is authoritative.
- The library's `cli` module never calls `exit`/`process::exit` (Task 5 completes this; intermediate tasks may leave remaining `exit` sites in the binary only).
- Phase 3 left `engine_code`-style shims in the binary — Task 3 deletes them.
- The write-error policy is explicit and unchanged: diff path ignores (`IgnoringWriter`), patch path checks (`EXI_WRI`). `IgnoringWriter` keeps its name and its doc; it moves to `cli` unchanged.
- Text reconciliation rule (Task 3): when two text sources exist for one error (legacy failure-point `eprintln` vs `exit_switch` arm), the round-trip tests' asserted stderr is the authority — reproduce exactly, print once.

---

### Task 1: `cli::config` — options parsing and buffer sizing out of the binary

**Files:**
- Create: `src/cli/config.rs`
- Modify: `src/cli/mod.rs` (`pub mod config;` + re-exports), `src/bin/jdiff.rs` (the option loop ~129-333, argv[0] dispatch ~93-107, default settings ~109-127, buffer math ~380-431)

**Interfaces:**
- Produces:

```rust
/// Function to execute (`enum {Diff, Patch, Dedup, Test} liFun`,
/// `main.cpp:293`). Dedup/Test are ported per rulings §21.4/§21.3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Function {
    Diff,
    Patch,
    Dedup,
    Test,
}

/// Read the function from argv[0] (`main.cpp:303-315`): the basename
/// after the last '/' or '\\', case-insensitively. `jpatch*` → Patch;
/// `jptch*` → Patch is the port extension (spec §21.2). `jdedup`/`jtst`
/// routes are not ported and fall through to Diff.
pub fn function_from_argv0(cmd: &OsStr) -> Function { … }

/// Parsed options (`main.cpp:276-476`): everything the phases need. The
/// recorded-but-unread knobs (`-s`, `-t <n>`) are kept as fields for the
/// §21.11/§21.3 parity documentation.
#[derive(Debug)]
pub struct Options {
    pub fun: Function,
    pub out_typ: i32,      // 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn (3 = dedup)
    pub verbose: i32,
    pub src_bkt: bool,
    pub cmp_all: bool,
    pub src_scn: i32,
    pub mch_max: i32,
    pub mch_min: i32,
    pub hsh_mbt: i32,
    pub buf_org: i64,
    pub buf_new: i64,
    pub blk_sze: i32,
    pub ahd_max: i32,
    pub li_hlp: i32,       // 0=no, 1=-h, 2=-hh, 3=error
    pub compat_081: bool,
    pub seq_org: bool,
    pub seq_new: bool,
    /// Operands after GNU permutation, plus `optind`.
    pub operands: Vec<OsString>,
    pub opt_arg_cnt: usize,
}

/// Parse the command line (`main.cpp:318-476`): getopt_long with GNU
/// permutation; `?` sets li_hlp=1 and parsing CONTINUES.
pub fn parse(args: &[OsString]) -> Options { … }

/// Effective buffer geometry (`main.cpp:617-645`): defaults per
/// sequentiality, MB→bytes, block alignment warnings (byte-pinned
/// texts), the ahd_max default. Pure computation + pinned `dbg_print`
/// warnings; no process exits.
#[derive(Debug)]
pub struct Buffers {
    pub ll_buf_org: i64,
    pub ll_buf_new: i64,
    pub blk_sze: i32,
    pub ahd_max: i32,
}

pub fn size_buffers(opts: &Options) -> Buffers { … }
```

- [ ] **Step 1: Move the code**

Bodies move verbatim from the binary: `function_from_argv0` from the argv[0] block; `parse` from the `Getopt::new` loop **including** the `-d` debug-name match and `VAL_COMPAT_081` arm (the cfg(debug) imports follow it); `size_buffers` from the `main.cpp:617-645` computations including the two misalignment warnings and the `ahd_max` default block. `parse` stores `opt.operands()`/`opt.optind()` instead of leaving them to the caller. The binary's `real_main` becomes: collect args → `let opts = config::parse(&args);` → destructure what it still needs.

- [ ] **Step 2: Unit tests (new — these were subprocess-only before)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn argv0(name: &str) -> OsString { OsString::from(name) }

    #[test]
    fn argv0_dispatch_rules() {
        assert_eq!(function_from_argv0(&argv0("jdiff")), Function::Diff);
        assert_eq!(function_from_argv0(&argv0("/usr/bin/jpatch")), Function::Patch);
        assert_eq!(function_from_argv0(&argv0("JPTCH-OLD")), Function::Patch); // port ext, case-insensitive
        assert_eq!(function_from_argv0(&argv0("jtst")), Function::Diff);       // not ported (§21.3)
    }

    #[test]
    fn parse_defaults_and_clusters() {
        let o = parse(&[argv0("jdiff"), OsString::from("a"), OsString::from("b")]);
        assert_eq!((o.mch_max, o.mch_min, o.hsh_mbt), (128, 2, 32));
        let o = parse(&[argv0("jdiff"), OsString::from("-vvb")]);
        assert_eq!(o.verbose, 2);
        assert!(o.cmp_all && o.src_bkt && o.src_scn == 1);
        assert_eq!(o.mch_min, 4); // 2 *2
    }

    #[test]
    fn parse_unknown_option_sets_help_and_continues() {
        let o = parse(&[argv0("jdiff"), OsString::from("-Z")]);
        assert_eq!(o.li_hlp, 1);
    }

    #[test]
    fn size_buffers_aligns_on_blocks() {
        let mut o = parse(&[argv0("jdiff"), OsString::from("-k"), OsString::from("8192")]);
        o.buf_org = 3 * 1024 * 1024 + 1; // force misalignment
        let b = size_buffers(&o);
        assert_eq!(b.ll_buf_org % i64::from(b.blk_sze), 0);
        assert!(b.blk_sze >= 4096); // floored
    }
}
```

(Adjust the `-vvb` expectations against the loop's actual arithmetic — `mch_min *2`, `mch_max *4`, `hsh_mbt *4`, `buf_org` growth — before committing; the assertions above encode those rules.)

- [ ] **Step 3: Full gate + commit**

```bash
git add src/cli/ src/bin/jdiff.rs
git commit -m "refactor: option parsing and buffer sizing move to cli::config (unit-tested)"
```

---

### Task 2: `cli::report` — the byte-pinned text blocks

**Files:**
- Create: `src/cli/report.rs`
- Modify: `src/cli/mod.rs`, `src/bin/jdiff.rs` (delete `print_greeting`/`print_usage`/`print_notes` ~897-1135)

**Interfaces:**
- Produces: `pub fn print_greeting()`, `pub fn print_usage(mch_min: i32, mch_max: i32)`, `pub fn print_notes()` — `pub(crate)` bodies moved **byte-for-byte** (each `dbg_print!` line unchanged; these are §21.10-pinned including stale texts and the "disbale" typo).

- [ ] **Step 1: Move the three functions verbatim; binary imports them**

`real_main`'s greeting/usage/exit-on-args block calls `report::print_greeting()` etc. No other change.

- [ ] **Step 2: Full gate + commit** (round-trip usage/greeting tests pin every byte)

```bash
git add src/cli/ src/bin/jdiff.rs
git commit -m "refactor: greeting/usage/notes move verbatim to cli::report"
```

---

### Task 3: `cli::error` — the single boundary

**Files:**
- Create: `src/cli/error.rs`
- Modify: `src/error.rs` (add CLI variants), `src/cli/mod.rs`, `src/bin/jdiff.rs` (`exit_switch`, `open_inputs`'s error arms, the output-open helper, the both-stdin and not-enough-args exits; delete the Phase 3 `engine_code` shim)

**Interfaces:**
- Produces (variants added to `JDiffError` in `src/error.rs`):

```rust
    /// `EXI_ARG` — not enough arguments, or both inputs from stdin.
    #[error("Error in arguments !")]
    Args,
    /// `EXI_FRT` — could not open the first (source) file.
    #[error("Could not open first file {name} for reading.")]
    OpenFirst { name: OsString, #[source] source: std::io::Error },
    /// `EXI_SCD` — could not open the second (destination/patch) file.
    #[error("Could not open second file {name} for reading.")]
    OpenSecond { name: OsString, #[source] source: std::io::Error },
    /// `EXI_OUT` — could not open the output file (create or append).
    #[error("Could not open output file {name} for writing.")]
    OpenOutput { name: OsString, append: bool, #[source] source: std::io::Error },
    /// `EXI_ERR` — the un-ported dedup route (§21.4).
    #[error("Error occurred !")]
    NotPorted,
```

with `exit_code()` extended: `Args => -EXI_ARG`, `OpenFirst{..} => -EXI_FRT`, `OpenSecond{..} => -EXI_SCD`, `OpenOutput{..} => -EXI_OUT`, `NotPorted => -EXI_ERR`.

And the boundary in `src/cli/error.rs`:

```rust
/// The single error→(code, stderr) boundary. Prints the pinned message
/// (the legacy `exit_switch` texts — including their leading blank line
/// and trailing " !"-forms) and returns the positive process exit code.
/// Success codes keep the 0.8.5 swap: EXI_EQL→0, EXI_DIF→1, with the
/// verbose verdict lines (`main.cpp:897-932`).
pub fn report(li_ret: Result<i32, JDiffError>, verbose: i32) -> i32 { … }
```

- [ ] **Step 1: Text reconciliation (do this first)**

Run the error-path round-trip tests (`cargo test --test roundtrip -- full` and any open-failure tests) and capture the exact stderr for: seek/read/write/large/memory errors, not-enough-args, both-stdin, first/second/output open failures, `-y`. Today some errors print at the failure point (`open_inputs` etc.) AND/OR in `exit_switch`; Phase 3 moved engine texts into `Display`. Decide per error: **one** text, printed **once**, at boundary call time, matching the captured bytes and order. Where the legacy text is name-carrying (open failures), it is the `#[error]` string above (verbatim from the current `dbg_print` sites); where `exit_switch` printed a different blanket text (e.g. `"\nError writing file !\n"` for `EXI_WRI`), the boundary prints the `exit_switch` form and the engine `Display` form is the same string modulo the leading `\n` — align them by making `report` prefix `\n` (matching `exit_switch`) and keeping `Display` newline-free. Record any test-observed mismatch in the commit body.

- [ ] **Step 2: Move the failure sites to `Result`**

`open_inputs`, the output-open helper, and the two argument-error sites in the binary return `Err(JDiffError::…)` with the name moved into the variant; the `dbg_print`+`exit` pairs are deleted. `exit_switch` becomes `report` (moved to `cli::error`), and `real_main` ends `report(ret, verbose)` → returned to `main` which exits. Delete the `engine_code` shim: engine `Result`s flow into `report` directly. Note: `not enough arguments` also gates greeting/usage printing — keep that printing at its current call site in `run`/`real_main`, only the message+code goes through the variant.

- [ ] **Step 3: Boundary unit test + full gate + commit**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::JDiffError;

    #[test]
    fn exit_codes_are_the_exi_negation() {
        assert_eq!(report(Ok(crate::defs::EXI_EQL), 0), 0);
        assert_eq!(report(Ok(crate::defs::EXI_DIF), 0), 1);
        assert_eq!(report(Err(JDiffError::Args), 0), 2);
        assert_eq!(report(Err(JDiffError::OpenOutput { ..default-test-value.. }), 0), 5);
    }
}
```

(concretize the `OpenOutput` construction with a real `io::Error::other("x")` and an empty `OsString`).

```bash
git add src/error.rs src/cli/ src/bin/jdiff.rs
git commit -m "refactor: single cli::error boundary maps JDiffError to pinned stderr + exit codes"
```

---

### Task 4: `diff_phase` and `patch_phase`

**Files:**
- Create: `src/cli/diff_phase.rs`, `src/cli/patch_phase.rs`, `src/cli/run.rs` (shared: `Inputs`, `Sink`, `Input`, `open_dash`, `IgnoringWriter`, `open_inputs`, the writer-wrap helpers)
- Modify: `src/cli/mod.rs`, `src/bin/jdiff.rs` (the two phase blocks: diff ~466-638, patch ~646-704)

**Interfaces:**
- Produces:

```rust
// src/cli/run.rs
pub(crate) struct Inputs { pub org: Box<dyn JFile>, pub new: Box<dyn JFile> }
pub(crate) fn open_inputs(nam_org: &OsStr, nam_new: &OsStr, buf_org: i64, buf_new: i64, blk_sze: i32) -> Result<Inputs, JDiffError>
// Input/Sink/IgnoringWriter move here unchanged (Write/Read/Seek impls included).

// src/cli/diff_phase.rs
/// The diff (and `-t` first-half) phase (`main.cpp:776-869`): sequential
/// auto-detect warnings (mutating `opts` exactly as the C++ locals),
/// writer construction with the diff-path ignore policy, JDiff wiring,
/// parameter echo, execution, the EXI_EQL/EXI_DIF swap, statistics.
pub(crate) fn diff_phase(
    opts: &mut Options,
    nam_out: &OsStr,
    buffers: &Buffers,
    file_out: Option<Sink>,
) -> Result<i32, JDiffError>

// src/cli/patch_phase.rs
/// The patch (and `-t` second-half) phase (`main.cpp:871-876`): fresh
/// readers, append-vs-create output per `-t`, checked writes (EXI_WRI),
/// JPatcht wiring and execution.
pub(crate) fn patch_phase(
    opts: &mut Options,
    nam_org: &OsStr,
    nam_new: &OsStr,
    nam_out: &OsStr,
    buffers: &Buffers,
    out_is_stdout: bool,
    is_test: bool,
) -> Result<i32, JDiffError>
```

- [ ] **Step 1: Move the blocks**

The phase bodies move verbatim (including the sequential-file warnings that mutate `seq_org`/`cmp_all`/`src_bkt`/`src_scn`/`mch_min` on `opts`, the parameter echo, the statistics blocks, and all deviation comments). `real_main` shrinks to: parse → report/greeting/usage gate → operands → `size_buffers` → open output (`Result`) → match `opts.fun` over the phases → `error::report`. The dead `let _ = (seq_org, seq_new)` mirror note moves with the phase that owns the locals.

- [ ] **Step 2: Full gate + commit** (the `-t` round-trip tests and argv[0] dispatch tests pin the phase wiring)

```bash
git add src/cli/ src/bin/jdiff.rs
git commit -m "refactor: diff/patch phases and shared file plumbing move into cli"
```

---

### Task 5: `cli::run` orchestration + the thin binary + `anyhow`

**Files:**
- Modify: `src/cli/run.rs` (add `run`), `src/cli/mod.rs` (re-export `run`)
- Modify: `src/bin/jdiff.rs` (becomes the thin wrapper), `Cargo.toml` (`anyhow = "1"`)

**Interfaces:**
- Produces:

```rust
/// Full CLI execution (`main.cpp`): returns the process exit code —
/// never exits itself. `main.cpp`'s structure: argv[0] dispatch, parse,
/// greeting/usage gate, operand extraction, buffer sizing, output open,
/// function dispatch, boundary report.
pub fn run(args: &[OsString]) -> Result<i32, JDiffError>
```

and the entire new `src/bin/jdiff.rs`:

```rust
//! `jdiff` — thin wrapper: argv in, exit code out (spec §8.1). All logic
//! lives in the library's `cli` module; the module-level documentation of
//! the four CLI deviations lives in `jojodiff_cli_rs::cli`.

use std::ffi::OsString;

fn main() -> anyhow::Result<()> {
    let args: Vec<OsString> = std::env::args_os().collect();
    // Pinned errors are printed and coded inside `cli::run`'s boundary;
    // anyhow context wraps only truly unexpected failures.
    let code = jojodiff_cli_rs::cli::run(&args).map_err(anyhow::Error::from)?;
    std::process::exit(code);
}
```

(If `JDiffError: std::error::Error + Send + Sync + 'static` — it is, via thiserror — `.context("jdiff: fatal")` may replace the `map_err`; keep whichever reads cleaner. The four module-doc deviation notes currently at the top of the binary move to `src/cli/mod.rs`'s `//!` docs, verbatim.)

- [ ] **Step 1: Move `real_main`'s remainder into `run`, replace the binary**

`run` holds the orchestration left in `real_main` after Task 4; the only `exit()`/`process::exit()` in the crate is now `main`'s final `std::process::exit(code)` plus the two oracle-pinned `cfg(debug)` asserts in `ahead.rs` (Phase 3 ruling). Count: `rg -n 'process::exit|[^_\w]exit\(' src/` — expect exactly those sites.

- [ ] **Step 2: Unit tests for `run`'s dispatch (in-process, first time possible)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_with_no_args_is_arg_error_exit_2() {
        let code = run(&[OsString::from("jdiff")]).unwrap_or_else(|e| e.exit_code() * -1);
        // via the boundary instead: report(run(..), 0) == 2
        assert_eq!(crate::cli::error::report(run(&[OsString::from("jdiff")]), 0), 2);
    }
}
```

(Use the `report(run(...))` form in the committed test; the first line above is illustrative. Add sibling tests: both-stdin → 2; `-y` → 20; unknown option + no operands → 2 with `li_hlp` path printing usage — assert only codes here; byte-exactness stays with the round-trip suite.)

- [ ] **Step 3: Full gate + commit**

```bash
git add src/cli/ src/bin/jdiff.rs Cargo.toml Cargo.lock
git commit -m "refactor!: thin jdiff binary; cli::run is the testable entry point (anyhow wrapper)"
```

---

### Task 6: Test-harness modernization — `tempfile`, `assert_cmd`, `predicates`

**Files:**
- Modify: `Cargo.toml` (`[dev-dependencies]` += tempfile, assert_cmd, predicates)
- Modify: `tests/roundtrip.rs`, `tests/crossver.rs`, `tests/debug.rs`, `tests/debug_off.rs`, `tests/oracle.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `tests/common/mod.rs` (shared harness):

```rust
//! Shared test harness (spec §8.4): tempfile-backed scratch dirs with
//! RAII cleanup, and an assert_cmd factory for spawning the binary.

use std::path::PathBuf;
use tempfile::TempDir;

/// Scratch dir under the system temp root, auto-removed on drop.
/// Bounded by construction: tests write fixtures here, never unbounded
/// streams (spec §10 bounded-loops rule).
pub fn scratch(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("jdiff-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("create scratch dir")
}

/// Spawns the jdiff binary with args.
pub fn jdiff(args: &[&std::ffi::OsStr]) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::new(env!("CARGO_BIN_EXE_jdiff"));
    c.args(args);
    c
}
```

- [ ] **Step 1: Add the dev-deps and the harness; migrate file by file**

  1. Dev-deps (versions: `tempfile = "3"`, `assert_cmd = "2"`, `predicates = "3"`; MSRV note from spec §8.4 — plain `cargo check` in the msrv job never compiles dev-deps).
  2. `tests/roundtrip.rs`: replace the local `temp_dir`/guard helpers with `common::scratch`; convert plain spawn sites to `common::jdiff(...)` + predicates where it reads better. **Keep unchanged:** the seekable-stdin `Stdio::from(File)` wiring (a real file is the behavior under test) and any argv[0] copy-the-binary steps.
  3. `tests/crossver.rs` + `tests/debug.rs` + `tests/debug_off.rs`: same migration; `crossver`'s `DirGuard` and `debug`'s manual `remove_dir_all` calls delete in favor of `TempDir` drops.
  4. `tests/oracle.rs`: only its scratch usage migrates; the oracle env discovery stays.

- [ ] **Step 2: Full gate + commit**

```bash
git add Cargo.toml Cargo.lock tests/
git commit -m "test: tempfile/assert_cmd/predicates harness; RAII scratch everywhere"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** §8.1 layout → Tasks 1-5 (all named modules exist; shared plumbing in `run.rs` per the task, a deliberate single deviation from the spec's seven-file sketch — ledger as a ruling if it survives review); §8.2 → Task 3 (+ Task 5 anyhow); §8.3 → unchanged-by-construction (`IgnoringWriter` moves, policy comments stay); §8.4 → Task 6 + the unit tests in Tasks 1/3/5.
- **Placeholder scan:** function bodies marked `…` are verbatim-move instructions naming their source blocks (the content exists in the tree; the plan forbids paraphrasing). The Task 3 reconciliation step is a measurement-then-decide procedure, not an unfilled placeholder — its inputs (test-captured stderr) are runtime facts.
- **Type consistency:** `Options`/`Buffers` field names are the binary locals' names (`mch_max`, `ll_buf_org`, …) so moved bodies compile unchanged; phase signatures share `&mut Options` for the sequential auto-detect mutations; `report(Result<i32, JDiffError>, verbose)` is produced by Task 3 and consumed by Task 5's `run`.
- **Ordering:** strictly strangler-ordered 1→6; Task 4 depends on Task 3's error variants and Task 1's `Options`; Task 5 depends on all prior.
