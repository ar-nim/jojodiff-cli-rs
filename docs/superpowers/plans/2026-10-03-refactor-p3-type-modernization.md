# Refactor Phase 3 — Engine Type Modernization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the i32 sentinel protocols with types — `ByteOrEof` reads, the `Op` opcode enum, `Result<_, JDiffError>` engine-wide, the `CmpVal` node-state enum — with zero observable change.

**Architecture:** Bridge migrations (Parallel Change): the typed API is introduced alongside the i32 one, callers migrate module-by-module while every commit stays green, then the original is flipped/removed. `from_raw`/`to_i32` conversions bound the blast radius of the 134 `get` call sites; true `match` translation is concentrated where semantics actually branch on sentinels.

**Tech Stack:** Rust (edition 2024, MSRV 1.85) + `thiserror = "2"` (added in Task 1; first runtime dependency).

**Spec:** `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` §7 (this phase), §3, §10, §13.

## Global Constraints

- Same core rules as the Phase 1 plan: worktree root, branch, pinned bytes, no `tests/fixtures/` edits, per-commit gate (`fmt` + `clippy -D warnings` + `cargo test --all-features`), conventional commits.
- **Line numbers reference the pre-Phase-1 tree; locate by identifier, verify with `rg`.** Quoted code is authoritative.
- Pinned edges that MUST survive (each has a pinning test that will fail loudly): EOB is only ever returned for soft-ahead reads; the negative-position EOF gate (§21.17) surfaces as `Eof`; `uf_get_int`'s EOF arithmetic (`pch_get` → `EOF` → tier math yielding 0) is preserved via the `pch_get` i32 boundary; `is_best`'s negated-estimate `cmp` values are algorithm, not accident.
- The two `cfg(debug)` parity asserts in `ahead.rs` (`process::exit(-EXI_SEK)` at the `getbuf` invariant, ~:391/:403) are **kept as-is** — Ruling: their observable behavior (debug build exits 6, pinned by the debug oracle tests) cannot be reproduced by a `Result`; the spec's acceptance line "no `process::exit` in library code" gets a carve-out for exactly these two sites in Phase 5. Ledger this ruling.
- Migration hygiene: a `to_i32()` pass-through is acceptable only at true representation boundaries (`pch_get`, `JOut::put` detail params); semantic branch sites (`< 0`, `>= 0`, `== EOF`, `<= EOF`) translate to real `match`es. Task 8 audits the count.

---

### Task 1: `thiserror` dependency + `JDiffError`

**Files:**
- Modify: `Cargo.toml` (`[dependencies]` — first runtime dep; keep the comment block updated)
- Create: `src/error.rs`
- Modify: `src/lib.rs` (`pub mod error;`)

**Interfaces:**
- Produces:

```rust
//! Library-wide error type: the negative `EXI_*` C vocabulary as a Rust
//! type. Engine APIs return `Result<_, JDiffError>`; the CLI boundary maps
//! [`JDiffError::exit_code`] to the process exit code (negating, like
//! `exit(-EXI_*)`) and prints the pinned texts.

/// Engine error (`EXI_*`, `JDefs.h:146-158`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // API-07: CLI variants arrive in Phase 4; downstream must not match exhaustively
pub enum JDiffError {
    /// `EXI_SEK` — seek error (`JFileAhead.h:115`).
    #[error("Seek error !")]
    Seek,
    /// `EXI_RED` — read error.
    #[error("Error reading file !")]
    Read,
    /// `EXI_WRI` — write error, with the underlying I/O failure.
    #[error("Error writing file !")]
    Write(#[from] std::io::Error),
    /// `EXI_MEM` — allocation failure.
    #[error("Error allocating memory !")]
    Memory,
    /// `EXI_LRG` — 64-bit number rejected.
    #[error("Error: 64-bit offsets not supported !")]
    Large,
}

impl JDiffError {
    /// The raw negative engine code (`EXI_SEK` etc.); the process exit code
    /// is its negation, as in `exit(-EXI_*)`.
    pub fn exit_code(&self) -> i32 {
        match self {
            JDiffError::Seek(_) | JDiffError::Seek => -crate::defs::EXI_SEK,
            JDiffError::Read => -crate::defs::EXI_RED,
            JDiffError::Write(_) => -crate::defs::EXI_WRI,
            JDiffError::Memory => -crate::defs::EXI_MEM,
            JDiffError::Large => -crate::defs::EXI_LRG,
        }
    }
}
```

(Clean up the `Seek` arm — one variant, no payload; the listing above must read `JDiffError::Seek =>`. The `#[error]` strings are placeholders for the **verbatim** current texts — see Step 1.)

- [ ] **Step 1: Harvest the pinned texts**

`rg -n 'eprintln!|Error writing|Error reading|Seek error|64-bit' src/jfileout.rs src/jpatcht.rs` — the five library-side error prints (`jfileout.rs:68,75,94,105`, `jpatcht.rs:306` in the pre-P1 tree) carry the byte-pinned texts. Copy each into the matching `#[error("…")]` verbatim **including trailing `!` and any `\n` placement decisions** (if a site prints a trailing newline, keep that newline at the boundary print in Task 6, not in Display — Display stays newline-free; note which sites had newlines). Do not invent or normalize spelling.

- [ ] **Step 2: Wire the dependency and module**

`Cargo.toml`:

```toml
[dependencies]
thiserror = "2"
```

`src/lib.rs`: add `pub mod error;` to the module list; update the crate doc's "std-only" sentence to name the dependency and why (spec §13). `cargo check` + doc build.

- [ ] **Step 3: Unit test + full gate + commit**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// ERRORS-12 / API-15: the error type crosses `anyhow` and thread
    /// boundaries in the binary — lock `Send + Sync + 'static` at compile time.
    #[test]
    fn jdiff_error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<JDiffError>();
    }

    #[test]
    fn exit_codes_match_exi() {
        assert_eq!(JDiffError::Seek.exit_code(), -crate::defs::EXI_SEK);
        assert_eq!(JDiffError::Read.exit_code(), -crate::defs::EXI_RED);
        assert_eq!(JDiffError::Memory.exit_code(), -crate::defs::EXI_MEM);
        assert_eq!(JDiffError::Large.exit_code(), -crate::defs::EXI_LRG);
    }
}
```

```bash
git add Cargo.toml Cargo.lock src/error.rs src/lib.rs
git commit -m "feat: JDiffError (thiserror) as the typed engine error vocabulary"
```

---

### Task 2: `ByteOrEof` type + `getv` bridge on `JFile`

**Files:**
- Modify: `src/jfile/mod.rs`

**Interfaces:**
- Produces:

```rust
/// The result of a [`JFile::getv`] read: a byte, end of input, the
/// soft-ahead end-of-buffer, or an error. Replaces the raw `i32` channel
/// (`0..=255` / `EOF` / `EOB` / `EXI_SEK` / `EXI_RED`) — same information,
/// typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive] // API-07; crate-internal matches stay exhaustive
pub enum ByteOrEof {
    /// A data byte.
    Byte(u8),
    /// `EOF` — end of input, or a gated negative position (§21.17).
    Eof,
    /// `EOB` — soft look-ahead past the buffer window (soft-ahead only).
    Eob,
    /// A read-side error (`EXI_SEK`/`EXI_RED`).
    Err(crate::error::JDiffError),
}

impl ByteOrEof {
    /// Reconstructs the legacy `i32` channel value. Boundary use only
    /// (`pch_get`, `JOut::put` detail args); semantic branches should
    /// `match` instead.
    pub(crate) fn to_i32(self) -> i32 {
        match self {
            ByteOrEof::Byte(b) => i32::from(b),
            ByteOrEof::Eof => crate::defs::EOF,
            ByteOrEof::Eob => crate::defs::EOB,
            ByteOrEof::Err(e) => e.exit_code(),
        }
    }

    /// Maps a legacy `get` return onto the typed form.
    pub(crate) fn from_raw(v: i32) -> ByteOrEof {
        match v {
            0..=255 => ByteOrEof::Byte(v as u8),
            x if x == crate::defs::EOF => ByteOrEof::Eof,
            x if x == crate::defs::EOB => ByteOrEof::Eob,
            x if x == -crate::defs::EXI_SEK => ByteOrEof::Err(crate::error::JDiffError::Seek),
            x if x == -crate::defs::EXI_RED => ByteOrEof::Err(crate::error::JDiffError::Read),
            other => unreachable!("legacy get channel held {other}"),
        }
    }
}
```

Trait addition (bridge):

```rust
    /// Typed read (bridge during the Phase 3 migration; becomes `get`'s
    /// signature in Task 4): [`ByteOrEof`] for the same byte/sentinel
    /// information `get` returns today.
    fn getv(&mut self, pos: i64, typ: ReadType) -> ByteOrEof {
        ByteOrEof::from_raw(self.get(pos, typ))
    }
```

- [ ] **Step 1: Add the type, bridge, and tests**

Tests (inline in `src/jfile/mod.rs`): `from_raw` maps 0/255/EOF/EOB/EXI-codes; `to_i32` round-trips all five; `getv` on `JFileMem` returns `Byte(b)` for data and `Eof` past the end and for negative positions (that last assertion is the §21.17 pin — copy its expectations from the existing `mem.rs` tests).

- [ ] **Step 2: Full gate + commit**

```bash
git add src/jfile/mod.rs
git commit -m "feat: ByteOrEof typed read with getv bridge on JFile"
```

---

### Task 3: Migrate engine `get` callers to `getv` (module by module)

**Files (one commit each):**
- Modify: `src/jdiff.rs` (21 sites), `src/jmatchtable.rs` (2), `src/jfileout.rs` (4), `src/jpatcht.rs` (2 via `pch_get`)

**Interfaces:**
- Consumes: Task 2's `getv`, `to_i32`.
- Produces: engine code reading `ByteOrEof`; `pch_get(&mut self) -> i32` becomes the single i32 boundary in `jpatcht`.

- [ ] **Step 1: `src/jpatcht.rs` — boundary pattern**

`pch_get` (pre-P3 tree ~:76) keeps its `i32` return and becomes:

```rust
    /// One patch byte or `EOF` (`JPatcht.cpp`). The decoder's length
    /// arithmetic computes with `EOF` like the C, so this is deliberately
    /// the i32 representation boundary of the typed reader.
    fn pch_get(&mut self) -> i32 {
        self.fil_pch.getv(self.pos_pch, ReadType::Read).to_i32()
    }
```

(Check the actual body — the position advance and field names stay as they are; only the read converts.) `uf_get_int` and friends remain untouched: their EOF arithmetic is pinned.

- [ ] **Step 2: `src/jdiff.rs` — branch-translate pattern**

Enumerate: `rg -n '\.(org|r#new|mch|inp)\.get\(|self\.org\.get\(|self\.r#new\.get\(' src/jdiff.rs`. For each site decide by shape:

Pass-through (value only stored/passed to `out.put` as detail): `let lc_org = self.org.get(p, ReadType::Read);` → `let lc_org = self.org.getv(p, ReadType::Read).to_i32();` — allowed with no comment ONLY until Task 8's audit; prefer storing `ByteOrEof` where the local is compared later.

Branch site (the common `while lc_org == lc_new && lc_new >= 0 && …` family in `jdiff`/`scan_equal_run`/`search`): translate to a real match, e.g.:

```rust
        while let (ByteOrEof::Byte(o), ByteOrEof::Byte(n)) = (lc_org, lc_new) {
            if o != n || lz_pos_new >= lap_sml {
                break;
            }
            …
        }
```

storing the locals as `ByteOrEof` (with `EOF`-sentinel locals like `lc_org` after an EOF read becoming `ByteOrEof::Eof`, which the tuple pattern then rejects exactly like `lc_new >= 0` did). Work function by function (`jdiff` main loop, `search`, `scan_equal_run`, `ufFndAhd*` helpers); after each function, `cargo test --lib jdiff`.

- [ ] **Step 3: `src/jmatchtable.rs` + `src/jfileout.rs`**

Same rules; `jfileout::copyfrom`'s `get` loop and the two `jmatchtable` compare sites translate to matches. One commit for both files.

- [ ] **Step 4: Full gate + one commit per module (3-4 commits)**

```bash
git commit -m "refactor: jpatcht reads through the typed boundary pch_get"
git commit -m "refactor: jdiff reads ByteOrEof (branch sites match, pass-through via to_i32)"
git commit -m "refactor: jmatchtable/jfileout read ByteOrEof"
```

---

### Task 4: Flip `JFile::get` to `ByteOrEof`; delete the bridge

**Files:**
- Modify: `src/jfile/mod.rs`, `src/jfile/mem.rs`, `src/jfile/ahead.rs`

**Interfaces:**
- Produces: `fn get(&mut self, pos: i64, typ: ReadType) -> ByteOrEof` — the trait's final signature. `getv` is deleted; `from_raw`/`to_i32` stay `pub(crate)` helpers.

- [ ] **Step 1: Flip the trait and implementations**

  1. Trait: rename `getv`'s body to `get` (delete the old `get` declaration; drop the "bridge" doc — `get`'s doc becomes the current `getv` doc merged with the original sentinel documentation, updated to describe the enum).
  2. `JFileMem::get` (mem.rs): native form —

```rust
    fn get(&mut self, pos: i64, _typ: ReadType) -> ByteOrEof {
        match self.data.get(pos as usize) {
            Some(&b) => ByteOrEof::Byte(b),
            None => ByteOrEof::Eof, // negative and past-end both gate to EOF (§21.17)
        }
    }
```

(Check `JFileMem`'s actual gating today — its tests pin negative→EOF; preserve exactly.)
  3. `JFileAhead::get` (ahead.rs): `fn get(&mut self, pos: i64, typ: ReadType) -> ByteOrEof { ByteOrEof::from_raw(self.get_frombuffer(pos, typ)) }` — the internal `get_frombuffer`/`getbuf` sentinel machinery (including the §21.17 gate and the two cfg(debug) parity asserts) stays i32-shaped internally; `from_raw` is the rim.
  4. Update the remaining in-crate callers' imports; `rg -n '\.getv\(' src/` must return zero hits.
  5. **API-26 verify-then-change on `getbuf`:** audit whether the `&mut i64` out-parameter is redundant — the run "never wraps the ring", so at every return site check whether `len == slice.len()`. If yes at all sites, change `getbuf` to return `Option<&[u8]>` alone and delete the out-param (callers derive `len` from the slice). If any site differs, keep the out-param and record the site in the commit body. Either way the §21.13 dead-parity surface does not grow.

- [ ] **Step 2: Full gate + commit**

```bash
git add src/jfile/ src/jdiff.rs src/jmatchtable.rs src/jfileout.rs src/jpatcht.rs
git commit -m "refactor!: JFile::get returns ByteOrEof (bridge removed)"
```

(Breaking lib change — the 0.9.0 release absorbs it; spec §2.1/§2.3.)

---

### Task 5: `Op` opcode enum through writers and decoder

**Files:**
- Modify: `src/defs.rs` (add `Op`; the six `i32` consts become `Op` associated constants or are removed — see step)
- Modify: `src/jout/mod.rs` (`JOut::put(opr: Op, …)`), `src/jout/bin.rs`, `src/jout/asc.rs`, `src/jout/rgn.rs`, `src/jpatcht.rs`, `src/jdiff.rs`

**Interfaces:**
- Produces:

```rust
/// Patch-format opcodes (`JDefs.h:163-168`): the wire values are the
/// discriminants. `Esc` doubles as the data-escape and the "no operator
/// yet" seed of `opr_cur`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive] // API-07; crate-internal matches stay exhaustive
pub enum Op {
    /// `ESC` 0xA7 — escape.
    Esc = 0xA7,
    /// `MOD` 0xA6 — modify.
    Mod = 0xA6,
    /// `INS` 0xA5 — insert.
    Ins = 0xA5,
    /// `DEL` 0xA4 — delete.
    Del = 0xA4,
    /// `EQL` 0xA3 — equal.
    Eql = 0xA3,
    /// `BKT` 0xA2 — backtrace.
    Bkt = 0xA2,
}
```

- [ ] **Step 1: Introduce `Op` and convert the six consts**

Add `Op` to `defs.rs`. Replace the six `pub const … : i32` with `impl Op { pub const fn byte(self) -> u8 { self as u8 } }` plus, for the transition, `pub fn from_byte(b: u8) -> Option<Op>`. Then migrate mechanically, file by file (`rg -n '\b(ESC|MOD|INS|DEL|EQL|BKT)\b' src/jout src/jpatcht.rs src/jdiff.rs src/jmatchtable.rs` enumerates ~390 uses; most are comparisons and `put` arguments):

  - `JOut::put(opr: i32, …)` → `put(opr: Op, …)`; the three writers' `match self.opr_cur` arms become `Op::Mod =>` etc.; `opr_cur: i32` fields become `opr_cur: Op` seeded `Op::Esc`.
  - Cast sites: `self.raw(ESC as u8)` → `self.raw(Op::Esc.byte())`; the `put_byte` range check `(BKT..=ESC).contains(&byt)` → `(Op::Bkt.byte()..=Op::Esc.byte()).contains(&(byt as u8))` (keep the surrounding i32 logic that computes `byt`).
  - `jdiff.rs` call sites: `self.out.put(EQL, 1, lc_org, …)` → `self.out.put(Op::Eql, 1, lc_org, …)`; engine-internal comparisons (`opr == INS`-style) convert to `Op` comparisons.
  - `jpatcht.rs` decode: the byte-vs-opcode dispatch (`opr` from the patch stream) classifies with `Op::from_byte(b)`; a non-opcode byte stays data exactly as today (the current `else`/default paths). The `li_opr: i32` parameters of `uf_put_dta`/`uf_get_dta` become `Op`.

- [ ] **Step 2: Full gate + commit** (golden85 + crossver suites are the wire-format proof)

```bash
git add src/defs.rs src/jout/ src/jpatcht.rs src/jdiff.rs src/jmatchtable.rs
git commit -m "refactor!: Op enum replaces the i32 opcode constants through writers and decoder"
```

---

### Task 6: `Result` engine-wide

**Files:**
- Modify: `src/jdiff.rs` (`jdiff`, `search`, `build_full_index`), `src/jpatcht.rs` (`jpatch`), `src/jfileout.rs` (`putc`, `copyfrom`), `src/bin/jdiff.rs` (call-site shim)

**Interfaces:**
- Produces: `pub fn jdiff(&mut self) -> Result<(), JDiffError>`, `fn search(&mut self, …) -> Result<(), JDiffError>`, `fn build_full_index(&mut self) -> Result<(), JDiffError>`, `pub fn jpatch(&mut self) -> Result<(), JDiffError>`, `pub fn putc(&mut self, ai_dta: i32) -> Result<(), JDiffError>`, `pub fn copyfrom(&mut self, inp: &mut dyn JFile, az_pos: i64, az_len: i64) -> Result<(), JDiffError>`.
- Consumes: Task 1's `JDiffError`.

- [ ] **Step 1: Convert the signatures bottom-up**

  1. `JFileOut::putc`/`copyfrom`: the `EXI_WRI`/`EXI_RED`/`EXI_SEK` return paths become `Err(JDiffError::Write/Read/Seek)`; **delete the five library-side `eprintln!` sites** (their texts now live in Display); the always-`1` return of `putc` is dropped after `rg -n '\.putc\(' src/` confirms callers ignore it (if a caller reads it — `uf_put_dta` returns `1` itself — keep that caller's `1` literal).
  2. `jdiff`/`search`/`build_full_index`/`jpatch`: `if li_ret < 0 { return li_ret; }` propagation becomes `?` on `Result`-returning callees; the negative-value returns (`return EXI_SEK;`-style) become `return Err(JDiffError::Seek);` etc. The success tail returns `Ok(())`. **The `EXI_EQL`/`EXI_DIF` decision stays with the CLI** (it reads `out_stats()`), exactly as today.
  3. `src/bin/jdiff.rs` call sites get a temporary shim (deleted in Phase 4):

```rust
/// Phase-3 shim: engine `Result` → the i32 vocabulary `exit_switch` still
/// speaks (removed in Phase 4 when the boundary maps `JDiffError` directly).
fn engine_code(r: Result<(), jojodiff_cli_rs::error::JDiffError>) -> i32 {
    match r {
        Ok(()) => jojodiff_cli_rs::defs::EXI_OK,
        Err(e) => e.exit_code(),
    }
}
```

with the error text printed once, at the same point `exit_switch` prints engine errors today (dbg_print the Display before the code match, mirroring the current message order — verify with the `/dev/full` round-trip test, which must still print the write-error text and exit 9).

- [ ] **Step 2: Full gate + commit**

```bash
git add src/jdiff.rs src/jpatcht.rs src/jfileout.rs src/bin/jdiff.rs
git commit -m "refactor!: engine APIs return Result<(), JDiffError>; error texts move to Display"
```

---

### Task 7: `CmpVal` — the `Node.cmp` sentinel field

**Files:**
- Modify: `src/jmatchtable.rs` (36 `.cmp` sites)

**Interfaces:**
- Produces:

```rust
/// Match-node state (`Node.cmp`, `JMatchTable.h`): run length when
/// non-negative; the C++ sentinels as variants; `is_best`'s negated EOB
/// distance estimates (negative values that are NOT the sentinels) as
/// `Est`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmpVal {
    /// Validated run length (C++ `>= 0`).
    Run(i32),
    /// `CMPINV` -1 — invalid, may be reused.
    Inv,
    /// `CMPSKP` -2 — very old, skipped.
    Skp,
    /// `CMPEOB` -3 — end-of-buffer estimate pending.
    Eob,
    /// A negative EOB distance estimate stored by `is_best` (algorithmic
    /// encoding, preserved faithfully; see the reuse-risk notes).
    Est(i32),
}
```

- [ ] **Step 1: Retype the field and migrate the sites**

Enumerate: `rg -n '\.cmp\b' src/jmatchtable.rs`. Migrate mechanically: `o.cmp != CMPINV` → `!matches!(o.cmp, CmpVal::Inv)` or pattern guards; `o.cmp > 0` → `if let CmpVal::Run(n) = o.cmp { n > 0 … }` shapes (site-specific; the elect chain in `is_best` and the reuse checks in `is_old2_skip`/`is_old2_reuse` are the delicate ones — translate each comparison preserving its exact arithmetic, including `tst + i64::from(cmp) > red_new` which becomes the `Run(n)` payload in scope); the `is_best` stores of negated estimates become `CmpVal::Est(v)`; debug prints of `.cmp` print the Debug form or a small helper matching today's numeric output (**check the debug-oracle-pinned traces: if a `-dmch` trace prints the number, keep the number via a `as_legacy_i32()` helper that inverts the mapping — Run(n)→n, Inv→-1, Skp→-2, Eob→-3, Est(v)→v — and print that**).
- Keep `CMPINV`/`CMPSKP`/`CMPEOB` consts only if the legacy-i32 helper wants them; otherwise inline the values in the helper.

- [ ] **Step 2: Full gate + commit** (run the debug-feature tests explicitly — `cargo test --all-features` already does)

```bash
git add src/jmatchtable.rs
git commit -m "refactor: Node.cmp sentinel field becomes the CmpVal enum"
```

---

### Task 8: Phase 3 close-out audit

**Files:** none (verification only)

- [ ] **Step 1: Boundary audit**

```bash
rg -n 'to_i32\(\)' src/          # only pch_get + JOut::put detail-arg conversions remain
rg -n '\.getv\(' src/            # zero hits
rg -n '\b(ESC|MOD|INS|DEL|EQL|BKT)\b' src/ | grep -v '// '   # only Op::… forms and wire bytes
rg -n '-> i32' src/jdiff.rs src/jpatcht.rs src/jfileout.rs   # only accessors (hsh_err etc.), no fallible ops
rg -n 'eprintln!' src/           # zero hits in library code
```

- [ ] **Step 2: Full gate + the two focused behavior proofs**

`cargo test --all-features` plus explicitly: the `/dev/full` patch-path test (exit 9 + write-error text) and the golden85 suites. Report the `to_i32` count and locations in the task report.

```bash
git commit --allow-empty -m "test: phase 3 close-out audit green"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** §7.1 → Tasks 2-4; §7.2 → Task 5; §7.3 → Task 6 (+ Task 1's type); §7.4 → Task 7; the cfg(debug) assert carve-out ruling → Global Constraints + Phase 5 follow-up; acceptance items 6's three clauses map to Tasks 4, 5, 6.
- **Placeholder scan:** Task 1's `#[error]` strings are explicitly verbatim-harvest instructions (the pinned texts live at named sites); Task 2's listing contains one deliberate self-correction note (`Seek` arm duplication typo flagged inline). No TBDs.
- **Type consistency:** `ByteOrEof::Err(JDiffError)` matches Task 1's enum; `exit_code()` sign convention (raw negative engine code, CLI negates for the process) is stated once in Task 1 and used consistently in Task 6's shim; `Op` discriminants equal the retired consts' values.
- **Known risk carried:** stderr interleaving of error texts moved from failure-point to boundary (Task 6) — verification step included (the `/dev/full` round-trip test); goldens do not cover a verbose+error interleaving, so the risk is bounded to untested combinations, as the spec's risk table records.
