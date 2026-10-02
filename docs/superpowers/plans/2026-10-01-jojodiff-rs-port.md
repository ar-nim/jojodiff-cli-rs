# JojoDiff → Rust (`jojodiff-cli-rs`) 1:1 Port Implementation Plan

> **STATUS (2026-10-02): Tasks 1–12 (0.8.1) are COMPLETE, reviewed and shipped as package
> version 0.8.1 (merged to main, CI green). The port is RE-TARGETED to JojoDiff 0.8.5
> (upstream commit 66a2806, vendored at `reference/jojodiff-0.8.5/`): spec PART II
> (`docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` §17–§22) is normative.
> Tasks 13–22 below upgrade the existing implementation to 0.8.5 parity. Tasks 1–12 are
> retained unchanged as the historical record of the 0.8.1 port.**

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A byte-compatible Rust port of the `jdiff` and `jptch` binaries (and a reusable library) from https://github.com/vibhorkalley/jojodiff, building and tested on Windows, Linux and macOS — published as **`jojodiff-cli-rs`** (repo and package; verified free on crates.io 2026-10-01). The name encodes all three distinguishing facts — `jojodiff` (brand), `-cli` (standalone tools, unlike the library-only `jojodiff` crate), `-rs` (Rust port, unlike the C++ original) — with no name conflict against `francisdb/jojodiff-rs` in any namespace.

**Architecture:** Direct 1:1 translation of the C++ class structure — one Rust module per C++ class, one shared `i32` byte-or-sentinel convention (`EOF=-1`, `EOB=-2`), `i64` offsets, `u32` wrapping hash keys. The engine is serial and deterministic. A fixed build of the vendored C++ source (2-line Linux fix, spec §15.1) is the acceptance oracle: patches, listings, logs and exit codes must match it byte-for-byte.

**Tech Stack:** Rust edition 2024 (MSRV 1.85, declared via `rust-version`), std-only (zero runtime dependencies). Dev-dependencies: `jojodiff` v0.1.2 (francisdb crate, cross-validation only), `pretty_assertions`. CI: GitHub Actions matrix ubuntu/windows/macos; oracle-compare job on ubuntu only.

**Spec:** `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` (functional inventory — every §number cited below is a section of that spec). The C++ tree is vendored pristine at `reference/jojodiff-cpp/` (Task 1); all `file:line` references point into it. **For Tasks 13–22:** spec PART II (§17–§22) is normative; the 0.8.5 tree is vendored at `reference/jojodiff-0.8.5/` and its `file:line` references point there; the verified change analysis is `docs/superpowers/research/2026-10-02-jojodiff-0.8.5-analysis.md`.

## Global Constraints

- License: **GPL-3.0** (the C++ original is GPLv3; the port is a derivative).
- Runtime dependencies: **none** (std only, no `unsafe`).
- Toolchain: edition **2024**, `rust-version = "1.85"`. The planned code uses no edition-2024-affected features (no `unsafe`, no `static mut`, no FFI, no RPITs), so the edition is a defaults/future-proofing choice only; dropping to 2021 later would be a one-line change with zero code impact. Edition 2024 defaults Cargo to `resolver = "3"` (MSRV-aware dependency resolution, needs Rust 1.84+) — with a std-only runtime this only affects dev-dependency selection, which is desirable. [Cargo Book, verified via Context7]
- Package/repo name: **`jojodiff-cli-rs`** (verified free on crates.io 2026-10-01; the bare `jojodiff` is taken by the francisdb library and `jojodiff-rs` collides with that project's GitHub repo — both rejected deliberately). Library target name: `jojodiff_cli_rs` (avoids lib-name collision with the `jojodiff` cross-validation dev-dependency). Binary names: `jdiff`, `jptch`. **Non-affiliation must be stated explicitly** in the crates.io description, the repo About box, and a README "Relationship to other projects" section (see Task 12 Step 5): independent port of Joris Heirbaut's original JojoDiff; not affiliated with or derived from the `jojodiff` crate / `francisdb/jojodiff-rs` (used only as an optional cross-validation consumer in tests). Version string in help output is fixed: `0.8.1 (beta) December 2011` (**0.8.5 re-target:** `0.8.5 (beta) 2020`, copyright `Copyright (C) 2002-2020 Joris Heirbaut`, package version 0.8.5 — spec §18.A).
- Byte-exact compatibility targets: patch files, `-l`/`-lr` listings, greeting/help/verbose/error text, exit codes 0/1/2/3/4/5/6/7/8/9/10/20 (spec §2–§5). **0.8.5 re-target (Tasks 13–22):** patch files per spec §18.C (implicit MOD, MINEQL=2 — breaking vs 0.8.1), `-l` hex + `-r` listings (§18.F), all new greeting/help/stats text (§18.A/D/F), exit codes with **0/1 swapped** (identical→0, differences→1; §18.D).
- Formatting convention: `P8zd` = `format!("{:>12}", v)` in release (default) builds; `{:>10}` when the `debug` feature is on (spec §14). (Unchanged at 0.8.5, now on `%zd`-style values — spec §18.A.)
- Integer quirks are part of the contract: `-m`/`-a` compute `atoi(v)/2*1024` (integer division first); `-s` divides by 1024 while `> 1024`; `-min`/`-max` clamp to 256; hash ops wrap on `u32` (spec §2, §4.1). **0.8.5 re-target:** the option set changes wholesale (spec §18.D table is normative — `-a`×1024, `-i` MB floor 1, `-k` floor 4096, `-m` MB-total split, `-n` floor 0, `-x` floor 1024, multiplicative presets, `c_atoi` still C-semantics).
- Deviations from C++ are only the seven documented in spec §15 (Linux fork bug, OpenMP, `-m 0` NUL fix, jptch stdin buffering, MinGW ifdefs collapsed, Ahead-impl unification, francisdb non-authoritative). **0.8.5 re-target:** deviations are the fifteen rulings of spec §21 (which also closes §15.1/2/3/10/11 as fixed upstream); §20 maps every 0.8.1 quirk's fate.

## File Structure (final state)

```
jojodiff-cli-rs/
├── Cargo.toml                  # lib + [[bin]] jdiff + [[bin]] jptch; feature "debug"
├── LICENSE                     # GPL-3.0
├── README.md
├── .gitignore                  # /target
├── .github/workflows/ci.yml    # 3-OS matrix + ubuntu oracle job
├── reference/
│   ├── PROVENANCE.md           # where each vendored tree came from
│   ├── jojodiff-cpp/           # pristine vendored 0.8.1 C++ tree (GPLv3)
│   └── jojodiff-0.8.5/         # pristine vendored 0.8.5 tree (upstream 66a2806)
├── scripts/
│   ├── build-oracle.sh         # applies 2-line fix, make, → target/oracle/{jdiff,jptch}
│   │                           #   (T13: builds the 0.8.5 oracle w/ hkey+LARGEFILE patches)
│   ├── gen-golden.sh           # regenerates tests/fixtures/golden/** from oracle
│   └── runtest.sh              # port of Makefile runtest + run.sh
├── src/
│   ├── lib.rs                  # pub mod wiring
│   ├── defs.rs                 # JDefs.h  (T1)
│   ├── jdebug.rs               # JDebug   (T1, extended T11)
│   ├── jfile/mod.rs            # JFile trait, ReadType (T2)
│   ├── jfile/mem.rs            # in-memory reader, -m 0 (T2)
│   ├── jfile/ahead.rs          # buffered look-ahead reader (T3)
│   ├── jhashpos.rs             # hash table (T4)
│   ├── jmatchtable.rs          # match table (T5)
│   ├── jout/mod.rs             # JOut trait + OutStats (T6)
│   ├── jout/bin.rs             # binary patch writer (T6)
│   ├── jout/asc.rs             # -l listing (T7)
│   ├── jout/rgn.rs             # -lr regions (T7)
│   ├── jdiff.rs                # diff engine (T8, reworked T17)
│   ├── jpatcht.rs              # JPatcht patch applier (T19, 0.8.5)
│   ├── jfileout.rs             # JFileOut patch-phase writer (T19, 0.8.5)
│   └── bin/
│       ├── jdiff.rs            # jdiff CLI (T9, rewritten T20: getopt_long, -u, argv[0])
│       └── jptch.rs            # jptch CLI ≡ jdiff forced-Patch (T10, re-pointed T20)
└── tests/
    ├── fixtures/
    │   ├── bkocomu.0000.fil / bkocomu.0009.fil / test2.001.txt / test2.002.txt
    │   ├── golden/                 # 0.8.1 goldens (kept for cross-version gate)
    │   └── golden85/<option-set>/...  # 0.8.5 oracle patches + listings (T22)
    ├── roundtrip.rs            # T9/T10/T12, extended T20/T22
    ├── crossver.rs             # 0.8.1 patch → 0.8.5 jptch compatibility (T22)
    └── oracle.rs               # T12 (skips unless JOJODIFF_ORACLE set / target/oracle exists)
```

Tasks 1–8 build the library bottom-up; 9–10 the CLIs; 11 the debug feature; 12 the oracle
harness, goldens, CI and docs. Every task compiles and tests green before its commit.
**Tasks 13–22 (spec Part II) re-target the completed 0.8.1 implementation to 0.8.5** —
reference/jojodiff-0.8.5/ is vendored pristine (see `reference/PROVENANCE.md`), and
`src/jfile/stdio.rs` (T14 note: the C++ stdio/istream adapters collapse into the single
Rust `JFileAhead` over seekable-or-sequential I/O, spec §21.11 — no new file unless the
implementer chooses one for the `JIo` abstraction).

---

### Task 1: Scaffold, `defs`, `jdebug`, vendor C++, license

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `LICENSE` (GPL-3.0 full text from `reference/jojodiff-cpp/COPYING`), `src/lib.rs`, `src/defs.rs`, `src/jdebug.rs`, `.github/workflows/ci.yml`, `reference/jojodiff-cpp/**` (pristine copy)
- Test: `src/defs.rs` (unit tests inline)

**Interfaces (produced, used by every later task):**
```rust
// defs.rs
pub const EOF: i32 = -1;
pub const EOB: i32 = EOF - 1;                      // -2
pub const SMPSZE: i32 = 32;
pub const MCH_PME: i64 = 127;
pub const MCH_MAX: i32 = 256;
pub const ESC: i32 = 0xA7;  pub const MOD: i32 = 0xA6;  pub const INS: i32 = 0xA5;
pub const DEL: i32 = 0xA4;  pub const EQL: i32 = 0xA3;  pub const BKT: i32 = 0xA2;
pub const EXI_DIF: i32 = 0;  pub const EXI_EQL: i32 = 1;  pub const EXI_ARG: i32 = 2;
pub const EXI_FRT: i32 = 3;  pub const EXI_SCD: i32 = 4;  pub const EXI_OUT: i32 = 5;
pub const EXI_SEK: i32 = 6;  pub const EXI_LRG: i32 = 7;  pub const EXI_RED: i32 = 8;
pub const EXI_WRI: i32 = 9;  pub const EXI_MEM: i32 = 10; pub const EXI_ERR: i32 = 20;
pub const JDIFF_VERSION: &str = "0.8.1 (beta) December 2011";
pub const JDIFF_COPYRIGHT: &str = "Copyright (C) 2002-2005,2009,2011 Joris Heirbaut";
pub const MAX_OFF_T: i64 = i64::MAX;
pub const GIPME: [i32; 20] = [134217689, 67108859, 33554393, 16777213, 8388593, 4194301,
    2097143, 1048573, 524287, 262139, 131071, 65521, 32749, 16381, 8191, 4093, 2039, 1021,
    509, 251];
/// Width used by P8zd in release builds (debug builds use 10, see jdebug).
pub fn p8(v: i64) -> String { format!("{:>12}", v) }
/// C `atoi` semantics: skip leading whitespace, optional sign, leading decimal digits,
/// ignore the rest; 0 when no digits. Operates on the lossy form of the OsStr.
pub fn c_atoi(s: &std::ffi::OsStr) -> i32;

// jdebug.rs
pub struct JDebug;                    // grows flags in Task 11
impl JDebug {
    thread_local! { pub static STDDBG_IS_STDOUT: std::cell::Cell<bool> } // or a static AtomicBool
}
pub fn stddbg() -> Box<dyn std::io::Write>;  // stderr or stdout per -do/-d; callers write+flush per line
```
(Implement `stddbg()` concretely as an enum target with a `print` helper — e.g. `pub fn dbg_out() -> std::io::Result<Box<dyn Write>>`; simplest exact-behavior choice: a `static DBG_TO_STDOUT: AtomicBool` plus `pub fn eprint_dbg(args: fmt::Arguments)` that writes to stderr or stdout and flushes.)

- [ ] **Step 1: Scaffold.** `git init`; write `Cargo.toml`:

```toml
[package]
name = "jojodiff-cli-rs"
version = "0.8.1"
edition = "2024"
rust-version = "1.85"
license = "GPL-3.0-or-later"
description = "Independent Rust port of JojoDiff 0.8.1: byte-compatible jdiff and jptch binary diff/patch CLIs (not affiliated with the jojodiff crate)"

[lib]
name = "jojodiff_cli_rs"

[[bin]]
name = "jdiff"
path = "src/bin/jdiff.rs"

[[bin]]
name = "jptch"
path = "src/bin/jptch.rs"

[features]
default = []
debug = []

[dependencies]
# none — std only

[dev-dependencies]
pretty_assertions = "1.4"
jojodiff = "0.1.2"   # francisdb crate, cross-validation only (tests/oracle.rs)
```

`.gitignore` = `/target\n`. Copy pristine C++ tree: `git clone https://github.com/vibhorkalley/jojodiff reference/jojodiff-cpp && rm -rf reference/jojodiff-cpp/.git`. Copy its `COPYING` to `LICENSE`. Create empty `src/lib.rs` (`pub mod defs; pub mod jdebug;` only — later tasks add modules). Stub `src/bin/jdiff.rs` and `src/bin/jptch.rs` with `fn main() {}` so `cargo build` passes. In tests, import pretty_assertions explicitly — `use pretty_assertions::{assert_eq, assert_ne};` — never `use pretty_assertions::*;` (glob import conflicts with the prelude macros and trips clippy).

- [ ] **Step 2: Write failing tests** (inline in `src/defs.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn constants_match_cxx() {
        assert_eq!(EOB, -2);
        assert_eq!((ESC, MOD, INS, DEL, EQL, BKT), (0xA7, 0xA6, 0xA5, 0xA4, 0xA3, 0xA2));
        assert_eq!(EXI_ERR, 20);
        assert_eq!(GIPME.len(), 20);
        assert_eq!(GIPME[19], 251);
    }
    #[test] fn atoi_semantics() {
        assert_eq!(c_atoi("128"), 128);
        assert_eq!(c_atoi("  -42abc"), -42);
        assert_eq!(c_atoi("abc"), 0);
        assert_eq!(c_atoi("1"), 1);          // -m 1 → 1/2*1024 = 0
        assert_eq!(c_atoi(""), 0);
    }
    #[test] fn width_formatting() {
        assert_eq!(p8(0), "           0");
        assert_eq!(p8(57751), "       57751");
    }
}
```

- [ ] **Step 3: Run `cargo test`** — fails (functions undefined). Implement `c_atoi` (byte-wise over `to_string_lossy().bytes()`, i64 accumulator clamped like C — values here are small; clamp to `i32::MAX/MIN` saturate is fine and documented), `p8`, constants, `jdebug.rs` with `DBG_TO_STDOUT: AtomicBool` + `pub fn dbg_print(args: std::fmt::Arguments)` writing to the chosen stream with flush.
- [ ] **Step 4: `cargo test` passes; `cargo build` passes.**
- [ ] **Step 5: Commit** `git add -A && git commit -m "feat: scaffold jojodiff-cli-rs, defs module, vendored C++ reference (GPLv3)"`

---

### Task 2: `JFile` trait + in-memory reader (`-m 0`)

**Files:**
- Create: `src/jfile/mod.rs`, `src/jfile/mem.rs`; Modify: `src/lib.rs`
- Test: `src/jfile/mem.rs` inline

**Interfaces:**
- Produces: `pub enum ReadType { Read, HardAhead, SoftAhead }` (discriminant values 0/1/2 — only used for dispatch, never serialized);
  `pub trait JFile { fn get(&mut self, pos: i64, typ: ReadType) -> i32; fn seekcount(&self) -> i64; }`
  `pub struct JFileMem { .. }` + `impl JFileMem { pub fn new(data: Vec<u8>) -> Self }` (spec §9).

- [ ] **Step 1: Failing tests** — sequential reads return bytes 0,1,2…; `get(len, _) == EOF`, `get(len+100, _) == EOF`; **never** returns `EOB` for any `ReadType`; NUL bytes survive; `seekcount()` stays 0 during a sequential run and increments once per out-of-order `get`; negative `pos` returns `EOF`.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{EOF, ReadType};
    #[test]
    fn seq_nul_eof_seekcount() {
        let mut f = JFileMem::new(vec![0u8, 1, 0, 3, 0xA7]);
        assert_eq!(f.get(0, ReadType::Read), 0);
        assert_eq!(f.get(1, ReadType::Read), 1);
        assert_eq!(f.get(2, ReadType::SoftAhead), 0);      // soft never EOBs
        assert_eq!(f.seekcount(), 0);
        assert_eq!(f.get(4, ReadType::Read), 0xA7);
        assert_eq!(f.seekcount(), 1);                      // jumped 2→4
        assert_eq!(f.get(5, ReadType::Read), EOF);
        assert_eq!(f.get(-1, ReadType::Read), EOF);
    }
}
```

- [ ] **Step 2: Verify failure** (`cargo test jfile`), then implement:

```rust
pub struct JFileMem { data: Vec<u8>, pos_inp: i64, seeks: i64 }
impl JFileMem {
    pub fn new(data: Vec<u8>) -> Self { JFileMem { data, pos_inp: 0, seeks: 0 } }
}
impl JFile for JFileMem {
    fn get(&mut self, pos: i64, _typ: ReadType) -> i32 {
        if pos != self.pos_inp { self.seeks += 1; }
        self.pos_inp = pos + 1;
        if pos < 0 || pos as usize >= self.data.len() { EOF } else { self.data[pos as usize] as i32 }
    }
    fn seekcount(&self) -> i64 { self.seeks }
}
```

- [ ] **Step 3: `cargo test` green. Commit** `feat: JFile trait and in-memory reader (-m 0 semantics)`

---

### Task 3: `JFileAhead` — buffered look-ahead reader (default mode)

**Files:**
- Create: `src/jfile/ahead.rs`; Modify: `src/lib.rs`
- Test: `src/jfile/ahead.rs` inline (plus fixtures built from `Vec<u8>` via `std::io::Cursor`)

**Interfaces:**
- Consumes: `ReadType`, `EOF/EOB/EXI_SEK` from T1.
- Produces: `pub struct JFileAhead<R: Read + Seek> { .. }` with
  `pub fn new(file: R, fid: &str, buf_sze: i64, blk_sze: i32) -> Self` and the `JFile` impl. Semantics: spec §10 (a faithful port of `reference/jojodiff-cpp/src/JFileAhead.cpp:69-346` — that file is the authority; `JFileIStreamAhead.cpp` is the same algorithm).

- [ ] **Step 1: Failing tests** — cover each behavioral branch with a `Cursor<Vec<u8>>`:

```rust
fn mk(data: Vec<u8>) -> JFileAhead<std::io::Cursor<Vec<u8>>> {
    JFileAhead::new(std::io::Cursor::new(data), "Tst", 1024, 16)
}
#[test] fn sequential_read_uses_fast_path()      // bytes 0..99 in order, seekcount stays 0
#[test] fn soft_ahead_before_buffer_returns_eob() // read to 200, then soft get(0) == EOB (outside buf)
#[test] fn hard_ahead_far_forward_resets()        // get(5000, HardAhead) works, seekcount increments
#[test] fn soft_ahead_far_forward_returns_eob()   // same pos with SoftAhead == EOB, no file seek
#[test] fn eof_at_end_only()                      // get(len) == EOF repeatedly; get(len-1) still returns data
#[test] fn scroll_back_serves_history()           // after reading 0..600 with blk 16/buf 1024, get(5, Read) returns data[5]
#[test] fn wraparound_buffer_integrity()          // buf_sze 64, blk 16, file 256 bytes: read 0..255 then re-read 0..63 correctly
#[test] fn seek_error_returns_neg_exi_sek()       // a failing Seek impl → get returns -EXI_SEK
```

(Write each test with concrete expected byte values from a deterministic pattern, e.g. `(i * 7 + 3) % 256`.)

- [ ] **Step 2: Verify red.** Then implement — full port, following `JFileAhead.cpp` statement by statement:

```rust
use std::io::{Read, Seek, SeekFrom};
use crate::defs::{EOB, EOF, EXI_SEK, ReadType};
use crate::jfile::JFile;

pub struct JFileAhead<R: Read + Seek> {
    file: R, fid: String,
    buf_sze: i64, blk_sze: i64,
    buf: Vec<u8>,
    red_sze: i64, buf_usd: i64,
    ptr_inp: usize, ptr_red: usize,
    pos_inp: i64, pos_red: i64, pos_eof: i64,
    seeks: i64,
}

impl<R: Read + Seek> JFileAhead<R> {
    pub fn new(file: R, fid: &str, buf_sze: i64, blk_sze: i32) -> Self {
        let buf_sze = if buf_sze <= 0 { blk_sze as i64 } else { buf_sze }; // guard; C++ never passes 0 here
        JFileAhead { file, fid: fid.into(), buf_sze, blk_sze: blk_sze as i64,
            buf: vec![0u8; buf_sze as usize], red_sze: 0, buf_usd: 0,
            ptr_inp: 0, ptr_red: 0, pos_inp: 0, pos_red: 0, pos_eof: i64::MAX, seeks: 0 }
    }
    /// fseek+fread equivalent; returns bytes read; Err → i32 sentinel -EXI_SEK via caller.
    fn read_chunk(&mut self, file_pos: i64, idx: usize, want: i64) -> Result<i64, i32> {
        if self.file.seek(SeekFrom::Start(file_pos as u64)).is_err() { return Err(-EXI_SEK); }
        let mut done = 0i64;
        while done < want {
            match self.file.read(&mut self.buf[(idx + done as usize)..(idx + want as usize)]) {
                Ok(0) => break,
                Ok(n) => done += n as i64,
                Err(_) => break,          // fread-error ≡ short read in C++
            }
        }
        Ok(done)
    }
    fn get_frombuffer(&mut self, pos: i64, typ: ReadType) -> i32 { /* port JFileAhead.cpp:103-174 */ }
    fn get_outofbuffer(&mut self, pos: i64, typ: ReadType, seek: i32) -> i32 { /* port :179-346 */ }
}
impl<R: Read + Seek> JFile for JFileAhead<R> {
    fn get(&mut self, pos: i64, typ: ReadType) -> i32 {
        if self.red_sze > 0 && pos == self.pos_red {
            self.pos_red += 1; self.red_sze -= 1;
            let b = self.buf[self.ptr_red] as i32;
            self.ptr_red = (self.ptr_red + 1) % self.buf.len();
            return b;
        }
        self.get_frombuffer(pos, typ)
    }
    fn seekcount(&self) -> i64 { self.seeks }
}
```

Porting notes that must be honored (all from the C++ lines cited in spec §10):
- `get_frombuffer`: before-buffer-but-near (`pos + blk_sze >= pos_inp - buf_usd`) → seek-mode 2; before-buffer-far → seek-mode 1; `pos >= pos_eof` → reset read cursor, `EOF`; `pos >= pos_inp + blk_sze` → seek-mode 1; else seek-mode 0. `typ == SoftAhead && seek != 0` → `EOB` **before** any file access. In-buffer backward read computes the wrapped index, sets `pos_red/ptr_red`, and `red_sze = if ptr_red > ptr_inp { buf_sze - ptr_red } else { pos_inp - pos_red }`.
- `get_outofbuffer` mode 2's four placement cases exactly as spec §10 lists them (pointer `lp` and count `todo`), the post-read repair on partial reads, and the extra seek back to `pos_inp` (and `seaks++` again) on full scroll-back reads.
- After modes 0/1: `pos_inp += done; ptr_inp += done;` wrap `== len → 0`; `> len → eprint "Buffer out of bounds on position {pos})!" and process::exit(6)` (exact C++ text incl. the stray `)`); `buf_usd = min(buf_usd + done, buf_sze)`; `red_sze += done`; `if ptr_red == buf.len() { ptr_red = 0 }`.
- End of both C++ `get_outofbuffer` paths: `return self.get(pos, typ)` (tail recursion).
- Partial read sets `pos_eof = file_pos + done`; `done == 0` returns `EOF` immediately.

- [ ] **Step 3: `cargo test` green — every branch test passing.**
- [ ] **Step 4: Commit** `feat: JFileAhead buffered look-ahead reader (port of JFileAhead.cpp)`

---

### Task 4: `JHashPos` — sample hash table

**Files:**
- Create: `src/jhashpos.rs`; Modify: `src/lib.rs`
- Test: inline + integration snippet in doc comment

**Interfaces:**
- Produces (spec §7):
```rust
pub struct JHashPos { /* tbl_pos: Vec<i64>, tbl_hsh: Vec<u32>, prime: i32, size_bytes: i32,
                         col_max: i32, col_cnt: i32, rlb: i32, load_cnt: i32, hits: i32 */ }
impl JHashPos {
    pub fn new(requested: i32) -> Self;
    pub fn hash(&self, byte: i32, cur: &mut u32);                    // *cur = *cur*2 + byte (wrapping)
    pub fn add(&mut self, key: u32, pos: i64, eql_cnt: i32);
    pub fn get(&mut self, key: u32, pos: &mut i64) -> bool;          // exact-key, hits++
    pub fn reliability(&self) -> i32;
    pub fn hash_prime(&self) -> i32;
    pub fn hash_size_bytes(&self) -> i32;                            // prime * 12
    pub fn hash_colmax(&self) -> i32;
    pub fn hash_hits(&self) -> i32;
}
pub const COLLISION_THRESHOLD: i32 = 4; pub const COLLISION_HIGH: i32 = 4; pub const COLLISION_LOW: i32 = 1;
```

- [ ] **Step 1: Failing tests** (port the numbers, they are all forced by the C++):

```rust
#[test] fn prime_selection() {              // JHashPos.cpp:58-61 with loop bound 19
    assert_eq!(JHashPos::new(8*1024*1024).hash_prime(), 8388593);
    assert_eq!(JHashPos::new(8388593).hash_prime(), 8388593);
    assert_eq!(JHashPos::new(8388592).hash_prime(), 4194301);
    assert_eq!(JHashPos::new(1).hash_prime(), 251);        // floor
    assert_eq!(JHashPos::new(0).hash_prime(), 251);
    assert_eq!(JHashPos::new(i32::MAX).hash_prime(), 134217689);
}
#[test] fn hash_wraps_u32() { let mut h=0u32; let mut k=0u32; for b in 0u32..300 { JHashPos::new(251).hash(b as i32,&mut h); k=k.wrapping_mul(2).wrapping_add(b); } assert_eq!(h,k); }
#[test] fn add_then_get_roundtrip_and_hits() { /* add ~1000 keys (hash 32-byte windows over a Vec), verify get(key) → stored pos, hits counter, unknown key → false */ }
#[test] fn quality_and_load_counters() {
    // high-quality sample (eql<=28) stores on first add (col_cnt 4+4 ≥ col_max 4)
    // low-quality (eql=32): 4+1=5 ≥ 4 stores; then verify overwrite semantics on same bucket
    // after `prime` adds, col_max → 8 and rlb → 52 (load counter rollover)
}
#[test] fn reliability_grows_by_4() { /* initial 48; drive load rollovers; assert 52, 56, ... */ }
```

- [ ] **Step 2: Red → implement** `new` (prime loop `while idx < 19 && GIPME[idx] > requested`), `add`/`get` exactly per spec §7 with `idx = (key % prime as u32) as usize`.
- [ ] **Step 3: Green. Commit** `feat: JHashPos sample hashtable (port of JHashPos.cpp)`

---

### Task 5: `JMatchTable` — match selection

**Files:**
- Create: `src/jmatchtable.rs`; Modify: `src/lib.rs`
- Test: inline (uses `JFileMem` from T2 as the two files, `JHashPos` from T4)

**Interfaces (spec §8):**
```rust
pub struct JMatchTable { nodes: Vec<Node>, buckets: [Option<usize>; 127], free: Option<usize>,
                         gld: Option<usize>, gld_delta: i64 }        // + Node{next,cnt,typ,beg,new,org,delta}
pub static HSH_RPR: std::sync::atomic::AtomicI32;                    // siHshRpr
impl JMatchTable {
    pub fn new() -> Self;
    pub fn add(&mut self, fnd_org: i64, fnd_new: i64, base_new: i64, eql_new: i32) -> i32; // 0/1/2
    pub fn get(&mut self, red_org: i64, red_new: i64, hsh: &JHashPos,
               org: &mut dyn JFile, new: &mut dyn JFile, cmp_all: bool) -> Option<(i64, i64)>;
    pub fn cleanup(&mut self, base_new: i64) -> bool;
}
```
`check` is a private free function
`fn check(org: &mut dyn JFile, new: &mut dyn JFile, pos_org: &mut i64, pos_new: &mut i64, len: i32, soft: bool) -> i32` (spec §8.4).

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn add_returns_and_bucket_math() {
    // add(100, 40, ..) → 1 (space left); delta 60 → bucket 60
    // add with delta -1 → bucket 1 (abs), delta 128 → 128%127=1 → same bucket, chain has 2 nodes
    // fill free list to exhaustion → last successful add returns 0; next returns 0 without adding
}
#[test] fn gliding_match_decrements() {
    // a(100,50), then a(101,52)? delta 51≠49 → not gliding. a(100,50) sets gld_delta=49;
    // a(101,51): delta 50 ≠ 49 → no. a(101,52): delta 49 == gld_delta → returns 2, gld_delta becomes 48
}
#[test] fn colliding_match_enlarges() { // same delta twice (not adjacent) → return 2, cnt increments, typ=1 }
#[test] fn cleanup_frees_old_and_empty() { // node with new < base_new removed → free list non-empty → true }
#[test] fn check_finds_run_and_rewinds() {
    // two JFileMems sharing 40 equal bytes at offset 10 → check(10,10,48,hard) == 0 and
    // positions rewound by 24-equal-run anchor exactly as C++ (pos - eql)
}
#[test] fn check_phase2_mismatch_fails() { // mismatch inside last 24 bytes → 2 }
#[test] fn check_soft_eob_returns_1() { // JFileAhead + SoftAhead beyond buffer → 1; hard-eof → 2 }
#[test] fn get_selects_nearest_verified() { /* build org/new with one 64-byte repeated block; feed adds; assert Some((org,new)) pointing at block start */ }
```

- [ ] **Step 2: Red → implement.** Node pool of 256, intrusive lists via `Option<usize>` indices; port `add` (gliding → colliding-chain → new node; new-node return `free.is_some() as i32`), `get` (full 127-bucket scan, candidate filter, test-position math incl. gliding/colliding branches and negative clamp, `check`, soft-EOF recovery `cmp==1` rules, `cnt--`+`HSH_RPR` on `cmp>=2`, best-accept rule with `FZY=0`), `cleanup` (prev-pointer relink). `HSH_RPR.fetch_add(1,..)` wherever C++ does `siHshRpr++`.
- [ ] **Step 3: Green. Commit** `feat: JMatchTable match selection (port of JMatchTable.cpp)`

---

### Task 6: `JOut` trait + `JOutBin` — binary patch writer

**Files:**
- Create: `src/jout/mod.rs`, `src/jout/bin.rs`; Modify: `src/lib.rs`
- Test: inline against `Vec<u8>` writers

**Interfaces:**
```rust
// jout/mod.rs
#[derive(Default, Clone, Copy)]
pub struct OutStats { pub dta: i64, pub ctl: i64, pub del: i64, pub bkt: i64, pub esc: i64, pub eql: i64 }
pub trait JOut {
    fn put(&mut self, opr: i32, len: i64, org: i32, new: i32, pos_org: i64, pos_new: i64) -> bool;
    fn stats(&self) -> OutStats;
}
// jout/bin.rs
pub struct JOutBin<W: std::io::Write> { /* out: W, stats, opr_cur: i32, eql_cnt: i64,
                                           eql_buf: [i32; 4], out_esc: bool */ }
impl<W: std::io::Write> JOutBin<W> { pub fn new(out: W) -> Self }
```

- [ ] **Step 1: Failing tests** (golden bytes — derive by hand from spec §3/§11.1; these exact vectors are asserted again against the C++ oracle in T12):

```rust
fn run(ops: &[(i32, i64, i32, i32)]) -> Vec<u8> { /* JOutBin over Vec, put(EQL/MOD/INS/DEL/BKT…), final ESC flush */ }
#[test] fn length_codec_tiers() {
    // put_len via public DEL: len 1 → [0]; 252 → [251]; 253 → [252,0]; 508 → [252,255];
    // 509 → [253,1,253]; 65535 → [253,255,255]; 65536 → [254,0,1,0,0];
    // 0x1_0000_0000 → [255,0,0,0,0,1,0,0,0]  (9-byte tier always enabled)
}
#[test] fn esc_escaping_in_data() {
    // MOD data 0x00,0xA7,0x00 → ESC MOD 00 A7 A7 00   (pending ESC + next data byte < BKT flushes ESC as data)
    // MOD data 0xA7,0xA6 → ESC MOD A7 A7 A6           (ESC followed by opcode-range byte → doubled)
    // verify stats.esc / stats.dta counts
}
#[test] fn eql_short_run_becomes_mod() {
    // prev MOD, 3 equal bytes, next MOD → emitted as MOD data (no EQL opcode)
}
#[test] fn eql_long_run_emits_opcode() {
    // 10 equal bytes → ESC EQL <len-1 byte>; stats.eql == 10
}
#[test] fn ins_after_mod_switches_opcode_once() { /* opcode emitted only on change */ }
```

- [ ] **Step 2: Red → implement** `put_len`/`put_opr`/`put_byte`/`put` exactly per spec §11.1 (flush condition `eql_cnt > 4 || (opr_cur != MOD && opr != MOD)`; EQL buffering returns `eql_cnt >= 4`).
- [ ] **Step 3: Green. Commit** `feat: JOut trait and binary patch writer (port of JOutBin.cpp)`

---

### Task 7: `JOutAsc` (`-l`) and `JOutRgn` (`-lr`)

**Files:**
- Create: `src/jout/asc.rs`, `src/jout/rgn.rs`; Modify: `src/lib.rs`
- Test: inline

**Interfaces:** `pub struct JOutAsc<W: Write>` / `pub struct JOutRgn<W: Write>` with `new(out: W)`, implementing `JOut`. Formats byte-exact per spec §11.2/§11.3 — octal is **space**-padded width 3 (`{:>3o}`), char rendering `32..=127` else `' '`.

- [ ] **Step 1: Failing tests** with literal expected strings:

```rust
#[test] fn asc_lines() {
    // put(MOD,1,0x65,0x41,7,9)  → "           7            9 MOD 145 101 e-A\n"
    // put(INS,1,-1,0xA7,0,1)    → "           0            1 INS     250  -\x20\n"  (byte 250 → non-printable → ' ')
    // put(DEL,57751,0,0,0,0)    → "           0            0 DEL 57751\n"
    // put(EQL,1,0x20,0x20,3,3)  → "           3            3 EQL  40  40  -\x20\n"
}
#[test] fn rgn_regions() {
    // sequence EQL×300, MOD×8, EQL×79, BKT 57751, EQL×73631, final ESC put → exactly the
    // reference -lr output captured for the text pair (spec §16 fixtures):
    // "           0            0 DEL 57751\n       57751            0 MOD 8\n       57759            8 EQL 1\n
    //  57760            9 MOD 30\n       57790           39 EQL 79\n       57869          118 BKT 57751\n
    //  118          118 EQL 73631\n"
}
#[test] fn stats_counters_match_cxx_rules() { /* ctl/dta/del/bkt/eql/esc after scripted sequences */ }
```

- [ ] **Step 2: Red → implement** both classes (Asc: always `false`; Rgn: always `true`, flush-on-change, final region flushed by engine's ESC).
- [ ] **Step 3: Green. Commit** `feat: ASCII (-l) and region (-lr) output writers`

---

### Task 8: `JDiff` engine

**Files:**
- Create: `src/jdiff.rs`; Modify: `src/lib.rs`
- Test: inline, driving the engine over `JFileMem` pairs with a recording `JOut` that logs `(opr,len,org,new)` tuples.

**Interfaces (spec §6):**
```rust
pub struct JDiff<'a> {
    org: Box<dyn JFile + 'a>, new: Box<dyn JFile + 'a>, out: Box<dyn JOut + 'a>,
    hsh: JHashPos, mch: JMatchTable,
    verbose: i32, src_bkt: bool, mch_max: i32, mch_min: i32, ahd_max: i64, cmp_all: bool,
    src_scn: i32,
    az_org: i64, az_new: i64, hsh_org: u32, hsh_new: u32,
    val_org: i32, val_new: i32, eql_org: i32, eql_new: i32, hsh_err: i32,
}
impl<'a> JDiff<'a> {
    pub fn new(org: Box<dyn JFile>, new: Box<dyn JFile>, out: Box<dyn JOut>,
               hsh_sze: i32, verbose: i32, src_bkt: bool, src_scn: bool,
               mch_max: i32, mch_min: i32, ahd_max: i64, cmp_all: bool) -> Self;
    pub fn jdiff(&mut self) -> i32;                       // 0 / negative EXI code
    pub fn hash(&self) -> &JHashPos;                      // stats getters used by main
    pub fn hsh_err(&self) -> i32;
}
fn fnd_ahd_get(file: &mut dyn JFile, pos: i64, val: &mut i32, eql: &mut i32);  // spec §6.3
```
Internal: `fn uf_put_eql(&mut self, pos_org: i64, pos_new: i64)`, `fn uf_fnd_ahd(&mut self, red_org: i64, red_new: i64, skp_org: &mut i64, skp_new: &mut i64, ahd: &mut i64) -> i32`, `fn uf_fnd_ahd_scn(&mut self) -> i32` — direct ports of spec §6.1/§6.2/§6.4. Field-disjoint borrows (`let Self { org, new, hsh, mch, .. } = self;`) make the C++ aliasing legal in Rust.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn identical_files_emit_nothing()       // ops == [], ret 0 (exit-1 case is main's job)
#[test] fn pure_insert()                        // "" → "abc": ops = [INS a, INS b, INS c]
#[test] fn pure_delete()                        // "abc" → "": one DEL len 3
#[test] fn modify_run()                         // "hello world" ×3 vs "hello world hello XYZlo world hello": MOD bytes for XYZ region (matches the /tmp reference behavior verified during research)
#[test] fn repeated_block_produces_bkt()        // new = org-block duplicated → at least one BKT op in stream
#[test] fn small_shift_produces_del_or_ins()    // new = org with 100 bytes removed at offset 500 → DEL near 500
#[test] fn engine_error_propagates()            // FailingJFile (get returns -EXI_RED) → jdiff() == -EXI_RED
```

- [ ] **Step 2: Red → implement** the four methods line-by-line from spec §6 (loop structure, `lzAhd` guard branch `lbFnd && lzAhd == 0 → lzAhd = 32`, DEL/BKT/INS skip execution, prescan dots at `verbose>0`, the whole `uf_fnd_ahd` budget/backtrack math, `ahd_max = max(ahd_max, 1024)` in `new`).
- [ ] **Step 3: Green — and cross-check by hand**: for the `modify_run` fixture the engine ops must reproduce the reference patch bytes `a7 a3 11 a7 a6 58 59 5a 6c 6f 20 77 6f 72 6c 64 20 68 65 6c 6c 6f 0a` seen from the C++ tool during research.
- [ ] **Step 4: Commit** `feat: JDiff engine (port of JDiff.cpp)`

---

### Task 9: `jdiff` CLI binary

**Files:**
- Create: full `src/bin/jdiff.rs`
- Test: `tests/roundtrip.rs` (jdiff side)

**Interfaces:**
- Consumes: everything above; `p8`, `c_atoi`, `JDebug` target, fixtures.
- Produces: executable `jdiff` — CLI spec §4 (options, presets, greeting/help, stats, exit codes).

- [ ] **Step 1: Failing integration tests** (`tests/roundtrip.rs`, run via `cargo test --test roundtrip`; use `std::process::Command` on `env!("CARGO_BIN_EXE_jdiff")`):

```rust
#[test] fn help_exit_code_and_text() {
    // `jdiff -h` → exit 2, stderr contains "Usage: jdiff [options] <original file> <new file> [<output file>]"
    // and "-ff         Try to be faster: no out of buffer compares, nor pre-scanning." and
    // "-m size     Size (in kB) for look-ahead buffer (default 512kB, 0=no buffers)."
}
#[test] fn missing_args_exit_2() { /* 0 or 1 file args → exit 2 */ }
#[test] fn unopenable_org_exit_3_message()  // stderr: "Could not open first file <p> for reading."
#[test] fn unopenable_new_exit_4()          // "Could not open second file <p> for reading."
#[test] fn unopenable_out_exit_5()          // "Could not open output file <p> for writing."
#[test] fn equal_files_exit_1_empty_patch() // fixture pair identical → patch 0 bytes, exit 1
#[test] fn differ_files_exit_0_nonempty_patch()
#[test] fn missing_output_goes_to_stdout()
#[test] fn greeting_matches_reference()     // `-v` on tiny pair: stderr == byte-exact block incl.
                                             // "File adressing is 64 bit (files up to 8388608 TB), samples are 4 bytes."
#[test] fn stats_lines_match_reference()    // `-v`/`-vv` on tiny pair → exact stat lines (spec §4.4 labels, incl. "Hastable size    :")
#[test] fn option_quirks() {
    // `jdiff -m 1 a b out` → in-memory mode (1/2*1024 == 0) — verify via -vv "Lookahead buffers: 0 kb. (0 kb. per file)."
    // `jdiff A B -l` → creates file named "-l" (options only parsed before filenames) — assert file exists
    // `jdiff -s 2048 -vv …` → liHshMbt == 2 (while >1024 divide)
}
```

- [ ] **Step 2: Red → implement `main`.** Structure:

```rust
fn main() { std::process::exit(real_main()); }
fn real_main() -> i32 {
    // 1. args_os → Vec<OsString>; option loop ported from main.cpp:205-315
    //    (first non-option ends parsing and is re-queued; value options consume-if-present;
    //     presets -b/-f/-ff assign unconditionally; -do sets JDebug target to stdout)
    // 2. greeting (spec §4.2) to stddbg when verbose>0||help||nargs<3
    // 3. usage text (spec §4.2, exact lines incl. ", 0=no buffers") when nargs<3||help||verbose>2;
    //    exit(2) when nargs<3||help
    // 4. filenames: argv[1+i], argv[2+i], argv[3+i] or "-"
    // 5. open org → exit 3; new → exit 4; out ("-" → stdout lock, else File create) → exit 5
    // 6. readers: buf_sze > 0 → JFileAhead::new(File, "Org"/"New", buf_sze, blk_sze)
    //             else → read whole file → JFileMem::new(bytes)
    // 7. out object: 0→JOutBin, 1→JOutAsc, 2→JOutRgn over a BufWriter; flush before exit
    // 8. JDiff::new(org,new,out, hsh_mbt*1024*1024, verbose, src_bkt, src_scn!=0,
    //               mch_max, mch_min, if ahd_max==0 {buf_sze} else {ahd_max}, cmp_all)
    // 9. verbose>1 pre-run lines; ret = jd.jdiff(); verbose stats (spec §4.4)
    // 10. error mapping (spec §4.5, messages WITHOUT trailing newline) → exit code
    // 11. exit 1 if stats.dta==0 && stats.del==0 else 0
}
```

All printed strings byte-exact from spec §4.2–§4.4 (copy them from `reference/jojodiff-cpp/src/main.cpp:318-383,539-567,589-608` verbatim).

- [ ] **Step 3: Green. Commit** `feat: jdiff CLI (option parsing, greetings, stats, exit codes)`

---

### Task 10: `jptch` CLI binary + decoder

**Files:**
- Create: full `src/bin/jptch.rs`
- Test: extend `tests/roundtrip.rs`

**Interfaces:** executable `jptch` per spec §5.

- [ ] **Step 1: Failing tests:**

```rust
#[test] fn roundtrip_all_option_sets() {
    // for each (pair, opts) in [default, -f, -ff, -b, -s 1, -bs 512, -m 64, -m 0, -l, -lr]:
    //   jdiff opts A B p → jptch A p out → cmp B out byte-equal
    //   (-l/-lr produce listings; patch them from a default-run patch instead)
}
#[test] fn stdin_dash_variants()     // `jptch - org patch out`, `jptch org - out`, `jptch org patch -`
#[test] fn verbose_lines_match_reference()  // `-vv` on tiny pair reproduces spec §5.1 lines incl. trailing "EOF" without newline
#[test] fn esc_escaped_data_roundtrip()     // patch containing A7 A7 data sequences applies exactly
#[test] fn garbage_after_ops_ignored()      // bytes while liOpr==ESC-initial are dropped (C++ behavior, spec §15.7)
#[test] fn truncated_length_is_faithful()   // patch [ESC,DEL,252] at EOF → ufGetInt yields 253+(-1)=252, no error (C++ semantics)
#[test] fn exit_codes()                     // -h → 2; missing files → 3/4/5; success → 0 always
```

- [ ] **Step 2: Red → implement.** Port `jpatch.cpp` wholesale: option loop (`-v -vv -vvv -d -h -t`), greeting (`File adressing is 64 bit.`), usage, `"-"` handling (org/patch stdin read fully into `Cursor`, out stdout lock), `uf_get_int` (byte-or-EOF `i32` arithmetic identical to C++), the `jpatch()` loop with `lz_mod`/`lb_esc`/`lb_chg` state, EQL block copies (`BLKSZE = 4096`), seek errors → **stderr** messages + exit 6/8/9 per spec §5.1, verbose prints with `p8` widths, final `EOF` line (verbose>1, no newline). Original-file positions tracked by real `SeekFrom::Current` seeks + an `org_pos` mirror for printing.
- [ ] **Step 3: Green. Commit** `feat: jptch CLI and patch decoder (port of jpatch.cpp)`

---

### Task 11: `debug` feature (parity with `-D_DEBUG` builds)

**Files:**
- Modify: `src/jdebug.rs`, `src/bin/jdiff.rs`, `src/jdiff.rs`, `src/jhashpos.rs`, `src/jmatchtable.rs`, `src/jfile/ahead.rs`, `src/jout/*` (no-op), `Cargo.toml` (feature already declared)
- Test: inline `#[cfg(feature = "debug")]` tests

**Interfaces:** `JDebug` gains `pub flags: [bool; 16]` (global, `OnceLock<Mutex<[bool;16]>>` or plain `static Mutex`), flag indices `DBGHSH..DBGDST` per spec §14; `-dhsh`…`-ddst` options accepted only under the feature (in default builds they fall through to the non-option branch, exactly like the release C++ build).

- [ ] **Step 1: Failing feature test:** `cargo test --features debug` asserting e.g. `-dbuf` produces a line starting `ufFabOpn(Tst):(buf=` and `-dmch` produces `Mch Add (` on the debug stream for a tiny run (widths now `{:>10}` — `p8` switches on the feature via `cfg!`).
- [ ] **Step 2: Implement** every debug print site enumerated in spec §14 with its exact format string (the site list there is exhaustive — it was built by grepping every `#if debug` in the vendored tree). Pointer `%p` values are inherently non-reproducible; print the Rust object address equivalently and document in the module doc-comment that only format/shape is guaranteed.
- [ ] **Step 3: `cargo test` (both with and without feature) green. Commit** `feat: debug feature with -d* flags (parity with _DEBUG builds)`

---

### Task 12: Oracle harness, golden fixtures, CI, docs, scripts

**Files:**
- Create: `scripts/build-oracle.sh`, `scripts/gen-golden.sh`, `scripts/runtest.sh`, `tests/oracle.rs`, `tests/fixtures/**`, `README.md`; Modify: `.github/workflows/ci.yml` (full matrix)

**Interfaces:** `tests/oracle.rs` auto-skips when neither `$JOJODIFF_ORACLE` nor `target/oracle/jdiff` exists, so the 3-OS matrix doesn't need g++.

- [ ] **Step 1: `scripts/build-oracle.sh`** — clone-or-copy pristine vendored tree to `target/oracle-src/`, apply exactly this patch (the fix validated during research; spec §15.1):

```python
# insert after the pthread_join(threadOrg, ...) block in src/main.cpp:
#   liFilOrg->clear(); liFilOrg->seekg(0);
#   liFilNew->clear(); liFilNew->seekg(0);
```
then `mkdir -p bin && make all` (note: the Makefile ignores `main.cpp` changes — `rm -f jdiff jptch` before `make`), binaries land in `target/oracle/`.

- [ ] **Step 2: `scripts/gen-golden.sh`** — for both bundled pairs × option matrix (spec §13): run oracle `jdiff`, store `tests/fixtures/golden/<pair>/<optset>.jdf` (and `.asc/.rgn` for `-l`/`-lr`), plus `-v`/`-vv` stderr captures for the text pair. Commit goldens.

- [ ] **Step 3: `tests/oracle.rs`** —
  1. Byte-compare Rust `jdiff` output vs every golden (patch/listing/stats).
  2. If live oracle present: byte-compare both binaries across the live matrix, and Rust `jptch` output vs C++ `jptch` output for every golden patch.
  3. Round-trip gate per spec §16.1.
  4. francisdb cross-validation (spec §16.4): call `jojodiff::patch(&mut org, &mut patch, &mut out)` — v0.1.2's signature is `pub fn patch<R: Read + Seek, W: Write>(in_reader: &mut R, patch_reader: &mut R, out_writer: &mut W) -> io::Result<()>`, i.e. **both readers must be the same concrete type** (use two `Cursor<Vec<u8>>`); apply Rust-produced patches of the text pair (NUL-free) and assert output == new file. (API from the crate's source; not indexed in Context7.)
- [ ] **Step 4: CI** — concrete workflow (toolchain usage per dtolnay/rust-toolchain docs via Context7: the `@rev` selects the toolchain, components via input):

```yaml
name: ci
on: [push, pull_request]
jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4        # or current major at implementation time
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --all --check
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test --all-features
  oracle:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: sudo apt-get update && sudo apt-get install -y g++ make
      - run: scripts/build-oracle.sh
      - run: cargo test --all-features
        env:
          JOJODIFF_ORACLE: target/oracle
```
- [ ] **Step 5: `README.md`** — usage mirrored from the C++ readme (jdiff/jptch invocations, option table, zip/tar workflow advice), port-fidelity notes and the seven documented deviations (spec §15), oracle instructions, plus a mandatory **"Relationship to other projects"** section near the top:

```markdown
## Relationship to other projects

`jojodiff-cli-rs` is an independent, complete Rust port of JojoDiff 0.8.1 by Joris Heirbaut
(GPLv3, https://sourceforge.net/projects/jojodiff/), via the v0.8.1 C++ class rewrite.

It is **not affiliated with, endorsed by, or derived from** the `jojodiff` crate /
[francisdb/jojodiff-rs](https://github.com/francisdb/jojodiff-rs) — a separate MIT-licensed
library that only *applies* patches. That crate is used solely as an optional cross-validation
consumer in this project's test suite. The original author's GPLv3 work is credited as the
source of the algorithm, wire format, and test data.
```

  Repo About box: "Independent Rust port of JojoDiff — byte-compatible jdiff & jptch CLIs for Windows, Linux & macOS (not affiliated with the `jojodiff` crate)"; topics: `jojodiff`, `binary-diff`, `patch`, `cli`, `rust`. `scripts/runtest.sh` ports `Makefile:runtest` + `run.sh` (`jdiff -m 0 A B > p; jptch A p > patched; md5sum/… compare`) cross-platform via the Rust binaries.
- [ ] **Step 6: Full local gate:** `scripts/build-oracle.sh && JOJODIFF_ORACLE=target/oracle cargo test --all-features` — all green.
- [ ] **Step 7: Commit** `test: oracle conformance harness, golden fixtures, 3-OS CI, docs`

---

# PART II — Tasks 13–22: the 0.8.5 re-target (spec Part II)

**Transition rules (bind every task below):**

- The authority is `reference/jojodiff-0.8.5/src/*` (cited `file:line`); spec Part II
  (§17–§22) is the map; `docs/superpowers/research/2026-10-02-jojodiff-0.8.5-analysis.md`
  is background evidence (never authoritative over the C++).
- Each task keeps `cargo test` (with and without `--features debug`) green at its commit.
  When a task intentionally changes bytes/behavior, it updates the tests that asserted
  the old 0.8.1 expectations **in the same task** (RED first: change the expectation,
  watch it fail, then implement). The full oracle regen and the 0.8.1→0.8.5
  cross-version suite land in Task 22; until then, live-oracle comparisons that 0.8.5
  invalidates are expected to fail and are skipped with a `// TODO(T22)` marker — never
  deleted silently.
- The historical 0.8.1 goldens under `tests/fixtures/golden/` are NEVER regenerated or
  deleted (they become Task 22's cross-version corpus).

### Task 13: 0.8.5 `defs` delta + 0.8.5 oracle build

**Files:**
- Modify: `src/defs.rs`, `Cargo.toml`, `scripts/build-oracle.sh`
- Test: `src/defs.rs` inline

**Interfaces:** version/copyright strings `"0.8.5 (beta) 2020"` /
`"Copyright (C) 2002-2020 Joris Heirbaut"`; `EXI_OK=0, EXI_DIF=1, EXI_EQL=2, EXI_ARG=-2,
EXI_FRT=-3, EXI_SCD=-4, EXI_OUT=-5, EXI_SEK=-6, EXI_LRG=-7, EXI_RED=-8, EXI_WRI=-9,
EXI_MEM=-10, EXI_ERR=-20` (spec §18.D); `MINEQL=2`; drop `GIPME`, add
`pub fn is_prime(n: i32) -> bool` and `pub fn get_lower_prime(n: i32) -> i32` — port
`JDefs.cpp:37-67` **including the exact switch cases** (1024→1021, 32M→33554393,
16M→16777213, 8M→8388593, 128M→134217689, 512M→536870909; else downward isPrime search).
Package version → `0.8.5`.

- [ ] **Step 1: Failing tests:** `get_lower_prime(2097152)==2097143` (the 32 MB default),
  each switch case, `is_prime` edges (0,1,2,even,odd prime/composite),
  `EXI_EQL==2 && EXI_ARG==-2 && EXI_OK==0`, `MINEQL==2`, version strings.
- [ ] **Step 2: Implement; update everything that referenced `EXI_*`/`GIPME`** (compile
  errors surface the list). Note: process exit codes stay 2/3/4/5/6/7/8/9/10/20 — the
  sign flip is internal (`-EXI_*`), CLI mapping lands in Task 20.
- [ ] **Step 3: `scripts/build-oracle.sh` → 0.8.5 oracle:** copy `reference/jojodiff-0.8.5`
  to `target/oracle-src85/`, patch `typedef unsigned long int hkey` → `typedef unsigned
  int hkey` (spec §21.7/§15.12) and build with `-D_FILE_OFFSET_BITS=64` (JDIFF_LARGEFILE
  live) and `make clean` between variants (§21.8); sanity-check: `./jdiff -v` prints
  `0.8.5 (beta) 2020` and `samples are 32 bytes`. Old 0.8.1 oracle script path stays for
  reference (`build-oracle-081.sh` rename is fine).
- [ ] **Step 4: Green both feature modes. Commit** `feat: 0.8.5 defs (version, EXI renumbering, primes) and 0.8.5 oracle build`

### Task 14: `JFile`/`JFileAhead` rework — Append/Reset/Scrollback + sequential streams

**Files:**
- Modify: `src/jfile/mod.rs`, `src/jfile/ahead.rs` (rewrite); `src/jfile/mem.rs` unchanged (pub API, no longer CLI-wired — spec §21.9 note)
- Test: inline in `src/jfile/ahead.rs` (new tests; adapt the T3 tests that encoded 0.8.1 quirks)

**Interfaces (spec §18.E):** JFile trait gains `is_sequential()` and a `getbuf` fast-path
hook; `JFileAhead` rewritten as the port of `JFileAhead.cpp:64-431` over an I/O handle
that may be **non-seekable** (sequential): buffer state `mpInp/miBufUsd/mzPosInp/mzPosBse`
+ read cursor `mzPosRed/miRedSze/mpRed`; `getbuf(pos,&len,eAhead)` public; decision tree
`get_fromfile` (`:269-307`): before-buffer → SoftAhead EOB / sequential HardAhead EOB /
sequential Read SeekError / else Scrollback-vs-Reset (`pos + mlBufSze - miBlkSze >
mzPosInp - miBufUsd`); beyond-buffer → SoftAhead EOB else Reset; else Append with
SoftAhead bounded by `mzPosBse + mlBufSze - miBlkSze`. Reset realigns to block boundary
(sequential variant keeps the tail, `:311-317`); Scrollback (`:341-382`) block-aligned
back, make-room, refil, **seek forward again** (2 seeks), mid-scrollback EOF → ReadError;
`readblocks` (`:392-431`) block-chunked, `miBufUsd` clamped, short read sets EOF. **No
`exit(6)` bounds abort** (0.8.1 §15.8/§10 quirk gone — remove the dead mode-2 arm and the
collapse note). `chkSeq()` seek-EOF probe auto-detect (`JFile.cpp:37-46`). Debug-feature
invariant asserts per `JFileAhead.cpp:240-251` (gated by the feature, spec §18.G).

- [ ] **Step 1: Failing tests** (RED before rewrite; keep a `Cursor` fixture harness):
  sequential stream (a `Read`-only pipe-like mock) serves forward reads with no seek;
  before-buffer Read on sequential → `-EXI_SEK` sentinel; SoftAhead beyond
  `mzPosBse+buf-blk` → EOB without I/O; Scrollback reachable now: read forward 64KB
  (buf 16KB, blk 4KB), then `get(pos_just_before_buffer, Read)` returns correct data and
  seekcount advanced by 2; Reset block-aligns (`mzPosInp % blkSze == 0` observed via
  getBufPos); short-read EOF latching; **T3's `Buffer out of bounds` exit test removed**
  (behavior gone) — replace with clamp/wrap correctness on tiny buffers.
- [ ] **Step 2: Implement** statement-by-statement from the vendored source.
- [ ] **Step 3: Green. Commit** `feat: JFileAhead 0.8.5 rewrite (Append/Reset/Scrollback, sequential streams)`

### Task 15: `JHashPos` 0.8.5 + `hash` with equal-run term

**Files:**
- Modify: `src/jhashpos.rs`; add the hash fn (used by T17) e.g. `src/jdiff.rs` stub or `src/jhashpos.rs` — place per C++ ownership in `JDiff::hash`; keep it a pure function `pub fn hash_key(cur: u32, old: i32, new: i32, eql: &mut i32) -> u32`
- Test: inline (replace T4's 0.8.1 number tests)

**Interfaces (spec §18.E):** `JHashPos::new(mb: i32)` — elements `mb*1024*1024/16` →
`get_lower_prime`; counters count **down** (colMax start 4, store at `<=0`, reset to
colMax; load counts down from prime, rollover `colMax+=4; rlb+=4`); reliability seed
`SMPSZE + SMPSZE/2` = 48; quality gate `eql_cnt <= SMPSZE*2 → COLLISION_HIGH else
COLLISION_LOW` (LOW dead — port as written, spec §21.13); **table zero-initialized**
(spec §21.5 deviation, documented in module doc); `reset()` present, never called;
`print()` unchanged format; `dist()` per `JHashPos.cpp:192-238` (`Overload =
colMax/4 − 1`, guarded Avg/Min/Max/Load). Hash: `if old==new {eql = min(eql+1, SMPSZE)}
else {old=new; eql=0}; (cur*2).wrapping_add(new).wrapping_add(eql)` (`JDiff.cpp:361-371`).

- [ ] **Step 1: Failing tests:** `new(32).hash_prime()==2097143`; switch-case primes;
  down-counter store cadence (first high-quality add stores immediately, resets to
  colMax); load rollover at `prime` adds bumps colMax/rlb by 4; seed 48; hash-vs-0.8.1
  divergence vector (same byte stream, different key once eql enters); hash eql cap at
  SMPSZE.
- [ ] **Step 2: Implement; delete `GIPME`-based tests.** Note `HSH_RPR` global is
  unaffected here (its fate is Task 16).
- [ ] **Step 3: Green. Commit** `feat: JHashPos 0.8.5 (MB sizing, lower primes, down-counters) and eql-aware hash`

### Task 16: `JMatchTable` dynamic rework

**Files:**
- Modify: `src/jmatchtable.rs` (rewrite); `src/jdebug.rs`/`src/test_util.rs` (HSH_RPR global retires)
- Test: inline (adapt T5 tests)

**Interfaces (spec §18.E):** size from `-x` (`miMchSze = max(13, x)`; **`miMchFre`
initialized from the UNCLAMPED x** — spec §21.15); two bucket tables sized
`get_lower_prime(2*sze)`: col on `|delta| % pme`, gld on `org % pme`; node fields
`nxt/col/gld/cnt/gld/beg/new/org/dlt/tst/cmp` with `CMPINV=-1, CMPSKP=-2, CMPEOB=-3`;
new/old aging lists, reuse via `isOld2Reuse` (MAXDST-bounded, `JMatchTable.cpp:733-768`);
incremental best-tracking `isBest()` during `add`/`cleanup`; `getbest()` returns tracked
best + EOB re-evaluation when `!cmpAll`; `cleanup(bseOrg, redNew)` → Full/Invalid/Valid/
Good/Best; instance `mi_hsh_rpr` + `get_hsh_rpr()` replaces the global static (delete
`HSH_RPR` + `hsh_rpr_guard`; update jdebug flag-test lock note — the shared lock may
still serialize GB_DBG, keep it if still needed). Constants `EQLSZE 8, EQLMIN 4, EQLMAX
256, MAXDST 2*1024*1024, MINDST 1024, MAXGLD 128 (dead), FZY 0`. `check()` single-loop
with glide realignment (`azPosOrg -= liEql` on mismatch when gliding) + EQLMAX cap
(`:843-854`).

- [ ] **Step 1: Failing tests:** two-table bucket math (col/gld indices); aging-list
  reuse when table "full" (old element reactivated, not an error); `-x 5` free-count
  quirk (spec §21.15) — deterministic behavior asserted; best-tracking: after adds,
  `getbest` returns the best without a full scan; cleanup return taxonomy; repairs
  counter increments on the instance; glide realignment on mismatched-but-gliding
  compare.
- [ ] **Step 2: Implement from the vendored source; retire `HSH_RPR`/guard.**
- [ ] **Step 3: Green. Commit** `feat: JMatchTable 0.8.5 dynamic table (dual hashing, aging lists, incremental best)`

### Task 17: `JDiff` engine 0.8.5

**Files:**
- Modify: `src/jdiff.rs` (major rework)
- Test: inline (recording-JOut tests; adapt T8)

**Interfaces (spec §18.E):** `int liFnd` live error check (`:277-279`); `search()`
(`:389-718`) replacing `uf_fnd_ahd` — lookahead budget `miAhdMax - (mzAhdNew -
azRedNew)` floored at cached `miRlb`; look-back `miRlb + 2*SMPSZE - 1`; early-terminating
hash re-init (`miEqlNew != liIdx`, `:546-573`); add driven by JMatchTable's return enum
(Full stops; Best/Good shorten to `miRlb`; Valid counts toward mchMin/mchMax with
soft-read switching); miss recovery budget `reliability/2` with `mi_hsh_err` counted in
release too (**wrapping i32**, spec §21.6) and verbose>2
`"\nInaccurate solution at positions %zd/%zd!\n"`; backtrack clamped against
`getBufPos()` when `!src_bkt`; `mz_ahd_org` not reset on backtrack; incremental
source indexing when `src_scn==0` (in-loop + equal-run fast loops `:185-224` + SoftAhead
prescan `:419-447`); `build_full_index` (`:726-793`) with 32 MiB progress marks
(`PGSMRK/PGSMSK`) and verbose>2 `dist(pos,10)`; constructor per `:103-125` (`hsh_sze` in
MB, `mch_min = min(mch_min, mch_max-1)`, `ahd_max = max(ahd_max, 1024)`).

- [ ] **Step 1: Failing tests:** error propagation from search (FailingJFile now
  surfaces via liFnd — no more bool collapse); equal-run indexing path exercised with
  `src_scn=0` on a shifted-block fixture; verbose>2 inaccurate-solution line on a
  crafted repetitive fixture; `mch_min > mch_max-1` clamp; MB-sized hash wiring
  (`hsh_sze=32` behaves like old 32MB).
- [ ] **Step 2: Implement.** Engine-level byte vectors change here only via match
  decisions — **do not** touch JOutBin (Task 18); recording-JOut tests stay op-level.
- [ ] **Step 3: Green. Commit** `feat: JDiff 0.8.5 engine (search, incremental scan, live error paths)`

### Task 18: `JOutBin` implicit-MOD + `MINEQL`; `JOutAsc` hex; `JOutRgn` stats

**Files:**
- Modify: `src/jout/bin.rs`, `src/jout/asc.rs`, `src/jout/rgn.rs`
- Test: inline (update T6/T7 vectors)

**Interfaces (spec §18.C/§18.F):** `JOutBin`: ctor seeds `opr_cur=MOD`; `put_opr`
emits `ESC opr` only when `opr != MOD || opr_cur == INS`; flush condition
`eql_cnt > MINEQL || (opr_cur != MOD && opr != MOD)`; `eql_buf[MINEQL]`. `JOutAsc`:
`%02x` hex (three sites). `JOutRgn`: EQL split on MINEQL; dead `if opr_cur == INS`
inside `case MOD` (ported as written); DEL/BKT `2+put_len` with put_len 1/2/3/4/8
(spec §21.14).

- [ ] **Step 1: Failing tests (the spec §18.C vectors):** hand-pair patch now
  `a7 a3 18 "MODIFIED" a7 a3 0a "XYZ..."` (no `ESC MOD` pairs; 0.8.1 vector minus the
  two `a7 a6`); ≥3-equal run → EQL; 2-equal run inside MOD → MOD data; INS→MOD still
  emits `ESC MOD`; ASC hex line `EQL 48 48 H-H`; RGN ctl accounting per new rules.
- [ ] **Step 2: Implement. Update the golden-bytes tests that encoded 0.8.1 patch
  output** (same-task RED→GREEN per transition rules).
- [ ] **Step 3: Green. Commit** `feat: 0.8.5 output layer (implicit MOD, MINEQL=2, hex listing, region stats)`

### Task 19: `JPatcht` + `JFileOut`

**Files:**
- Create: `src/jpatcht.rs`, `src/jfileout.rs`; Modify: `src/lib.rs`
- Test: inline + `tests/roundtrip.rs` (library-level apply tests)

**Interfaces (spec §18.B/§18.C):** `JPatcht::new(org: &mut dyn JFile, patch: &mut dyn
JFile, out: JFileOut, verbose) -> jpatch() -> i32` — port `JPatcht.cpp` whole: default
operator MOD at sequence start and after `ESC <unknown>` (`:246-254`); `ESC <same-opr>`
inside a run handled as data (`:176-185`); `uf_get_int` tiers incl. live 8-byte form
(non-LARGEFILE reject branch ported as dead code); trailing-byte warning to **stderr**
(`:243`); DEL/EQL/BKT position arithmetic via the JFile readers (EQL copies through
`JFileOut::copyfrom` with its stray-discard fallback byte-loop — spec §21.14); verbose
per-op traces (`:101-107,156-158,166-169,179-182,265-335`). Reads 0.8.1-style explicit
patches (compatibility — spec §22.3 vectors from `tests/fixtures/golden/`).

- [ ] **Step 1: Failing tests:** applies a 0.8.5-style implicit-MOD patch (bytes from
  Task 18's writer); applies a 0.8.1 golden patch and restores exactly; `ESC ESC` /
  `ESC <unknown>` at sequence start → MOD data; truncated-length EOF semantics; warning
  line on trailing byte; verbose traces byte-exact.
- [ ] **Step 2: Implement.** `jptch`/`jdiff -u` wiring is Task 20 — this task is the
  library.
- [ ] **Step 3: Green. Commit** `feat: JPatcht patch applier and JFileOut (port of JPatcht.cpp/JFileOut.cpp)`

### Task 20: CLI rewrite — getopt_long surface, `-u`, argv[0], sequential/stdin, exits

**Files:**
- Rewrite: `src/bin/jdiff.rs`; re-point: `src/bin/jptch.rs`; Create: `src/cli/opts.rs` (std-only getopt_long-equivalent: short string `a:bcd:fhi:jk:lm:n:pqrst::uvx:y`, long table from `main.cpp:238-261`, GNU permutation, `--`, `?`→help-and-continue, missing-arg `-d`→exit 2)
- Modify: `Cargo.toml` if adding the module path
- Test: `tests/roundtrip.rs` (major update)

**Interfaces (spec §18.A/§18.D):** argv[0] dispatch (`jpatch*` → Patch; `jdedup`/
`jtst` routes not ported — spec §21.2); `-j`/`-u`/`-t`/`-y` function options; full
option table of §18.D incl. multiplicative presets in parse order; defaults
(mchMax 128, mchMin 2, hshMbt 32 MB, buf 1MB+1MB, blk 32K) and buffer normalization +
sequential defaults 32/16 MB (`main.cpp:617-620`); `-` = stdin/stdout, both-`-` → exit 2;
sequential auto-detect with the two warnings (`main.cpp:781-795`); greeting/usage blocks
verbatim from `main.cpp:480-602` (stale texts kept — spec §21.10) incl. `-hh` notes;
pre-run echo + post-run stats blocks per §18.F; **exit swap** (identical→0,
differences→1 via `out.dta > 0`) and `exit(-EXI_*)` error paths with exact messages;
`-t` faithful (release: JPatcht fed the destination after diff — corrupt mixed output,
exit per stats; debug feature: getbuf assert → exit 6 — spec §21.3); `-y` → exit 20
(spec §21.4). `jptch` = same parser, function pre-forced to Patch (packaging extra).

- [ ] **Step 1: Failing tests (rewrite roundtrip CLI tests to spec §22.1 matrix):** exit
  swap both ways; `-Z a b c` prints help then diffs (exit 1); `-h a b c` help + diff;
  `- a b` (stdin org) and both-`-` (exit 2); pipe flows (source pipe → `-p` warning +
  round-trip; dest pipe → `-q` warning; `cat p | jdiff -u org -`); argv[0] `jpatch`
  symlink applies patches; `-m 0`/`-m 7`/`-m 2048` echo lines; `-i 1`, `-k 0` clamp;
  `-x 5` runs; `-vv` stats block byte-exact vs the 0.8.5 oracle capture; `-t` release
  output shape; `-y` exit 20; usage text contains `disbale` and `(in KB)` verbatim.
- [ ] **Step 2: Implement.** Copy all literal strings from the vendored `main.cpp` —
  never from memory.
- [ ] **Step 3: Green. Commit** `feat: 0.8.5 CLI (getopt_long surface, -u patch mode, argv[0] dispatch, swapped exits)`

### Task 21: debug feature — `-d <name>` syntax + 0.8.5 site census

**Files:**
- Modify: `src/jdebug.rs`, `src/cli/opts.rs` (flag-name table), engine modules' print sites
- Test: `tests/debug.rs` rewrite

**Interfaces (spec §18.G):** `-d <name>` with names `hsh ahd cmp prg buf hsk ahh bkt
red mch dst` (unknown silently ignored); same 11 flag indices; `hsk/bkt/dst` accepted
with **zero** sites; implement the 0.8.5 site census (JDiff 6 sites, JMatchTable 10,
JFileAhead 2 + always-on invariant asserts, JHashPos 2 — exact format strings from the
vendored lines listed in §18.G); remove 0.8.1-only sites (e.g. JHashPos::hash DBGHSK);
`-vvv` Hash Dist block; `-c` replaces `-do` everywhere (Task 20 parser already routes
it; this task is the print sites + flag plumbing).

- [ ] **Step 1: Failing feature tests:** `-d mch` on a tiny pair emits the new formats
  (`Match Failure at ...`, `Add [ ... ] bse=...`); `-d buf` emits
  `ufFabOpn(Org):(buf=...` with 1 MB size; `-d hsk`/`-d bkt`/`-d dst` produce nothing;
  `-vvv` prints Hash Dist; debug-feature build of `-t` aborts exit 6 with the getbuf
  assert line.
- [ ] **Step 2: Implement sites (pointer `%p` values documented as shape-only).**
- [ ] **Step 3: `cargo test` both modes green. Commit** `feat: 0.8.5 debug surface (-d <name> flags, new site census, invariant asserts)`

### Task 22: 0.8.5 goldens, cross-version suite, CI, docs, release notes

**Files:**
- Create: `tests/crossver.rs`, `tests/fixtures/golden85/**`; Modify: `tests/oracle.rs`, `scripts/gen-golden.sh`, `.github/workflows/ci.yml`, `README.md`, `Cargo.toml` (description mentions 0.8.5)

**Interfaces (spec §22):** `gen-golden.sh` regenerates from the 0.8.5 oracle into
`golden85/` (never overwrites `golden/`); `oracle.rs` byte-compares against `golden85`
across the full §22.1 matrix (live-oracle path identical); `crossver.rs` applies every
`golden/` (0.8.1) patch with the new `jptch` and `jdiff -u` and asserts exact restore;
CI oracle job builds the 0.8.5 oracle (Task 13 script); README: 0.8.5 usage/option
table, **breaking wire-format note** (patches from ≥0.8.5 unreadable by 0.8.1-era
patchers; 0.8.1 patches still apply), `-u`/argv[0] patch modes, `-t` upstream-broken
note, updated deviations (spec §20/§21), version-history section (0.8.1 → 0.8.5
re-target with upstream changelog summary).

- [ ] **Step 1: Failing:** crossver test (old goldens must apply — they will only after
  Tasks 18–20; this task pins them); oracle byte-gate against `golden85` (generate
  first, then assert).
- [ ] **Step 2: Full local gate:** `scripts/build-oracle.sh && JOJODIFF_ORACLE=target/oracle
  cargo test --all-features` — all green; manual pipe/argv[0] smoke per §22.1.
- [ ] **Step 3: Docs/CI/README. Commit** `test: 0.8.5 goldens, cross-version compatibility suite, CI and docs (re-target release)`

---

## Coverage checklist (functionality → task)

| Functionality | C++ source | Task |
|---|---|---|
| Constants/opcodes/exit codes/version strings | JDefs.h | 1 |
| `atoi` CLI semantics, `P8zd` widths | main.cpp:216-250 | 1, 9 |
| Debug destination `-do`/`-d`, debug flags | JDebug.*, main.cpp:283-309 | 1, 11 |
| In-memory reader (`-m 0`) | JFileIStream.cpp | 2 |
| Buffered look-ahead reader (default) | JFileAhead.cpp ≡ JFileIStreamAhead.cpp | 3 |
| Sample hash table (hash/add/get/reliability/primes) | JHashPos.cpp | 4 |
| Match table (add/get/cleanup/check, gliding, repairs) | JMatchTable.cpp | 5 |
| Binary patch writer (length codec, ESC escaping, EQL/MOD batching) | JOutBin.cpp | 6 |
| ASCII listing `-l` | JOutAsc.cpp | 7 |
| Region listing `-lr` | JOutRgn.cpp | 7 |
| Diff engine (main loop, find-ahead, prescan, backtracking) | JDiff.cpp | 8 |
| jdiff CLI (options/presets/greeting/help/stats/exits) | main.cpp | 9 |
| jptch CLI + decoder (ufGetInt, lzMod accounting, stdio `-`, verbose) | jpatch.cpp | 10 |
| Debug-build parity (`-dhsh`…`-ddst`, print sites) | all `#if debug` sites | 11 |
| GPLv3 licensing, tests corpus, oracle equality, 3-OS CI, docs, runtest/gentest scripts | Makefile, run.sh, generate_testFile.sh, tests/ | 1, 12 |

**0.8.5 re-target (spec Part II; source `reference/jojodiff-0.8.5/src/`):**

| Functionality | 0.8.5 C++ source | Task |
|---|---|---|
| Version/EXI/MINEQL constants, isPrime/getLowerPrime, 0.8.5 oracle build | JDefs.h/.cpp, Makefile | 13 |
| Buffer engine rewrite (Append/Reset/Scrollback, readblocks, sequential/chkSeq, asserts) | JFile.* JFileAhead.* JFileAheadStdio/IStream.* | 14 |
| JHashPos 0.8.5 (MB sizing, down-counters, seed, dist) + eql-aware hash | JHashPos.cpp, JDiff.cpp:361-371 | 15 |
| Dynamic match table (dual hash, aging lists, incremental best, EQLSZE family) | JMatchTable.cpp | 16 |
| Engine 0.8.5 (search, buildFullIndex, incremental scan, live liFnd, miRlb) | JDiff.cpp | 17 |
| Output layer 0.8.5 (implicit MOD, MINEQL, hex `-l`, `-r` stats quirks) | JOutBin/JOutAsc/JOutRgn.cpp | 18 |
| Patch applier + patch-phase writer (default-MOD reader, compat, copyfrom) | JPatcht.cpp, JFileOut.cpp | 19 |
| CLI 0.8.5 (getopt_long surface, -u/-j/-t/-y, argv[0], stdin/sequential, exit swap, texts) | main.cpp | 20 |
| Debug 0.8.5 (`-d <name>`, site census, dead flags, invariant asserts, Hash Dist) | all `#if debug` sites | 21 |
| Goldens/cross-version/CI/README/release notes | Makefile, tst/*.sh | 22 |

No C++ file, CLI option, output byte, exit code, or documented behavior is left unassigned;
the only behavioral deltas are the seven spec-§15 deviations (0.8.1) and the fifteen
spec-§21 rulings (0.8.5), each deliberate and documented.

## Context7 documentation audit (2026-10-01)

Every technology the plan touches was checked against Context7 (`mcp__context7__*`); verdicts and
resulting plan changes:

| # | Plan item | Context7 source | Verdict / change |
|---|---|---|---|
| 1 | `edition = "2024"`, `rust-version = "1.85"` | Cargo Book — manifest, rust-version, resolver (`/websites/doc_rust-lang_cargo`) | Confirmed syntax/semantics. New fact encoded: edition 2024 ⇒ default `resolver = "3"` (MSRV-aware, Rust 1.84+); acceptable (dev-deps only). Noted in Global Constraints. |
| 2 | `env!("CARGO_BIN_EXE_jdiff")` in `tests/roundtrip.rs` | Cargo Book — environment variables, `cargo test` | Confirmed: set for integration tests, name is the binary target verbatim; binaries auto-built with the test. No change. |
| 3 | Shared state design: `static HSH_RPR: AtomicI32`, `Mutex<[bool;16]>` / `OnceLock` for debug flags | std docs — `OnceLock`, `keyword.static` | Confirmed: interior-mutable non-`mut` statics are std's recommended pattern; no `static mut` anywhere, so edition-2024's `static_mut_refs` hardening cannot affect us. No change. |
| 4 | stdout as patch sink, stdin as source, relative seeks in jptch | std docs — `Stdout::lock` (`StdoutLock<'static>: Write`), `Stdin::lock` (`Read + BufRead`), `Seek::seek(SeekFrom)` | Confirmed all three usages as written in Tasks 9–10. No change. |
| 5 | CI toolchain installation | dtolnay/rust-toolchain (`/dtolnay/rust-toolchain`) | Prose replaced with concrete YAML: `dtolnay/rust-toolchain@stable` + `components: rustfmt, clippy`; separate ubuntu `oracle` job (Task 12 Step 4). |
| 6 | `pretty_assertions` dev-dependency | **Not indexed in Context7** (matches were unrelated .NET/JS libs) | Version `1.4` kept, sourced from the francisdb crate's own `Cargo.toml` (`1.4.1`, read directly). Usage rule added to Task 1: import `assert_eq`/`assert_ne` explicitly, no glob import. |
| 7 | `jojodiff` v0.1.2 crate (cross-validation) | **Not indexed in Context7** | API recorded in Task 12 Step 3 from the crate's source read directly — notably the single-type-`R` reader constraint (`patch<R, W>(in_reader: &mut R, patch_reader: &mut R, …)`). |

Implementation-time rule: when a task touches a third-party interface, query Context7 for the
pinned version's docs before writing the code; items 6–7 above have no Context7 coverage and are
exempt (their exact sources are vendored/read).

## Self-Review (performed on this plan)

1. **Spec coverage** — the table above maps every spec §2–§16 requirement to a task; §16 acceptance gates are realized in T12 (`tests/oracle.rs`). ✔
2. **Placeholder scan** — implementation steps reference either complete code blocks above or an exact C++ line-range in the vendored tree with a precise behavioral spec section; no "TBD"/"add handling" steps remain. ✔
3. **Type consistency** — `i64` offsets, `i32` byte-or-sentinel, `u32` keys, `ReadType` enum, `JFile`/`JOut` traits, `OutStats`, `HSH_RPR` atomic are used identically in T2–T10. ✔

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-10-01-jojodiff-rs-port.md` (spec: `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`).**

- **Tasks 1–12 (0.8.1): COMPLETE** — executed 2026-10-01 via subagent-driven development, merged to main, CI green; shipped as package version 0.8.1.
- **Tasks 13–22 (0.8.5 re-target): PENDING** — execute with the same process; the same two options apply:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session via executing-plans, batch execution with checkpoints.
