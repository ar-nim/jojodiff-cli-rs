# Memory-Guard Bugfix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the allocation-failure abort (SIGABRT) on huge `-m`/`-x`/`-i`/`-k` option values with a clean exit 10 (`EXI_MEM`) and fix the `-i ≥ 2049` i32 overflow — without changing behavior for any configuration that fits the machine's memory.

**Architecture:** Two independent layers. **Layer A** is a pre-flight gate in `cli::run`: before any file is opened, compute the exact byte footprint of everything the run will allocate (two input buffers, hash table, match table) and refuse with exit 10 plus an actionable breakdown if it exceeds `MemAvailable + SwapFree − 512 MiB` (Linux `/proc/meminfo`; skipped elsewhere or with `JDIFF_UNSAFE_NO_MEMGUARD=1`). **Layer B** converts every big `vec![0; n]` in the three engine constructors to `try_reserve_exact`, so an OS refusal (including on non-Linux and under the escape hatch) becomes `JDiffError::Memory` → exit 10 instead of a Rust allocation abort.

**Tech Stack:** Rust (MSRV 1.85), std only — no new dependencies. Existing dev-dependencies `assert_cmd`, `predicates`, `tempfile` for integration tests.

**Spec:** This plan is self-contained (it embeds the review findings it fixes). The deviation it introduces must be registered in `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` §21 as **§21.19** (Task 8). The byte-contract rule it must honor: *patches, listings, verbose output and exit codes are held byte-identical to the C++ oracle for every input that runs today.*

## Background (why: the two bugs, with evidence)

**Finding 1 — allocation abort.** `c_atoi` (`src/defs.rs:179-213`) deliberately saturates out-of-range numbers at `i32::MAX` (C `atoi` truncates them to `-1`; the saturation is a documented deviation). Nothing downstream bounds the resulting allocations:

- `-m 99999999999999999999` → 2 input buffers × 1 125 899 905 794 048 bytes → observed: `memory allocation of … bytes failed`, **SIGABRT, rc=134**.
- `-x 99999999999999999999` → match nodes `104 × 2 147 483 647` = **223 338 299 288 bytes** (104 = `size_of::<Node>()`; the three `Option<usize>` links are 16 bytes each — `usize` has no niche) plus 2 bucket tables × 8 × 2 147 483 647 → same abort.

The C++ has an `EXI_MEM` exit code (`JDefs.h:146-158`, exit 10, `"\nError allocating memory !\n"`) whose arm is dead code in both the C++ and the port. This plan makes it reachable.

**Finding 2 — i32 overflow.** `src/jhashpos.rs:140` computes `let size_bytes = prime * 12;` in i32. For `-i ≥ 2049` the product exceeds `i32::MAX`: debug-feature builds panic (observed rc=101 at `-i 2049`, `-i 6144`); release builds wrap and the `-vv` statistic prints garbage (observed `-2046Mb` at `-i 2049`, `2048Mb` at `-i 6144`; correct value for 6144 is `6442450908` bytes = 6144Mb). `-i 2048` lands exactly on `i32::MAX` (2147483640) and is fine.

**Verified reference points** (used as test vectors below, cross-checked against the live binary): `get_lower_prime(256)=251`, `get_lower_prime(2048)=2039`, `get_lower_prime(2796202)=2796181`, `get_lower_prime(179044352)=179044297` (`-i 2049`), `get_lower_prime(536870912)=536870909` (`-i 6144`), `NODE_SIZE=104`, default footprint 35 668 652 bytes.

**Out of scope (known, deliberately not fixed here):** diff-path write errors are swallowed (`IgnoringWriter`, oracle-pinned C++ behavior).

**Also fixed by this plan (Task 9) — flag-chain panics.** `-f` takes no argument (only `a: d: i: k: m: n: x:` and optional `t::` do in the optstring), so `-ffffffff` is one getopt *cluster* = eight `-f` flags, like `-vv` is two `-v`s. Upstream designs two levels (`-f`, `-ff`) but never guards the arithmetic: each `-f` does `mch_max /= 2`, so 128→64→32→16→8→4→2→1→**0** at the eighth, the bucket prime becomes 0, and `JMatchTable`'s ctor assert fires (observed rc=101). `-b` chains reach the same assert by wrapping: `mch_max.wrapping_mul(4)` is `i32::MIN` after 12 `-b`s — observed rc=101 with `-bbbbbbbbbbbb -m 8 -i 1` (small buffers reach the ctor; without a small `-m` the chain dies earlier in a giant allocation, which Tasks 5/6 already convert to exit 10). The C++ is UB at every one of these points (`calloc(0)`, then a modulo-by-zero on the first bucket access).

## Global Constraints

- **Byte contract:** no change to any stdout/stderr byte or exit code for configurations that fit available memory. Every existing suite (`cargo test --release`) must stay green after every task.
- **No new dependencies**; `Cargo.toml` unchanged (thiserror 2 / anyhow 1 pinned).
- **No `unsafe`.**
- **MSRV 1.85** — no let-chains (stabilized 1.88).
- **Machine-independence of tests:** integration tests may only rely on *petabyte*-scale option values (they exceed `MemAvailable + SwapFree` on any conceivable machine). Never test the gate with GB-scale values — CI RAM is unknown. Never construct a real `JHashPos::new(2049)` in tests (allocates 2 GB) — test the pure helper instead.
- **Every spawned `jdiff` in tests runs under a timeout** (project lesson: a runaway `jdiff` once filled `/tmp` and killed a session). `assert_cmd` has no timeout; keep fixtures ≤ 32 MiB so runs finish in seconds.
- **Conventional commits** (`feat:`, `fix:`, `test:`, `docs:`, `refactor:`), one per task, on branch `fix/memguard`.
- All shell commands in this plan are prefixed with `rtk` (user's RTK proxy; see AGENTS.md).

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `src/defs.rs` | `fmt_mb` byte-size label | 1 |
| `src/jfile/ahead.rs` | `effective_geometry` (shared buffer arithmetic) + `try_reserve` ctor | 2, 6 |
| `src/jhashpos.rs` | `elements_for_mb`, `size_bytes_for` helpers; i64 `size_bytes`; `try_reserve` ctor | 2, 6, 7 |
| `src/jmatchtable.rs` | `NODE_SIZE`, `mch_pme_for`; `try_reserve` ctor | 2, 6 |
| `src/jdiff.rs` | `JDiff::new` becomes fallible | 6 |
| `src/lib.rs` | `try_zeroed_vec` allocation helper | 6 |
| `src/cli/config.rs` | `MemoryPlan` + `memory_footprint`; post-parse `mch_max` floor | 3, 9 |
| `src/cli/sysmem.rs` (new) | `/proc/meminfo` ceiling, headroom, escape-hatch env | 4 |
| `src/cli/mod.rs` | register `sysmem` module | 4 |
| `src/cli/run.rs` | Layer-A gate + messages; Layer-B note arms | 5, 6 |
| `src/cli/diff_phase.rs` | `JDiff::new?`, i64 stat line | 6, 7 |
| `tests/memguard.rs` (new) | integration tests | 5, 6, 8, 9 |
| spec §21 + release notes | deviation register | 8 |

---

### Task 1: `fmt_mb` helper

**Files:**
- Modify: `src/defs.rs` (add after `print_char`, ~line 226)
- Test: `src/defs.rs` tests module

**Interfaces:**
- Produces: `pub fn fmt_mb(bytes: i64) -> String` — e.g. `fmt_mb(33554172) == "32.0Mb"`. Consumed by Tasks 5 and 6.

- [ ] **Step 1: Write the failing test** — append to `src/defs.rs` `mod tests`:

```rust
    /// Memory-budget labels (`fmt_mb`): one decimal, `Mb` suffix matching
    /// the options' own units (-m/-i are MB).
    #[test]
    fn fmt_mb_labels() {
        assert_eq!(fmt_mb(0), "0.0Mb");
        assert_eq!(fmt_mb(1048576), "1.0Mb");
        assert_eq!(fmt_mb(33554172), "32.0Mb");
        assert_eq!(fmt_mb(2148531564), "2048.0Mb");
        assert_eq!(fmt_mb(1125899905794048), "1073741824.0Mb");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `rtk cargo test --release fmt_mb`
Expected: FAIL — `cannot find function fmt_mb`.

- [ ] **Step 3: Implement** — add to `src/defs.rs` after `print_char`:

```rust
/// Human-readable MiB label for memory-budget messages: one decimal place,
/// `Mb` suffix matching the options' own units (`-m`/`-i` are MB).
pub fn fmt_mb(bytes: i64) -> String {
    format!("{:.1}Mb", bytes as f64 / (1024.0 * 1024.0))
}
```

- [ ] **Step 4: Run the test** — `rtk cargo test --release fmt_mb` → PASS.

- [ ] **Step 5: Commit**

```bash
rtk git add src/defs.rs
rtk git commit -m "feat: fmt_mb memory-size label for budget diagnostics"
```

---

### Task 2: Shared sizing math (no behavior change)

Extracts the size arithmetic the footprint needs, so the gate can never drift from what the constructors actually allocate. **Pure refactor: every existing test must stay green unchanged.**

**Files:**
- Modify: `src/jfile/ahead.rs` (extract `effective_geometry` mirroring `JFileAhead::new` lines 125–147)
- Modify: `src/jhashpos.rs` (extract `elements_for_mb` from `new` line 134-135; add `size_bytes_for`)
- Modify: `src/jmatchtable.rs` (add `NODE_SIZE` const; extract `mch_pme_for` from `new` lines 327-332)

**Interfaces:**
- Produces (all `pub(crate)`):
  - `crate::jfile::ahead::effective_geometry(buf_sze: i64, blk_sze: i32) -> (i64, i64)`
  - `crate::jhashpos::elements_for_mb(mb: i32) -> i32`
  - `crate::jhashpos::size_bytes_for(prime: i32) -> i64`
  - `crate::jmatchtable::NODE_SIZE: usize` (== 104)
  - `crate::jmatchtable::mch_pme_for(mch_sze: i32) -> i32`
- Consumed by: Task 3 (`memory_footprint`), Task 7 (field rewire).

- [ ] **Step 1: Write the failing tests**

`src/jfile/ahead.rs` tests module — the cross-check pins the helper to the constructor's actual behavior (`test_geometry` is added in Step 3):

```rust
    /// `effective_geometry` mirrors `JFileAhead::new`'s buffer arithmetic
    /// exactly (JFileAhead.cpp:44-57): zero-floors and block alignment.
    #[test]
    fn effective_geometry_matches_ctor() {
        assert_eq!(effective_geometry(0, 16), (1024, 16));
        assert_eq!(effective_geometry(1024, 0), (1024, 1));
        assert_eq!(effective_geometry(100, 16), (96, 16));
        assert_eq!(effective_geometry(8, 16), (16, 16)); // shrunk to 0 -> blk
        assert_eq!(effective_geometry(2048, 4096), (4096, 4096));
        // No drift: the constructor produces the same pair for each vector.
        for (buf, blk) in [(0i64, 16i32), (1024, 0), (100, 16), (8, 16), (2048, 4096)] {
            let f = JFileAhead::new(Cursor::new(Vec::new()), "T", buf, blk)
                .expect("tiny alloc")
                .test_geometry();
            assert_eq!(f, effective_geometry(buf, blk), "ctor vs helper ({buf},{blk})");
        }
    }
```

`src/jhashpos.rs` tests module:

```rust
    /// `elements_for_mb` / `size_bytes_for` (JHashPos.cpp:53-61): MB ->
    /// elements with the i32 clamp, then the 12-bytes-per-element footprint.
    #[test]
    fn element_and_size_helpers() {
        assert_eq!(elements_for_mb(32), 2_796_202);
        assert_eq!(elements_for_mb(1), 87_381);
        assert_eq!(elements_for_mb(0), 87_381); // mb < 1 behaves like 1
        assert_eq!(elements_for_mb(-5), 87_381);
        assert_eq!(
            elements_for_mb(i32::MAX),
            i32::MAX // clamped, no overflow
        );
        assert_eq!(size_bytes_for(2_796_181), 33_554_172);
        // -i 2049: the product that overflows i32 (finding 2) — exact here.
        assert_eq!(size_bytes_for(179_044_297), 2_148_531_564);
        assert_eq!(size_bytes_for(536_870_909), 6_442_450_908);
    }
```

`src/jmatchtable.rs` tests module:

```rust
    /// `NODE_SIZE` pins the footprint formula: 3 × Option<usize> (16 bytes
    /// each — usize has no niche) + 2 × i32 + 5 × i64 + CmpVal = 104.
    /// If a Node field ever changes, this assert fails so the memory
    /// budget cannot silently under-count.
    #[test]
    fn node_size_is_104() {
        assert_eq!(NODE_SIZE, 104);
    }

    /// `mch_pme_for` (JMatchTable.cpp:97): bucket prime from the UNCLAMPED
    /// -x value, i64-clamped product (mirrors the ctor exactly).
    #[test]
    fn mch_pme_for_values() {
        assert_eq!(mch_pme_for(64), 127);
        assert_eq!(mch_pme_for(5), 7);
        assert_eq!(mch_pme_for(0), 0); // ctor asserts; helper stays pure
        assert_eq!(mch_pme_for(i32::MAX), 2_147_483_647); // Mersenne prime
    }
```

- [ ] **Step 2: Run to verify failure** — `rtk cargo test --release effective_geometry element_and_size node_size mch_pme` → FAIL (functions not defined).

- [ ] **Step 3: Implement**

`src/jfile/ahead.rs`, module level (above `impl<R: Read + Seek> JFileAhead<R>`):

```rust
/// The buffer arithmetic of `JFileAhead::new` (`JFileAhead.cpp:44-57`),
/// pure so the CLI memory budget can compute the exact allocation without
/// building a reader: zero `buf_sze` -> 1024; zero `blk_sze` -> 1; shrink
/// `buf_sze` down to a block multiple; a zero result grows to `blk_sze`.
pub(crate) fn effective_geometry(mut buf_sze: i64, blk_sze: i32) -> (i64, i64) {
    if buf_sze == 0 {
        buf_sze = 1024;
    }
    let mut blk_sze = i64::from(blk_sze);
    if blk_sze == 0 {
        blk_sze = 1;
    }
    if buf_sze % blk_sze != 0 {
        buf_sze -= buf_sze % blk_sze;
    }
    if buf_sze == 0 {
        buf_sze = blk_sze;
    }
    (buf_sze, blk_sze)
}
```

Do **not** rewire `JFileAhead::new` to call it (its warning prints are byte-pinned and stay verbatim); the cross-check test is the drift guard. Add the test accessor inside `impl<R: Read + Seek> JFileAhead<R>`:

```rust
    /// Test-only view of the constructor's final buffer geometry (the
    /// `effective_geometry` cross-check).
    #[cfg(test)]
    fn test_geometry(&self) -> (i64, i64) {
        (self.buf_sze, self.blk_sze)
    }
```

`src/jhashpos.rs`, module level:

```rust
/// MB -> element count (`JHashPos.cpp:53-59`): `mb * 1024 * 1024 / 12`,
/// `mb < 1` behaves like 1, product computed in i64 and clamped to
/// `i32::MAX` (the C++ int multiply overflows for mb > 2047 — UB there).
pub(crate) fn elements_for_mb(mb: i32) -> i32 {
    let sze: i64 = if mb < 1 { 1 } else { i64::from(mb) };
    (sze * 1024 * 1024 / 12).min(i64::from(i32::MAX)) as i32
}

/// Element count -> table bytes: 12 per element (i64 position + u32 key,
/// spec §21.18). i64 so `-i >= 2049` cannot overflow (finding 2).
pub(crate) fn size_bytes_for(prime: i32) -> i64 {
    i64::from(prime) * 12
}
```

In `JHashPos::new`, replace lines 134-135 (`let sze … let elements …`) with `let elements = elements_for_mb(mb);` and line 140 with `let size_bytes = size_bytes_for(prime) as i32;` (the `as i32` wrap is removed in Task 7 — for now the field stays i32 so behavior is bit-identical).

`src/jmatchtable.rs`: add `use std::mem::size_of;` if needed, then module level:

```rust
/// `size_of::<Node>()` — pinned at 104 by test. The CLI memory budget
/// multiplies this by the (clamped) -x value.
pub(crate) const NODE_SIZE: usize = size_of::<Node>();

/// Bucket prime for a -x value (`JMatchTable.cpp:97`): from the UNCLAMPED
/// `mch_sze * 2`, i64 product clamped to `i32::MAX` (the C++ int multiply
/// overflows for huge -x — UB there). Pure: non-positive results are the
/// caller's (the ctor asserts; the budget clamps).
pub(crate) fn mch_pme_for(mch_sze: i32) -> i32 {
    let two_sze = i64::from(mch_sze) * 2;
    if two_sze > i64::from(i32::MAX) {
        get_lower_prime(i32::MAX)
    } else {
        get_lower_prime(two_sze as i32)
    }
}
```

In `JMatchTable::new`, replace the `two_sze`/`mch_pme` computation (lines 327-332) with `let mch_pme = mch_pme_for(mch_sze);` (the `assert!` on it stays).

- [ ] **Step 4: Run all tests** — `rtk cargo test --release` → everything PASS (new + all existing).

- [ ] **Step 5: Commit**

```bash
rtk git add src/jfile/ahead.rs src/jhashpos.rs src/jmatchtable.rs
rtk git commit -m "refactor: share sizing math (effective_geometry, elements_for_mb, mch_pme_for, NODE_SIZE)"
```

---

### Task 3: `memory_footprint` (the MemoryPlan)

**Files:**
- Modify: `src/cli/config.rs` (add after `size_buffers`)
- Test: `src/cli/config.rs` tests module

**Interfaces:**
- Consumes: Task 2 helpers.
- Produces:

```rust
pub(crate) struct MemoryPlan { pub total: u64, pub buffers: u64, pub index_table: u64, pub match_table: u64 }
pub(crate) fn memory_footprint(opts: &Options, buffers: &Buffers) -> MemoryPlan
```

- [ ] **Step 1: Write the failing tests** — append to `config.rs` tests:

```rust
    /// `memory_footprint` (memguard plan): the exact bytes the engines will
    /// allocate for one diff run. Vectors verified against the live binary
    /// (NODE_SIZE=104; get_lower_prime(256)=251, (2048)=2039).
    #[test]
    fn footprint_default_options() {
        let o = parse(&[argv0("jdiff"), OsString::from("a"), OsString::from("b")]);
        let b = size_buffers(&o);
        let p = memory_footprint(&o, &b);
        assert_eq!(p.buffers, 2 * 1024 * 1024);
        assert_eq!(p.index_table, 33_554_172); // 12 * 2796181 (-i 32)
        assert_eq!(p.match_table, 104 * 128 + 16 * 251); // nodes + buckets
        assert_eq!(p.total, 35_668_652);
    }

    #[test]
    fn footprint_m_2048_and_x_1024() {
        let o = parse(&[
            argv0("jdiff"),
            OsString::from("-m"), OsString::from("2048"),
            OsString::from("-x"), OsString::from("1024"),
        ]);
        let b = size_buffers(&o);
        let p = memory_footprint(&o, &b);
        assert_eq!(p.buffers, 2_147_483_648); // 1 GiB each
        assert_eq!(p.match_table, 104 * 1024 + 16 * 2039);
        assert_eq!(p.total, 2_147_483_648 + 33_554_172 + 104 * 1024 + 16 * 2039);
    }

    #[test]
    fn footprint_saturated_values_overflow_to_u64_max() {
        // -m 99999999999999999999 saturates to i32::MAX MB per buffer:
        // the total must saturate, never wrap or underflow.
        let o = parse(&[
            argv0("jdiff"),
            OsString::from("-m"), OsString::from("99999999999999999999"),
        ]);
        let b = size_buffers(&o);
        let p = memory_footprint(&o, &b);
        assert!(p.total > 1_000_000_000_000_000); // petabyte-scale
    }
```

- [ ] **Step 2: Run** — `rtk cargo test --release footprint` → FAIL (not defined).

- [ ] **Step 3: Implement** — in `src/cli/config.rs` (needs `use crate::jfile::ahead::effective_geometry;` — `jfile` re-exports `ahead` as a public module — plus the Task 2 helper paths):

```rust
/// Memory-budget breakdown in bytes: everything one diff run allocates —
/// the two input buffers (`JFileAhead`), the hashtable (`JHashPos`) and
/// the matching table (`JMatchTable` nodes + two bucket tables). The
/// `-t` patch phase reuses only the two buffers after the diff phase
/// dropped its tables, so this sum is the process peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryPlan {
    pub total: u64,
    pub buffers: u64,
    pub index_table: u64,
    pub match_table: u64,
}

/// The exact allocations the engines will make for `opts`/`buffers`
/// (memguard Layer A). Saturating arithmetic: absurd option values must
/// overflow to `u64::MAX` (gate refuses), never wrap small.
pub(crate) fn memory_footprint(opts: &Options, buffers: &Buffers) -> MemoryPlan {
    let org = effective_geometry(buffers.ll_buf_org, buffers.blk_sze).0.max(0) as u64;
    let new = effective_geometry(buffers.ll_buf_new, buffers.blk_sze).0.max(0) as u64;
    let buf_bytes = org.saturating_add(new);

    let prime_i = get_lower_prime(crate::jhashpos::elements_for_mb(opts.hsh_mbt));
    let index_table = crate::jhashpos::size_bytes_for(prime_i).max(0) as u64;

    let nodes = u64::try_from(crate::jmatchtable::NODE_SIZE)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(opts.mch_max.max(13)).unwrap_or(u64::MAX));
    let buckets = u64::try_from(crate::jmatchtable::mch_pme_for(opts.mch_max).max(0))
        .unwrap_or(u64::MAX)
        .saturating_mul(16);
    let match_table = nodes.saturating_add(buckets);

    let total = buf_bytes
        .saturating_add(index_table)
        .saturating_add(match_table);
    MemoryPlan { total, buffers: buf_bytes, index_table, match_table }
}
```

(`size_buffers`' alignment already aligns `ll_buf_*` to `blk_sze`; `effective_geometry` re-applies the ctor's own adjustment so `-k`-grows-buffer corners cannot be under-counted. The `use crate::defs::get_lower_prime;` import goes at the top of config.rs if not present.)

- [ ] **Step 4: Run** — `rtk cargo test --release footprint` → PASS; then full `rtk cargo test --release` → PASS.

- [ ] **Step 5: Commit**

```bash
rtk git add src/cli/config.rs
rtk git commit -m "feat: memory_footprint plan computation (memguard Layer A input)"
```

---

### Task 4: `cli::sysmem` module

**Files:**
- Create: `src/cli/sysmem.rs`
- Modify: `src/cli/mod.rs` (add `mod sysmem;` next to the other module declarations, ~line 63)
- Test: `src/cli/sysmem.rs` tests module

**Interfaces:**
- Produces (all `pub(crate)`, module private to `cli`):
  - `const MEMGUARD_HEADROOM: u64 = 512 * 1024 * 1024;`
  - `const MEMGUARD_ENV: &str = "JDIFF_UNSAFE_NO_MEMGUARD";`
  - `fn parse_meminfo(text: &str) -> Option<(u64, u64)>` — (MemAvailable_kB, SwapFree_kB)
  - `fn available_anon_bytes() -> Option<u64>` — None off-Linux / unreadable / missing fields
  - `fn memguard_enabled() -> bool` — false iff `MEMGUARD_ENV` is set to any value

- [ ] **Step 1: Write the failing tests** — create `src/cli/sysmem.rs` with the module docs, the tests, and empty/minimal stubs so it compiles:

```rust
//! Machine memory ceiling for the CLI memory guard (plan
//! 2026-10-04-memguard-bugfix, Layer A): anonymous pages the engines touch
//! are served by RAM + swap, so the honest ceiling is `MemAvailable +
//! SwapFree` from Linux `/proc/meminfo` (kB fields × 1024). Off Linux, or
//! when the file/fields are missing, the ceiling is unknown (`None`) and
//! only Layer B (`try_reserve` in the constructors) protects the run.
//!
//! `JDIFF_UNSAFE_NO_MEMGUARD=1` disables Layer A deliberately (the OS
//! overcommit heuristic then decides, exactly like the pre-guard builds).

/// Safety margin below the ceiling: page tables for huge tables, output
/// BufWriters, allocator slack.
pub(crate) const MEMGUARD_HEADROOM: u64 = 512 * 1024 * 1024;

/// Escape hatch: any value set disables the Layer-A pre-flight gate.
pub(crate) const MEMGUARD_ENV: &str = "JDIFF_UNSAFE_NO_MEMGUARD";

/// (MemAvailable, SwapFree) in kB out of a `/proc/meminfo` body; `None`
/// when either field is missing or unparsable.
pub(crate) fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut avail = None;
    let mut swap = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("MemAvailable:") => avail = it.next().and_then(|v| v.parse().ok()),
            Some("SwapFree:") => swap = it.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Some((avail?, swap?))
}

/// Bytes of anonymous memory the machine can back (RAM + swap), or `None`
/// when that cannot be determined (non-Linux, unreadable file).
pub(crate) fn available_anon_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let (avail, swap) = parse_meminfo(&text)?;
    Some(avail.saturating_add(swap).saturating_mul(1024))
}

/// Layer A active? Disabled by setting `JDIFF_UNSAFE_NO_MEMGUARD`.
pub(crate) fn memguard_enabled() -> bool {
    std::env::var_os(MEMGUARD_ENV).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
MemTotal:       16000000 kB
MemFree:          500000 kB
MemAvailable:   12000000 kB
SwapTotal:       4000000 kB
SwapFree:        3000000 kB
";

    #[test]
    fn parses_avail_and_swap() {
        assert_eq!(parse_meminfo(FIXTURE), Some((12_000_000, 3_000_000)));
    }

    #[test]
    fn missing_fields_are_none() {
        assert_eq!(parse_meminfo("MemTotal: 100 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
        // Present but unparsable values are None too.
        assert_eq!(parse_meminfo("MemAvailable: x kB\nSwapFree: 1 kB\n"), None);
    }

    #[test]
    fn memguard_env_disables() {
        // read-only check: unset means enabled
        assert!(std::env::var_os(MEMGUARD_ENV).is_none() || !memguard_enabled());
    }
}
```

- [ ] **Step 2: Register + run** — add `mod sysmem;` to `src/cli/mod.rs`, then `rtk cargo test --release sysmem` → PASS (module created with implementation in the same commit — the TDD red step is satisfied by writing the tests first against stubs if you prefer; the fixture values are pure so no flakiness).

- [ ] **Step 3: Full suite** — `rtk cargo test --release` → PASS.

- [ ] **Step 4: Commit**

```bash
rtk git add src/cli/sysmem.rs src/cli/mod.rs
rtk git commit -m "feat: cli sysmem module (MemAvailable+SwapFree ceiling, guard env)"
```

---

### Task 5: Layer-A gate in `cli::run` (the fix for finding 1 on Linux)

**Files:**
- Modify: `src/cli/run.rs` (gate after `size_buffers` ~line 108; two new fns; imports)
- Create: `tests/memguard.rs`
- Test: `tests/memguard.rs` (integration, follows the repo's assert_cmd harness)

**Interfaces:**
- Consumes: Task 1 `fmt_mb`, Task 3 `MemoryPlan`/`memory_footprint`, Task 4 sysmem fns.
- Produces: `fn print_memguard_refusal(plan: &config::MemoryPlan, avail: u64)` and `fn mb(u64) -> String` (private to run.rs; Task 6 reuses them).

- [ ] **Step 1: Write the failing integration tests** — create `tests/memguard.rs`:

```rust
//! Memory-guard integration tests (plan 2026-10-04-memguard-bugfix):
//! absurd option values must exit 10 (EXI_MEM) with the refusal message
//! instead of aborting (SIGABRT). All refusal vectors are petabyte-scale
//! (saturated c_atoi values), so they exceed MemAvailable + SwapFree on
//! any conceivable machine — the tests are machine-independent.

use assert_cmd::Command;
use predicates::boolean::PredicateBooleanExt;
use predicates::str::contains;

fn jdiff() -> Command {
    Command::cargo_bin("jdiff").unwrap()
}

/// Fixture pair: 17 bytes differing in the tail (roundtrip.rs pair).
fn fixtures(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    std::fs::write(&a, b"hello world hello").unwrap();
    std::fs::write(&b, b"hello world byebye").unwrap();
    (a, b)
}

#[test]
fn absurd_buffer_size_exits_10_with_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .args(["-m", "99999999999999999999", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("available (RAM + swap)"))
        .stderr(contains("JDIFF_UNSAFE_NO_MEMGUARD=1"))
        .stderr(contains("Error allocating memory !"))
        .stderr(contains("buffers (-m)"));
}

#[test]
fn absurd_buffer_and_search_max_exits_10() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .args(["-m", "99999999999999999999", "-x", "99999999999999999999", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("match table (-x)"))
        .stderr(contains("Error allocating memory !"));
}

#[test]
fn sane_sizes_still_run_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    let patch = dir.path().join("ok.patch");
    // Exit 1 = differences found, the normal 0.8.5 swap.
    jdiff()
        .args(["-m", "64", "-i", "8", "-x", "256", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(&patch)
        .assert()
        .code(1);
    let out = dir.path().join("out.bin");
    jdiff()
        .args(["-u"])
        .arg(&a)
        .arg(&patch)
        .arg(&out)
        .assert()
        .code(0);
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&b).unwrap());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `rtk cargo test --release --test memguard`
Expected: first two tests FAIL (rc 134 / no refusal text); third PASSes.

- [ ] **Step 3: Implement the gate** — in `src/cli/run.rs`:

Add imports: `use crate::cli::sysmem;` — and after line 108 (`let buffers = config::size_buffers(&opts);`), before the `open_inputs` block:

```rust
    /* Memory guard, Layer A (plan 2026-10-04-memguard-bugfix): refuse
     * option combinations whose allocations exceed what the machine can
     * back (RAM + swap) — these die as allocation-failure aborts today.
     * Exit 10 (EXI_MEM) with an actionable breakdown; disabled by
     * JDIFF_UNSAFE_NO_MEMGUARD or when the ceiling is unknown (non-Linux). */
    let mem_plan = config::memory_footprint(&opts, &buffers);
    if sysmem::memguard_enabled() {
        if let Some(avail) = sysmem::available_anon_bytes() {
            if mem_plan.total > avail.saturating_sub(sysmem::MEMGUARD_HEADROOM) {
                print_memguard_refusal(&mem_plan, avail);
                return Ok(report(Err(JDiffError::Memory), verbose));
            }
        }
    }
```

Add the two helpers at module level in `run.rs`:

```rust
/// `fmt_mb` clamped for u64 budget values (saturated footprints print as
/// i64::MAX MiB, never a negative wrap).
fn mb(bytes: u64) -> String {
    crate::defs::fmt_mb(i64::try_from(bytes).unwrap_or(i64::MAX))
}

/// Layer-A refusal block (site-printed like the JFileOut family; the
/// boundary then prints the pinned "\nError allocating memory !\n" family
/// line and exits 10).
fn print_memguard_refusal(plan: &config::MemoryPlan, avail: u64) {
    dbg_print(format_args!(
        "Error: jdiff needs {} of memory, but only {} is available (RAM + swap).\n\n",
        mb(plan.total),
        mb(avail),
    ));
    dbg_print(format_args!("  buffers (-m)     : {}\n", mb(plan.buffers)));
    dbg_print(format_args!("  index table (-i) : {}\n", mb(plan.index_table)));
    dbg_print(format_args!("  match table (-x) : {}\n", mb(plan.match_table)));
    dbg_print(format_args!(
        "\nLower -m, -i or -x (see jdiff -hh), close memory-hungry programs, or add\nswap. To skip this check and let the OS overcommit decide, re-run with\nJDIFF_UNSAFE_NO_MEMGUARD=1.\n",
    ));
}
```

Register the module import: `use crate::cli::config::{self, Function};` already exists; add `use crate::cli::sysmem;` next to it (the module is `pub(crate)`-scoped inside `cli`, so `crate::cli::sysmem` resolves).

- [ ] **Step 4: Run** — `rtk cargo test --release --test memguard` → 3/3 PASS. Full suite `rtk cargo test --release` → PASS (the gate is invisible at every size the suites use).

- [ ] **Step 4b (execution finding): opt the byte-contract harnesses out of the gate.** The
first full-suite run exposed a plan gap: `tests/oracle.rs`, `tests/roundtrip.rs` and
`tests/crossver.rs` pin `-m 2048` (a valid, oracle-pinned configuration) — on hosts where
`MemAvailable + SwapFree − 512 MiB` dips below its 2080 MB footprint (this workstation under
load: ~2.5 GB available), the gate refuses it while the sparse zero pages previously ran fine.
Byte-contract suites must be RAM-independent (Global Constraints forbid GB-scale
machine-dependence), so their binary-spawning helpers set the documented escape hatch —
`JDIFF_UNSAFE_NO_MEMGUARD=1` in `tests/oracle.rs` `run_in`, `tests/common/mod.rs` `jdiff` and
`tests/crossver.rs` `run_copied` — and the guard stays exercised by `tests/memguard.rs`
(petabyte-scale, machine-independent). The §21.19 "sparse-overcommit now fails fast" note
remains true in production; in the harnesses it is the pre-existing behavior.

- [ ] **Step 5: Commit**

```bash
rtk git add src/cli/run.rs tests/memguard.rs
rtk git commit -m "fix: refuse memory-exceeding -m/-x/-i values with exit 10 (memguard Layer A)"
```

---

### Task 6: Layer B — `try_reserve` in the constructors (aborts become exit 10 everywhere)

**Files:**
- Modify: `src/lib.rs` (one generic helper), `src/jfile/ahead.rs`, `src/jhashpos.rs`, `src/jmatchtable.rs`, `src/jdiff.rs` (ctors → `Result`), `src/cli/run.rs` (2 note arms), `src/cli/diff_phase.rs` (`JDiff::new?`)
- Test: updates across unit tests/doctests listed below + one new integration test

**Interfaces:**
- Produces:
  - `crate::try_zeroed_vec<T: Clone>(len: usize, fill: T) -> Result<Vec<T>, JDiffError>` (lib.rs, `pub(crate)`)
  - `JFileAhead::new(...) -> Result<Self, JDiffError>` (same params)
  - `JHashPos::new(mb: i32) -> Result<Self, JDiffError>`
  - `JMatchTable::new(mch_sze: i32, cmp_all: bool, ahd_max: i32) -> Result<Self, JDiffError>`
  - `JDiff::new(...) -> Result<Self, JDiffError>` (same 11 params)
- Consumes: Task 5 `mb()` and `mem_plan`.

- [ ] **Step 1: Write the failing integration test** — append to `tests/memguard.rs`:

```rust
/// Layer B / escape hatch: with the guard disabled, a petabyte request is
/// refused by the OS (`try_reserve`) — exit 10 with the refusal note, NOT
/// the Layer-A "(RAM + swap)" text and NOT a SIGABRT. Petabyte scale keeps
/// this machine-independent (no OS grants 2.25 PB to one process).
#[test]
fn escape_hatch_lets_the_os_decide() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .env("JDIFF_UNSAFE_NO_MEMGUARD", "1")
        .args(["-m", "99999999999999999999", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("refused the"))
        .stderr(contains("Error allocating memory !"))
        .stderr(contains("(RAM + swap)").not());
}
```

Run: `rtk cargo test --release --test memguard escape_hatch` → FAIL (rc 134 today: the guard is skipped and `vec![0; …]` aborts).

- [ ] **Step 2: Implement the helper** — `src/lib.rs`, after the module declarations:

```rust
/// Allocation-guarded zeroed Vec (memory guard Layer B, plan
/// 2026-10-04-memguard-bugfix): `try_reserve_exact` instead of the
/// aborting `vec![fill; len]`, mapping an OS refusal (allocation failure,
/// overcommit limit) to `JDiffError::Memory` — exit 10 at the CLI
/// boundary instead of a Rust allocation abort.
pub(crate) fn try_zeroed_vec<T: Clone>(len: usize, fill: T) -> Result<Vec<T>, error::JDiffError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| error::JDiffError::Memory)?;
    v.resize(len, fill);
    Ok(v)
}
```

- [ ] **Step 3: Convert the three constructors + `JDiff::new`**

In each, swap the `vec![…]` field initializers for `try_zeroed_vec(...)?` and change the signature to `-> Result<Self, JDiffError>`:

- `src/jfile/ahead.rs` — `JFileAhead::new`: build the struct without `buf`, then insert `buf: try_zeroed_vec(buf_sze as usize, 0_u8)?,` (keep the field order comment; `chk_seq()` still runs last). Signature: `pub fn new(file: R, fid: &str, buf_sze: i64, blk_sze: i32) -> Result<Self, JDiffError>`. Import `crate::error::JDiffError`.
- `src/jhashpos.rs` — `JHashPos::new`: `tbl_pos: try_zeroed_vec(prime as usize, 0_i64)?, tbl_hsh: try_zeroed_vec(prime as usize, 0_u32)?,` (guard `prime.max(0) as usize` if you want belt-and-braces; prime is positive by the assert). Signature → `Result<Self, JDiffError>`.
- `src/jmatchtable.rs` — `JMatchTable::new`: `nodes: try_zeroed_vec(clamped as usize, EMPTY_NODE)?, col_tbl: try_zeroed_vec(mch_pme as usize, None)?, gld_tbl: …` where `const EMPTY_NODE: Node = Node { nxt: None, col: None, gldcnt: 0, … , cmp: CmpVal::Run(0) };` — a `Node` const needs every field const-constructible (they are: `CmpVal::Run(0)` is a const-friendly enum variant). Alternatively construct the struct literally inside the call. Signature → `Result<Self, JDiffError>`.
- `src/jdiff.rs` — `JDiff::new`: `hsh: JHashPos::new(hsh_sze)?, mch: JMatchTable::new(mch_max, cmp_all, ahd_max)?,` — signature → `Result<Self, JDiffError>` (params unchanged).

- [ ] **Step 4: Update every call site.** Enumerate first:

```bash
rtk proxy bash -c 'grep -rn "JFileAhead::new\|JHashPos::new\|JMatchTable::new\|JDiff::new" src tests | grep -v "fn new"'
```

Transformation rule: production code propagates with `?` (the surrounding functions already return `Result<_, JDiffError>`); tests and doctests append `.expect("test allocation")` (doctests may also `.unwrap()`). Concretely:

- `src/cli/run.rs` `open_inputs`: `Box::new(JFileAhead::new(open_dash(), "Org", buf_org, blk_sze)?)` and the `File::open` arm: `Ok(file) => Box::new(JFileAhead::new(file, "Org", buf_org, blk_sze)?),`
- `src/cli/diff_phase.rs`: `let mut lo_jdiff = JDiff::new(...)?;`
- `src/jdiff.rs` tests: `engine()` gets `JDiff::new(...).expect("test engine")`; the two direct `JDiff::new` calls in `ctor_clamps_and_mb_hash_wiring` / `src_scn_0_incremental_indexing_shifted_block` get `.expect("test engine")`.
- `src/jfile/ahead.rs` tests: `mk()` and `pipe()` get `.expect("test alloc")`; every direct `JFileAhead::new(...)` in the tests module (~10 sites) gets `.expect("test alloc")`.
- `src/jmatchtable.rs` tests + module doctest: every `JMatchTable::new(...)` (~20 sites) gets `.expect("test table")`; the doctest's `let mut tbl = JMatchTable::new(64, true, 1024).expect("doc table");`.
- `src/jhashpos.rs` tests + module doctest: every `JHashPos::new(...)` (~15 sites) gets `.expect("test table")`; doctest likewise.

- [ ] **Step 5: Layer-B note arms in `run.rs`.** The `open_inputs` failure arm (currently `Err(e) => return Ok(report(Err(e), verbose)),`) becomes:

```rust
        Err(e) => {
            if matches!(e, JDiffError::Memory) {
                print_mem_refusal_note(&mem_plan);
            }
            return Ok(report(Err(e), verbose));
        }
```

and before the final `Ok(report(li_ret, verbose))`:

```rust
    if let Err(JDiffError::Memory) = &li_ret {
        print_mem_refusal_note(&mem_plan);
    }
```

with the helper next to `print_memguard_refusal`:

```rust
/// Layer-B note: the pre-flight gate passed (or is disabled) but the OS
/// refused an allocation anyway (overcommit limits, non-Linux). The two
/// arms that can see this call here; the boundary prints the pinned
/// family line and exits 10.
fn print_mem_refusal_note(plan: &config::MemoryPlan) {
    dbg_print(format_args!(
        "Error: the operating system refused the {} memory allocation (allocation failed or overcommit limit reached). Lower -m, -i or -x.\n",
        mb(plan.total),
    ));
}
```

- [ ] **Step 6: Run everything**

`rtk cargo test --release` → all PASS (unit + integration + oracle + doctests). `rtk cargo test --release --test memguard` → 4/4 PASS.

- [ ] **Step 7: Commit**

```bash
rtk git add -A src tests
rtk git commit -m "fix: try_reserve engine allocations; allocation aborts become exit 10 (memguard Layer B)"
```

---

### Task 7: Finding 2 — 64-bit `size_bytes`

**Files:**
- Modify: `src/jhashpos.rs` (field + getter), `src/cli/diff_phase.rs:96` (drop the `i64::from`)
- Test: `src/jhashpos.rs` tests

**Interfaces:**
- Produces: `JHashPos::hash_size_bytes(&self) -> i64` (was `i32`). The ctor's `size_bytes_for(prime) as i32` cast becomes the direct i64 value.
- Consumes: Task 2 `size_bytes_for`.

- [ ] **Step 1: Write the failing test** — in `jhashpos.rs` tests (the existing `JHashPos::new(32).hash_size_bytes() == 33_554_172` assertions keep their values; add):

```rust
    /// `size_bytes` is i64: `-i >= 2049` overflows i32 (finding 2) — the
    /// values that wrapped to negative (observed "-2046Mb" in the -vv
    /// statistic) are now exact. The pure helper carries the math; these
    /// constructor checks stay at sizes any machine can allocate.
    #[test]
    fn hash_size_bytes_is_i64_exact() {
        assert_eq!(size_bytes_for(179_044_297), 2_148_531_564); // -i 2049 prime
        assert_eq!(size_bytes_for(536_870_909), 6_442_450_908); // -i 6144 case
        assert!(size_bytes_for(179_044_297) > i64::from(i32::MAX));
        // Constructor round-trip at sane sizes:
        assert_eq!(JHashPos::new(32).expect("table").hash_size_bytes(), 33_554_172_i64);
        assert_eq!(JHashPos::new(8).expect("table").hash_size_bytes(), 8_388_444_i64);
    }
```

(If Task 2's ctor still assigns `as i32`, the two `hash_size_bytes` assertions fail to compile against i32 — that is the red step.)

- [ ] **Step 2: Run** — `rtk cargo test --release hash_size_bytes` → FAIL (type mismatch / wrong values).

- [ ] **Step 3: Implement** — in `src/jhashpos.rs`: field `size_bytes: i64` (struct doc line ~98), ctor `let size_bytes = size_bytes_for(prime);` (drop `as i32`), getter `pub fn hash_size_bytes(&self) -> i64`. In `src/cli/diff_phase.rs` line 96: `let hashsize = lo_jdiff.hash().hash_size_bytes();` (the `i64::from` wrapper must go — it no longer compiles, which is the compiler proving the rewire). The debug-print at jhashpos.rs:166 formats the i64 unchanged.

- [ ] **Step 4: Run** — `rtk cargo test --release` → PASS. Also `rtk cargo test --release --features debug` → PASS (no overflow panic path left; the `-i 2049` panic is structurally gone because the multiply is i64).

- [ ] **Step 5: Commit**

```bash
rtk git add src/jhashpos.rs src/cli/diff_phase.rs
rtk git commit -m "fix: 64-bit hashtable size_bytes (-i >= 2049 i32 overflow, debug panic + wrong -vv stat)"
```

---

### Task 8: Big-patch streaming regression, docs, full verification

**Files:**
- Modify: `tests/memguard.rs` (one test), `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` (§21.19), `docs/superpowers/notes/2026-10-03-0.9.0-release-notes.md` (bullet)

**Interfaces:** none (test + docs).

- [ ] **Step 1: Write the big-patch regression test** — append to `tests/memguard.rs`:

```rust
/// "Don't break valid big patches": patch/destination files are STREAMED
/// through the fixed buffers — their SIZE is never bounded by the memory
/// guard, only the option VALUES are. A 32 MiB pair with edits produces a
/// real patch; applying it with defaults and with -m 256 must both be
/// byte-exact. (32 MiB keeps the run in the seconds range.)
fn lcg_bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as u8
        })
        .collect()
}

#[test]
fn big_patch_files_stream_unbounded() {
    let dir = tempfile::tempdir().unwrap();
    let org = lcg_bytes(1, 32 * 1024 * 1024);
    let mut new = org.clone();
    new[..16].copy_from_slice(&org[16 * 1024..16 * 1024 + 16]); // edit near head
    new[10_000_000..10_000_128].fill(0); // edit mid-file
    let a = dir.path().join("big.org");
    let b = dir.path().join("big.new");
    std::fs::write(&a, &org).unwrap();
    std::fs::write(&b, &new).unwrap();
    let patch = dir.path().join("big.patch");

    for m in ["64", "256"] {
        jdiff()
            .args(["-m", m, "-j"])
            .arg(&a)
            .arg(&b)
            .arg(&patch)
            .assert()
            .code(1);
        let out = dir.path().join("big.out");
        jdiff()
            .args(["-u", "-m", m])
            .arg(&a)
            .arg(&patch)
            .arg(&out)
            .assert()
            .code(0);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            new,
            "roundtrip byte-exact with -m {m}"
        );
    }
}
```

Run: `rtk cargo test --release --test memguard big_patch` → PASS (this is a regression guard, so red-first does not apply; it must pass on the current task-6 state).

- [ ] **Step 2: Spec §21.19** — append to the deviations section of `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`:

```markdown
### §21.19 Memory guard (exit 10 instead of allocation abort)

The C++ never checks its `malloc`s: oversized `-m`/`-x`/`-i`/`-k` values
die in the allocator (and C `atoi` truncates them to small values anyway,
a UB-adjacent divergence the port's saturating `c_atoi` does not share).
The port refuses them cleanly:

* **Layer A** (`cli::run`, Linux only): before any file is opened, the
  exact allocation footprint (two input buffers, hash table, match table)
  is compared against `MemAvailable + SwapFree − 512 MiB`
  (`/proc/meminfo`); exceeding it prints a per-option byte breakdown and
  exits 10 (`EXI_MEM`, "Error allocating memory !"), the C++ exit code
  reserved for this condition. Configurations that fit run byte-identical.
* **Layer B** (all platforms): the three engine constructors allocate via
  `try_reserve`; an OS refusal is `JDiffError::Memory` → exit 10, never a
  Rust allocation abort.
* `JDIFF_UNSAFE_NO_MEMGUARD=1` disables Layer A (the OS overcommit
  heuristic decides, as in the pre-guard builds).
* New stderr text exists ONLY on these paths — the pre-fix binary produced
  no jdiff output there (SIGABRT), so no pinned bytes changed.
* Known deliberate effect: sparse-overcommit runs (e.g. a 25 GB `-i` on a
  4 GB machine with small inputs) that previously "worked" by luck now
  fail fast with exit 10.
```

- [ ] **Step 3: Release-notes bullet** — append to the "Fixed" section of `docs/superpowers/notes/2026-10-03-0.9.0-release-notes.md`:

```markdown
- Huge `-m`/`-x`/`-i`/`-k` values now exit 10 (`Error allocating memory !`)
  with a per-option memory breakdown instead of aborting; the check is
  bounded by `MemAvailable + SwapFree` and can be skipped with
  `JDIFF_UNSAFE_NO_MEMGUARD=1` (spec §21.19). `-i ≥ 2049` no longer
  overflows the table-size statistic (was negative garbage; debug builds
  panicked).
```

- [ ] **Step 4: Full verification wave**

```bash
rtk cargo test --release                     # all suites
rtk cargo test --release --features debug    # debug-feature build (overflow panics gone)
rtk cargo clippy --all-targets -- -D warnings
rtk cargo fmt --check
```

Manual differential check against the pre-fix build (byte-contract proof — run once, not committed):

```bash
rtk proxy bash -c '
cd /tmp && rm -rf memguard-diff && mkdir memguard-diff && cd memguard-diff
OLD=$HOME/Projects/jojodiff-cli-rs/.worktrees/refactor-idiomatic-rust/target/release/jdiff
NEW=$HOME/Projects/jojodiff-cli-rs/.worktrees/fix-memguard/target/release/jdiff
python3 -c "
import sys
s=1; out=bytearray()
for _ in range(100000):
    s=(s*1664525+1013904223)&0xffffffff; out.append((s>>8)&0xff)
sys.stdout.buffer.write(out)" > a1.bin
cp a1.bin a2.bin; printf X | dd of=a2.bin bs=1 seek=5000 conv=notrunc 2>/dev/null
FAIL=0
for opts in "" "-b" "-bb" "-f" "-ff" "-p" "-q" "-l" "-r" "-v" "-vv" "-a 8" "-i 1" "-k 512" "-m 8" "-n 1" "-x 5" "-x 13" "-i 8 -m 32 -k 4096"; do
  timeout 30 $OLD $opts -j a1.bin a2.bin o.patch >o.out 2>o.err; orc=$?
  timeout 30 $NEW $opts -j a1.bin a2.bin n.patch >n.out 2>n.err; nrc=$?
  cmp -s o.out n.out && cmp -s o.err n.err && cmp -s o.patch n.patch && [ "$orc" = "$nrc" ] || { echo "DIVERGE [$opts] rc $orc/$nrc"; FAIL=1; }
done
echo "differential: FAIL=$FAIL"; cd /tmp && rm -rf memguard-diff'
```

Expected: `differential: FAIL=0` (the guard changes nothing for any in-memory configuration).

- [ ] **Step 5: Commit**

```bash
rtk git add tests/memguard.rs docs/superpowers
rtk git commit -m "test: big-patch streaming regression; docs: spec 21.19 memory guard + release notes"
```

---

### Task 9: Flag-chain floor — `mch_max <= 0` never reaches the engine

**Files:**
- Modify: `src/cli/config.rs` (one floor in `parse`, before the `Options` construction at ~line 305)
- Test: `src/cli/config.rs` tests module; `tests/memguard.rs`

**Interfaces:** none new — this only guarantees the invariant `Options.mch_max >= 1` that `JMatchTable::new`'s assert (and the C++'s `calloc`) rely on.

**Background for the implementer:** `-f` has no argument; `-ffffffff` is a cluster of eight flags. Each `-f` beyond the first does `mch_max /= 2` (0 at the 8th); each `-b` does `mch_max.wrapping_mul(4)` (`i32::MIN` at the 12th). `mch_max <= 0` makes `mch_pme_for` return `<= 0` and the ctor assert dies (rc=101 today; C++ `calloc(0)` + modulo-zero UB). The `-x` handler already floors its own `<= 0` to 1024 (`src/cli/config.rs:262-269`) — this task applies the same floor once, after parsing, so no combination can reach the engine with a non-positive value. Safe by construction: every `mch_max <= 0` configuration dies today, so no working behavior can change.

- [ ] **Step 1: Write the failing unit test** — append to `config.rs` tests:

```rust
    /// Flag-chain floor (plan task 9): `-f` x8 halves mch_max 128 -> 0 and
    /// `-b` x12 wraps `*4` to i32::MIN — both tripped JMatchTable's ctor
    /// assert (rc=101; C++ calloc(0)/%0 UB). After parsing, mch_max is
    /// floored like a non-positive -x value; no working config changes.
    #[test]
    fn f_and_b_chains_floor_mch_max() {
        let f8 = parse(&[argv0("jdiff"), OsString::from("-ffffffff")]);
        assert_eq!(f8.mch_max, 1024);
        let b12 = parse(&[argv0("jdiff"), OsString::from("-bbbbbbbbbbbb")]);
        assert_eq!(b12.mch_max, 1024);
        // A 7-deep -f chain stays at 1 — the floor must not touch it.
        let f7 = parse(&[argv0("jdiff"), OsString::from("-fffffff")]);
        assert_eq!(f7.mch_max, 1);
    }
```

- [ ] **Step 2: Run** — `rtk cargo test --release f_and_b_chains` → FAIL (`f8.mch_max` is 0, `b12.mch_max` is `i32::MIN`).

- [ ] **Step 3: Implement** — in `parse`, immediately before the `Options { … }` construction:

```rust
    // Post-parse floor (plan task 9): -f chains (8+) and -b chains (12+)
    // can drive mch_max to <= 0 — JMatchTable's ctor assert dies there
    // today (the C++ hits calloc(0)/modulo-zero UB). Floor like the -x
    // handler (<= 0 -> 1024); safe because <= 0 never ran.
    if mch_max <= 0 {
        mch_max = 1024;
    }
```

- [ ] **Step 4: Write the integration regression test** — append to `tests/memguard.rs`:

```rust
/// Flag-chain regression (plan task 9): `-ffffffff` (8x -f halves
/// mch_max to 0) and `-bbbbbbbbbbbb -m 8 -i 1` (12x -b wraps it to
/// i32::MIN with small buffers) both panicked in JMatchTable's ctor
/// assert (rc=101). Both now run the normal diff path (exit 1).
#[test]
fn flag_chains_floor_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    // -f x8: buffers 16 MB, table 1 MB (hsh_mbt halves to 0 -> clamped
    // to 1 by JHashPos) — deterministic on any machine.
    jdiff()
        .args(["-ffffffff", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("f8.patch"))
        .assert()
        .code(1);
    // -b x12 + small -m/-i: without the floor this reached the ctor with
    // mch_max = i32::MIN (observed rc=101 pre-fix).
    jdiff()
        .args(["-bbbbbbbbbbbb", "-m", "8", "-i", "1", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("b12.patch"))
        .assert()
        .code(1);
}
```

- [ ] **Step 5: Run everything** — `rtk cargo test --release` → all PASS (the pre-fix state fails Step 4's first assertion with rc=101, which is the red step for the integration side).

- [ ] **Step 6: Commit**

```bash
rtk git add src/cli/config.rs tests/memguard.rs
rtk git commit -m "fix: floor mch_max after -f/-b flag chains (-f x8 / -b x12 hit the ctor assert)"
```



---

## Self-Review (completed during planning)

1. **Coverage:** Finding 1 → Tasks 3–6 (gate + try_reserve + messages); finding 2 → Task 2 (helper) + Task 7 (rewire); flag-chain panics (`-f`×8, `-b`×12) → Task 9 (post-parse `mch_max` floor, verified vectors rc 101 → 1); "don't break valid big patches" → Task 8 streaming test + differential wave; user-facing guidance → Task 5/6 message blocks; escape hatch → Task 4 + Task 6 test; docs → Task 8. Exit code stays the pinned `EXI_MEM` family (10).
2. **Placeholders:** none — every step carries code, commands, and expected results.
3. **Type consistency:** `memory_footprint` returns `MemoryPlan` (u64) used verbatim by both run.rs arms; `size_bytes_for -> i64` feeds Task 7's i64 field; `try_zeroed_vec` is the single allocation helper for all three ctors; `fmt_mb(i64)` wrapped by run.rs's `mb(u64)` clamp.
4. **Known accepted effect** (documented in §21.19): sparse-overcommit configurations now fail fast instead of running by luck.
