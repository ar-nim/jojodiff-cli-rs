# Rust Pitfall Audit Residue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the behavior-preserving fixes from the 2026-10-05 three-axis Rust audit (avoidable mutables via C-style out-params, unsafe-bypass hygiene, Rust-only pitfalls) with zero byte-contract regressions.

**Architecture:** Seven surgical, independently-committable transformations on the 0.9.0 codebase: panic-free tier emission in `JOutBin`, self-verifying lint suppression, a Cargo lint-key rename, and four API-shape modernizations that replace `&mut` out-parameter channels (`JHashPos::get`, `JDiff::search`, `hash_key`, `JFileAhead::getbuf_off`) with typed returns. Every task is green-to-green: the existing 253-test suite (which byte-compares goldens) is the regression authority; each task first adds a test that pins the new API shape (red = compile error), then lands the change.

**Tech Stack:** Rust (MSRV 1.85, edition 2024, no `let`-chains), std + thiserror/anyhow/sysinfo already in tree. No new dependencies.

**Spec:** This plan is self-contained (it embeds its audit findings), but it argues from:
- `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` — §21 remains the behavioral authority (byte contract, do-not-fix quirks).
- `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` — §15's adopted/deferred/do-not-fix rulings bound what this plan may touch. The 0.9.0 refactor it describes is merged (PR #1); this plan is a residue pass, not a restart.
- Rule IDs cited as `ID` (e.g. `ERRORS-06`, `OWNERSHIP-05`) are from the AlphaOne Rust 1.98 standard / Apollo handbook in the `rust-best-practices` skill.

## Global Constraints

- **Byte contract**: no change to any stdout/stderr byte, patch byte, or exit code for
  configurations that run today. Goldens are never regenerated. (port spec §21; design §3)
- **No `unsafe`**, no `static mut` — hard. (AGENTS.md; unchanged by this plan)
- **MSRV 1.85, edition 2024** — no `let`-chains, no 1.91+ APIs (`strict_*`, `NumBuffer`). (design §15)
- **Full gate before any "done" claim**, run bare and judged on its own exit code:
  `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`
- **Conventional Commits** (`refactor:`, `fix:`, `docs:`, `test:`), one logical change per commit.
- Tests are machine-independent; wrap every spawned `jdiff` in `timeout`. Byte-contract
  harnesses already set `JDIFF_UNSAFE_NO_MEMGUARD=1`; do not remove it.
- The dev-profile test run is the integer-overflow canary (debug builds panic on
  overflow); tasks here add no arithmetic, only reshaping.
- Work happens in the worktree `.worktrees/refactor-rust-pitfall-audit` on branch
  `refactor/rust-pitfall-audit` (based on `main` @ `54d2b98`, baseline: build green,
  fmt green, clippy green, 253 tests / 9 suites green — verified 2026-10-05).

## Audit findings this plan acts on

Full audit record (including the ruled-out items, so a future auditor does not
re-litigate them) — see Appendix A at the end of this document.

| ID | Finding | Axis | Rule | Task |
|----|---------|------|------|------|
| F1 | `JHashPos::get(key, &mut pos) -> bool` out-param forces `let mut pos` receiver cells | mut | OWNERSHIP-05, API | 4 |
| F2 | `JDiff::search` takes three `&mut i64` out-params | mut | OWNERSHIP-05 | 6 |
| F3 | `hash_key(cur, &mut old, new, &mut eql) -> u32` three-channel rolling-hash update scattered across `sst` fields and locals | mut | OWNERSHIP-05, API | 5 |
| F4 | `JFileAhead::getbuf_off(pos, &mut len, typ)` + `get_frombuffer -> i32`: internal legacy-i32 sentinel seam with an out-param | mut + pitfall | ERRORS-01 family, API | 7 |
| F7 | `LC_TST.lock().unwrap()` inconsistent with the crate's own `PoisonError::into_inner` idiom (debug-only path) | unsafe-hygiene | ERRORS-06 | 7 |
| F9 | `tier.marker().unwrap()` ×4 in `JOutBin::put_len` — panic-capable where a panic-free form is byte-identical | pitfall | ERRORS-06/07 | 3 |
| F10 | Five `#[allow(...)]` where `#[expect(...)]` would self-verify the reason still holds | pitfall | Apollo ch.2 | 2 |
| F11 | Cargo.toml `[lints] rust future-incompatible` key deprecated → warning on every build (the tree's only build warnings) | pitfall | TOOLING | 1 |

Ruled out (documented, not acted on): engine loop `&mut` threading and manual loops
(design §15 ITER-01 do-not-fix), `p8()` per-call `String` (deferred to an MSRV bump,
design §15), writer `expect("write error")` (policy: diff path sits on
`IgnoringWriter`, port spec deviation 3), lossy `to_string_lossy` arg handling
(pinned grammar — "lossy `c_atoi`", design §2), pointer-format debug traces
(shape-pinned, safe, debug-only), `dist()` SIGFPE-parity division (§21-ruled),
`build_full_index`'s single field-disjoint `let Self {...}` destructure (sanctioned
borrow splitting, not a workaround), ported `as` casts at labeled C++ narrowing sites
(pinned), dead stores in `jpatch` (documented parity).

## File Structure

No new files except this plan. All edits are in place:

- `Cargo.toml` — lint table key rename (Task 1)
- `src/jdiff.rs`, `src/jmatchtable.rs`, `src/jpatcht.rs` — allow→expect (Task 2)
- `src/jout/bin.rs` — put_len marker arms + new unit test (Task 3)
- `src/jhashpos.rs` — `get` returns `Option<i64>`; tests updated (Task 4)
- `src/jdiff.rs` — `RollingHash` bundle; `sst` field regrouping (Task 5)
- `src/jdiff.rs` — `search` returns `SearchOutcome` (Task 6)
- `src/jfile/ahead.rs` — typed internal get channel + LC_TST idiom (Task 7)

---

### Task 1: Rename the deprecated Cargo lint key

**Files:**
- Modify: `Cargo.toml` (the `[lints]` table, around line 60–75)

**Interfaces:** none (build metadata only).

- [x] **Step 1: Reproduce the warning**

Run: `cargo build 2>&1 | grep -c "future-incompatible"`
Expected: `2` (one per manifest parse — the tree's only build warnings today).

- [x] **Step 2: Apply the rename**

In the `[lints]` table, change the rust subsection key `future-incompatible` to
`future_incompatible` ( Cargo's documented name; the hyphenated form is the
deprecated alias). Everything under it (`level = "deny"`) is unchanged. If the
table also carries `clippy::cast_lossless = "deny"` or equivalent entries, leave
them exactly as they are.

- [x] **Step 3: Verify zero warnings and that the deny still binds**

Run: `cargo build 2>&1 | grep -ci "warning"` → expect `0`.
Run: `cargo clippy --all-targets --all-features -- -D warnings` → expect exit 0
(this exercises the lint table; a broken table fails the manifest parse outright).

- [x] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock 2>/dev/null || git add Cargo.toml
git commit -m "fix: rename deprecated lints.rust.future-incompatible key to future_incompatible"
```

---

### Task 2: Replace lint `#[allow]`s with self-verifying `#[expect]`s

**Files:**
- Modify: `src/jdiff.rs:174` (`#[allow(clippy::too_many_arguments)]`)
- Modify: `src/jmatchtable.rs:312` (`#[allow(dead_code)]` on field `ahd_max`)
- Modify: `src/jmatchtable.rs:920` (`#[allow(clippy::manual_clamp)]`)
- Modify: `src/jmatchtable.rs:1439` (`#[allow(clippy::if_same_then_else)]`)
- Modify: `src/jpatcht.rs:295` (`#[allow(unused_assignments)]`)

**Interfaces:** none (attribute-level change; each lint fires today, so `#[expect]` compiles).

Rationale: `#[allow]` silences forever, even after the reason disappears; `#[expect]`
errors the moment the lint stops firing, so the justification stays honest. Where the
existing comment does not already state the reason, add one line stating it.

- [x] **Step 1: Write the verification probe (red)**

Temporarily neutralize one suppression to prove the lint still fires — e.g. change
`#[allow(dead_code)]` to `#[deny(dead_code)]` on `ahd_max`:

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: FAIL — `field ahd_max is never read` (the §21.13 parity reason holds).

- [x] **Step 2: Convert all five sites**

Each site: `#[allow(LINT)]` → `#[expect(LINT)]`. The existing doc/comment blocks
already carry the justifications (dead-code parity §21.13, the C++ if/else-if clamp
spelling, the 1:1 same-branch port, the ported dead stores, the 11-arg C++ ctor);
keep them. Restore the Task-1 probe site to `#[expect(dead_code)]`.

- [x] **Step 3: Verify the expects are live**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: exit 0. (If any `#[expect]` did NOT fire, rustc errors with
`this lint expectation is unfulfilled` — that would mean the reason is stale and
the suppression should be deleted instead; investigate, don't force it.)

- [x] **Step 4: Full gate + commit**

Run the Global-Constraints gate bare. Then:

```bash
git add src/jdiff.rs src/jmatchtable.rs src/jpatcht.rs
git commit -m "refactor: self-verifying #[expect] lint suppressions replace #[allow]"
```

---

### Task 3: Panic-free tier-marker emission in `JOutBin::put_len`

**Files:**
- Modify: `src/jout/bin.rs:132-171` (`put_len`)
- Test: `src/jout/bin.rs` (in `#[cfg(test)] mod tests`, which already has a
  `Vec<u8>`-sink driver pattern around line 344)

**Interfaces:**
- Consumes: `crate::jout::wire::{len_tier, LenTier}` (unchanged).
- Produces: same `put_len(&mut self, len: i64)` private method, byte-identical
  output, no `unwrap()`.

- [x] **Step 1: Write the failing test first**

Add to `mod tests` in `src/jout/bin.rs` (adapt the sink construction to the file's
existing test driver: if the driver returns `(Vec<u8>, OutStats)` via
`JOutBin::new` + `into_inner`, use it; the assertion targets are the wire bytes
from the `put_len` doc table, `JOutBin.cpp:65-104`):

```rust
/// `put_len` emits exactly the documented tier encoding for one length
/// per tier: L252 (len-1), L508 (252, len-253), L16 (253, hi, lo),
/// L32 (254, 4 bytes BE), L64 (255, 8 bytes BE). Byte-identical to the
/// oracle tier table; guards the marker-emit restructure.
#[test]
fn put_len_emits_exact_tier_bytes() {
    let cases: &[(i64, &[u8])] = &[
        (1, &[0x00]),                       // L252: len-1
        (252, &[0xFB]),                     // L252: len-1 = 251
        (253, &[252, 0x00]),                // L508: marker + len-253
        (508, &[252, 0xFF]),                // L508: 508-253 = 255
        (509, &[253, 0x01, 0xFD]),          // L16: 509 = 0x01FD
        (0xFFFF, &[253, 0xFF, 0xFF]),       // L16
        (0x1_0000, &[254, 0x00, 0x01, 0x00, 0x00]), // L32: 65536 BE
        (0x1_0000_0000, &[255, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]), // L64
    ];
    for &(len, want) in cases {
        let mut out = JOutBin::new(Vec::new());
        out.put_len(len);
        let buf = out.into_inner();
        assert_eq!(buf.as_slice(), want, "put_len({len})");
    }
}
```

Note: `put_len` is private and `JOutBin::new` wraps a raw sink (no IgnoringWriter
here), so this test exercises the exact byte path; write errors are impossible on
`Vec<u8>`.

- [x] **Step 2: Run it — expect PASS first (pin step)**

Run: `cargo test --all-features put_len_emits_exact_tier_bytes`
Expected: PASS. This is deliberate: the test pins CURRENT behavior so the
restructure in Step 3 provably changes nothing. (If it fails, the assumed vectors
are wrong — recompute them from the `put_len` doc table before touching the code,
and re-verify one vector by printing `buf` with `-- --nocatch`-style debugging.)

- [x] **Step 3: Restructure the marker emission**

In each multi-byte arm of `put_len`, replace

```rust
self.raw(tier.marker().unwrap());
```

with

```rust
// L252 has no marker; the multi-byte tiers emit theirs.
if let Some(marker) = tier.marker() {
    self.raw(marker);
}
```

(4 sites: `L508`, `L16`, `L32`, `L64`. The `L252` arm has no marker call and is
untouched. Do NOT hoist the emission above the `match` — arm-local keeps the diff
minimal and review trivial.)

- [x] **Step 4: Verify green**

Run: `cargo test --all-features put_len_emits_exact_tier_bytes` → PASS.
Run the full gate bare (goldens byte-compare the real patch streams).

- [x] **Step 5: Commit**

```bash
git add src/jout/bin.rs
git commit -m "refactor: panic-free LenTier marker emission in JOutBin::put_len"
```

---

### Task 4: `JHashPos::get` returns `Option<i64>`

**Files:**
- Modify: `src/jhashpos.rs:267-282` (`get`)
- Modify: `src/jdiff.rs:878` (the single production caller)
- Test: `src/jhashpos.rs` `mod tests` (callers at ~503–600 use the old shape)

**Interfaces:**
- Produces: `pub fn get(&mut self, key: u32) -> Option<i64>` — `Some(pos)` on
  exact-key match (including the untouched-zero-bucket `(key==0) → Some(0)` quirk,
  spec §21.5), `None` otherwise. The `hits` counter still increments exactly on
  the `Some` path.

- [x] **Step 1: Write the failing test (red = compile error on new shape)**

In `src/jhashpos.rs` tests, add:

```rust
/// The typed lookup: Some(pos) on hit (hits counter bumped), None on miss.
/// The untouched zero-bucket quirk (§21.5) answers Some(0) for key 0.
#[test]
fn get_returns_option_position() {
    let mut tbl = JHashPos::new(1).expect("test table");
    let mut feeder_old = EOF as i32;
    let mut feeder_eql = 0_i32;
    let mut key = 0_u32;
    for b in 0_i32..40 {
        key = crate::jdiff::hash_key(key, &mut feeder_old, b, &mut feeder_eql);
    }
    tbl.add(key, 1234, 32);
    let hits_before = tbl.hash_hits();
    assert_eq!(tbl.get(key), Some(1234));
    assert_eq!(tbl.hash_hits(), hits_before + 1);
    assert_eq!(tbl.get(key ^ 0xFFFF_0000), None); // different key, same table
    assert_eq!(tbl.get(0), Some(0)); // untouched zero bucket (§21.5)
}
```

(The exact feed loop only needs a key that was `add`ed; reuse whatever pattern the
neighboring `add`/`get` tests already use — copy their feeder, don't invent one.)

- [x] **Step 2: Verify it fails**

Run: `cargo test --all-features get_returns_option_position`
Expected: FAIL to compile — `this method takes 2 arguments but 1 was supplied`.

- [x] **Step 3: Change the signature and callers**

```rust
/// Hashtable lookup (`JHashPos.cpp:158-171`): exact-key match at
/// `key % prime` only; increments the hit counter on match. On a
/// zero-filled untouched bucket (`key == 0`) this answers `Some(0)` —
/// the C++ UB reads zero pages in practice (spec §21.5 deviation).
pub fn get(&mut self, key: u32) -> Option<i64> {
    let idx = (key % self.prime as u32) as usize;
    if self.tbl_hsh[idx] == key {
        self.hits += 1;
        return Some(self.tbl_pos[idx]);
    }
    None
}
```

Production caller `src/jdiff.rs:878` — from

```rust
if self.hsh.get(self.sst.hsh_new, &mut lz_fnd_org) {
```

to

```rust
if let Some(lz_fnd_org) = self.hsh.get(self.sst.hsh_new) {
```

(the surrounding block body already reads `lz_fnd_org`; delete the now-unneeded
`let mut lz_fnd_org` receiver cell at the top of `search` — its comment says the
C++ declares `lzFndNew` as `getbest`'s out-parameter, which the `Option` return
replaced; update that comment to mention both lookups now).

Update every `tbl.get(k, &mut pos)` pattern in the `jhashpos.rs` tests to
`tbl.get(k)` with `assert_eq!`, and the doc-example comment at `jhashpos.rs:60-65`
to the new shape.

- [x] **Step 4: Verify green + full gate**

Run: `cargo test --all-features` (all 9 suites; the roundtrip/oracle suites pin
the engine behavior end to end).

- [x] **Step 5: Commit**

```bash
git add src/jhashpos.rs src/jdiff.rs
git commit -m "refactor: JHashPos::get returns Option<i64> instead of a &mut out-parameter"
```

---

### Task 5: Bundle the rolling-hash state into a `RollingHash` struct

**Files:**
- Modify: `src/jdiff.rs` — `SearchState` fields (~the `hsh_*`/`prv_*`/`eql_*`
  triplets), `hash_add_org` (line ~271), the three other `hash_key` production
  call sites (~677, ~817, ~868), `build_full_index` locals (~1069–1120)
- Test: `src/jdiff.rs` tests (port/extend the `hash_key` tests at ~1217–1264)

**Interfaces:**
- Consumes: `pub fn hash_key(cur: u32, old: &mut i32, new: i32, eql: &mut i32) -> u32`
  (STAYS public and unchanged — it is the single source of the hash math, used by
  `jhashpos` doc/tests).
- Produces:

```rust
/// The rolling sample hash (`ufHshAdd` state): the current key, the previous
/// byte, and the equal-run counter, bundled so the three values update as one
/// unit instead of threading through `&mut` out-parameters.
#[derive(Clone, Copy)]
pub(crate) struct RollingHash {
    key: u32,  // current hash value (lzHshOrg / lzHshNew)
    prv: i32,  // previous byte (EOF-seeded)
    eql: i32,  // equal-character run feeding the hash
}

impl RollingHash {
    /// Fresh state, matching the C++ locals' zero/EOF initialization.
    pub(crate) fn new() -> Self {
        RollingHash { key: 0, prv: EOF, eql: 0 }
    }

    /// Fold one byte (the raw i32 channel value, EOF included) into the hash.
    pub(crate) fn roll(&mut self, new: i32) -> u32 {
        self.key = hash_key(self.key, &mut self.prv, new, &mut self.eql);
        self.key
    }

    /// Current key (for table lookups and adds).
    pub(crate) fn key(&self) -> u32 {
        self.key
    }

    /// Current equal-run length (the `add` quality argument).
    pub(crate) fn eql(&self) -> i32 {
        self.eql
    }
}
```

- [x] **Step 1: Write the failing test**

```rust
/// RollingHash bundles exactly the hash_key channel state: rolling a byte
/// sequence gives the same key/eql as the raw three-variable form, and the
/// EOF seed matches the C++ locals (`lcValPrv = EOF`, key 0, eql 0).
#[test]
fn rolling_hash_matches_raw_channel() {
    let mut rh = RollingHash::new();
    let mut raw_key = 0_u32;
    let mut raw_prv = EOF;
    let mut raw_eql = 0_i32;
    for b in b"the quick brown fox" {
        let via_struct = rh.roll(i32::from(*b));
        raw_key = hash_key(raw_key, &mut raw_prv, i32::from(*b), &mut raw_eql);
        assert_eq!(via_struct, raw_key);
        assert_eq!(rh.key(), raw_key);
        assert_eq!(rh.eql(), raw_eql);
    }
}
```

- [x] **Step 2: Verify it fails**

Run: `cargo test --all-features rolling_hash_matches_raw_channel`
Expected: FAIL to compile — `RollingHash` not found.

- [x] **Step 3: Introduce the struct and migrate the six call sites**

In `SearchState`, replace the two field triplets (`hsh_org`/`prv_org`/`eql_org`
and `hsh_new`/`prv_new`/`eql_new` — exact names per the struct definition; keep
the port-anchor comments) with `rh_org: RollingHash` and `rh_new: RollingHash`.
Then:

- `hash_add_org` becomes:

```rust
fn hash_add_org(&mut self, lc_org: i32) {
    self.sst.rh_org.roll(lc_org);
    self.hsh.add(self.sst.rh_org.key(), self.sst.az_org, self.sst.rh_org.eql());
    self.sst.az_org += 1;
}
```

- The other three `sst` call sites follow the same `roll`/`key()`/`eql()` shape
  (site ~677 is the org-side scan, ~817/~868 the new-side lookahead loop —
  each currently assigns `self.sst.hsh_X = hash_key(self.sst.hsh_X, &mut
  self.sst.prv_X, val, &mut self.sst.eql_X);`).
- In `build_full_index`, replace the `lk_hsh_org`/`lc_val_prv`/`li_eql_org`
  locals with one `let mut rh = RollingHash::new();` and the two `hash_key`
  calls with `rh.roll(...)` (keeping the `ByteOrEof::Byte` destructuring around
  them unchanged).
- Update any `SearchState` construction site (the `Default`/initializer) and the
  struct's doc comment to name the bundle.

- [x] **Step 4: Verify green + full gate**

The roundtrip/golden suites exercise the hash math on every byte; any drift is a
suite failure.

- [x] **Step 5: Commit**

```bash
git add src/jdiff.rs
git commit -m "refactor: bundle rolling-hash state into RollingHash (no &mut out-params)"
```

---

### Task 6: `JDiff::search` returns a `SearchOutcome` struct

**Files:**
- Modify: `src/jdiff.rs:622-629` (`search` signature), the three `*skp_*`/`*ahd`
  write sites (~1013–1049), and the caller in `jdiff` (~500–506 plus the
  receiver cells at ~335–338)

**Interfaces:**
- Produces:

```rust
/// Result of one find-ahead pass (`JDiff::search`, the C++ `azSek*`/`azAhd`
/// reference out-parameters plus its 0/1 return): `found` is the C++ `int`
/// return (1 = solution found), the offsets are the skip/advance vector.
#[derive(Debug, Clone, Copy)]
struct SearchOutcome {
    found: bool,  // C++ returns 1 when a solution was found, else 0
    skp_org: i64, // bytes to skip on the original file (negative = backtrack)
    skp_new: i64, // bytes to skip on the new file
    ahd: i64,     // bytes both cursors advance to reach the solution
}
```

  New signature: `fn search(&mut self, red_org: i64, red_new: i64) -> Result<SearchOutcome, JDiffError>`

- [x] **Step 1: Write the failing test**

The suite already pins `search` end-to-end through `jdiff` (engine tests at
~1404–1760 drive it via `JDiff::jdiff`). Add a direct compile-shape pin:

```rust
/// search answers a struct; the no-solution path zeroes the skips and
/// floors the advance at SMPSZE (JDiff.cpp:716-717).
#[test]
fn search_no_solution_returns_smpsze_floor() {
    let mut jd = test_engine(b"abcdef", b"zzzzzz"); // reuse the file's engine fixture
    let out = jd.search(0, 0).expect("search never errors on in-memory files");
    assert!(!out.found);
    assert_eq!(out.skp_org, 0);
    assert_eq!(out.skp_new, 0);
    assert!(out.ahd >= i64::from(SMPSZE));
}
```

Adapt the fixture name to the actual test helper in `src/jdiff.rs` tests (there
are `JDiff::new(...).expect("test engine")` fixtures around line 1372 — reuse one;
`search` is private, so this test lives inside the file's `#[cfg(test)] mod`).

- [x] **Step 2: Verify it fails**

Run: `cargo test --all-features search_no_solution_returns_smpsze_floor`
Expected: FAIL to compile — wrong arg count / unknown field `found`.

- [x] **Step 3: Migrate signature, writes, and caller**

- Signature drops `skp_org`/`skp_new`/`ahd` params; the `Ok(0)`/`Ok(1)` returns
  become `Ok(SearchOutcome { found: false/true, skp_org, skp_new, ahd })` with
  the same values the `*out` writes produce today (write them once in the struct
  literal at each of the three return arms — `None` arm ~1013, `Some` forward
  arms ~1029/1034, backtrack arm ~1040/1046).
- Caller in `jdiff`: delete the `let mut lz_skp_org/lz_skp_new/lz_ahd` receiver
  cells (keep `lz_ahd` — it IS mutated independently across the loop as the
  ahead budget; re-bind via `let mut lz_ahd = 0_i64;` only if the loop still
  needs a mutable local, seeding it from `outcome.ahd` at the call):

```rust
let outcome = self.search(lz_pos_org, lz_pos_new)?;
li_fnd = i32::from(outcome.found);
let mut lz_skp_org = outcome.skp_org;
let mut lz_skp_new = outcome.skp_new;
lz_ahd = outcome.ahd;
```

  (Check every read of `li_fnd` afterwards: the loop compares `li_fnd == 1` —
  becomes `li_fnd == 1` against the i32, unchanged — and resets `li_fnd = 0`.
  Keep the debug trace at ~513 reading the outcome fields.)

- [x] **Step 4: Verify green + full gate** (the golden/oracle suites are the
  behavior authority for the offset math).

- [x] **Step 5: Commit**

```bash
git add src/jdiff.rs
git commit -m "refactor: JDiff::search returns SearchOutcome instead of &mut out-params"
```

---

### Task 7: Typed internal get channel in `JFileAhead`

**Files:**
- Modify: `src/jfile/ahead.rs` — `get_frombuffer` (~264–293), `getbuf_off`
  (~363–442), `getbuf` (~707–712), `get` (~666), `verify_buffer` LC_TST lock
  (~323)
- Test: `src/jfile/ahead.rs` tests (~1135–1186 use `getbuf_off` with `&mut len`)

**Interfaces:**
- Produces (private, same file):

```rust
/// Failure half of the buffered-read channel: which sentinel ended the run.
/// Same information as today's `*len` out-parameter writes
/// (JFileAhead.cpp:222-225), typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GetBufMiss {
    Eof,
    Eob,
    SeekError,
    ReadError,
}

/// getbuf_off: `Ok((offset into buf, run length))` or the miss reason.
/// Post-conditions identical to today: on success `len` is the number of
/// available bytes (the run never wraps the ring); on failure the miss
/// names the sentinel previously written through `*len`.
fn getbuf_off(&mut self, pos: i64, typ: ReadType) -> Result<(usize, i64), GetBufMiss>;
/// get_frombuffer: returns ByteOrEof directly (Byte on success, the miss
/// mapped to Eof/Eob/Err(JDiffError::Seek|Read)).
fn get_frombuffer(&mut self, pos: i64, typ: ReadType) -> ByteOrEof;
```

- [x] **Step 1: Write the failing test**

```rust
/// The typed channel: each miss reason maps onto the same ByteOrEof the
/// trait-level `get` produced via from_raw (EOF at/past end and negative
/// positions, EOB on soft-ahead overrun, errors from the file).
#[test]
fn get_frombuffer_typed_misses() {
    let mut f = JFileAhead::new(Cursor::new(data(256)), "Tst", 64, 16).expect("test alloc");
    assert_eq!(f.get_frombuffer(-1, ReadType::Read), ByteOrEof::Eof); // §21.17 gate
    assert_eq!(f.get_frombuffer(300, ReadType::Read), ByteOrEof::Eof); // past EOF
    assert_eq!(f.getbuf_off(1500, ReadType::SoftAhead).unwrap_err(), GetBufMiss::Eob);
    assert!(matches!(
        f.getbuf_off(100, ReadType::Read),
        Ok((_, _))
    ));
}
```

(Reuse the file's existing `data(n)` helper and adjust positions to the ones the
neighboring `getbuf_off` tests already prove — ~1135–1186 pin the exact miss
positions; copy those.)

- [x] **Step 2: Verify it fails**

Run: `cargo test --all-features get_frombuffer_typed_misses`
Expected: FAIL to compile — arity/type mismatch.

- [x] **Step 3: Reshape the channel**

- `getbuf_off`: delete the `len: &mut i64` parameter. The four early-return arms
  (`EndOfBuffer`/`EndOfFile`/`SeekError`/`ReadError` from `get_fromfile`, plus
  the EOF/negative gate) return `Err(GetBufMiss::…)`; the success path returns
  `Ok((off, az_len))` keeping today's two-length computation exactly (including
  the `az_len` reassignment in the wrap branch and the `*len` write it fed).
- `get_frombuffer`: match on the `Result`, mapping `Err` to
  `ByteOrEof::Eof/Eob/Err(JDiffError::Seek)/Err(JDiffError::Read)` — i.e. inline
  what `ByteOrEof::from_raw` did at the old call site, minus the i32 round-trip.
  On success keep the `pos_red`/`red_sze`/`ptr_red` updates and return
  `ByteOrEof::Byte(self.buf[off])`. The `len` value now arrives in the `Ok`
  tuple (it feeds `red_sze = len - 1` and the debug `verify_buffer` call).
- `get` at ~666: `self.get_frombuffer(pos, typ)` — the `from_raw` wrapper call
  disappears from this site (it stays for `mem.rs`/`pch_get` boundaries).
- `getbuf` at ~707: `let (off, _) = self.getbuf_off(pos, typ).ok()?;` (the
  slice length already speaks for `len`, per its existing comment).
- Update the three direct `getbuf_off` test call sites (~1137, ~1140, ~1186) to
  the `Result` form.
- In the same commit, align the debug-only lock idiom at ~323:
  `let mut lc_tst = LC_TST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);`
  (matching `jdebug.rs` — a poisoned debug scratch buffer must not abort the run).

- [x] **Step 4: Verify green + full gate**

The ahead-buffer tests (~733–1190) and every golden/oracle run exercise this
channel on every byte read.

- [x] **Step 5: Commit**

```bash
git add src/jfile/ahead.rs
git commit -m "refactor: typed internal get channel in JFileAhead (GetBufMiss, no &mut len)"
```

---

### Task 8: Final verification and completion audit

**Files:** none (verification only).

- [x] **Step 1: Full gate, bare, judged on exit code**

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all three exit 0; test count ≥ 253 + the four new tests from Tasks 3–7.

- [x] **Step 2: Zero-byte-diff proof**

`git diff --stat main` must show NO files under `tests/fixtures/` (goldens
untouched) and no `*.stderr`/`*.jdf` changes anywhere.

- [x] **Step 3: Oracle layer (if the oracle binary is available)**

```bash
scripts/build-oracle.sh   # only if target/oracle is absent
JOJODIFF_ORACLE=target/oracle cargo test --all-features --test oracle
```

Expected: PASS (skip gracefully if the oracle cannot build — CI runs it).

- [x] **Step 4: Grep residue checks**

```bash
grep -rn 'marker().unwrap()' src/          # expect none
grep -rn '#\[allow(' src/                  # expect none (all are #[expect] now)
grep -rn '\.get(.*&mut' src/jdiff.rs src/jhashpos.rs  # expect none
grep -rn 'getbuf_off(.*&mut' src/          # expect none
grep -rnE '\bunsafe\b' src/                # doc comments only
```

- [x] **Step 5: Update this plan's checkboxes and commit any doc touch-ups**

```bash
git add docs/superpowers/plans/2026-10-05-rust-pitfall-audit.md
git commit -m "docs: mark rust pitfall audit plan tasks complete"
```

---

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| API-shape change drifts buffer/offset math (Tasks 4, 6, 7) | Each task's struct/enum carries the SAME values the old channels carried; golden + oracle suites byte-compare the result; per-task full gate |
| `put_len` restructure alters patch bytes (Task 3) | Pin test written FIRST and passing on old code; arm-local `if let` keeps emission order identical |
| `#[expect]` unfulfilled after future refactors | That is the feature — the build then fails loudly and the suppression is deleted with its reason |
| Breaking the young lib API (`JHashPos::get`, `hash_key` visibility) | Design §2.3 reserves lib-API stability for 1.0.0; the CLI surface (the user's compat contract) is untouched. `hash_key` itself stays public and unchanged |
| MSRV creep | No new APIs beyond 1.85; `#[expect]` is 1.81+; gate runs on the pinned toolchain |

## Out of scope (do not do)

- Any behavioral change, including §21-ruled quirks (wrapping counters, lossy
  `c_atoi`, SIGFPE-parity division, dead stores).
- Iterator conversions of ported engine loops (design §15 ITER-01 do-not-fix).
- `p8()` allocation work (deferred to an MSRV bump, design §15).
- Replacing the JOut writer `expect`s (policy-pinned via `IgnoringWriter`).
- Performance work, dependency changes, version bumps.

---

## Appendix A: Full audit record (2026-10-05)

Method: full read of `src/` (all 27 files) plus pattern scans
(`unsafe|static mut|RefCell|mem::|transmute|from_raw_parts|as_ptr`, `unwrap|expect|panic`,
`as (i8..f64)`, `wrapping_|checked_|saturating_`, `%|/`, `#[allow]`, `let mut`,
`env::args|lossy`), cross-checked against the 0.9.0 refactor design §15 rulings.
Baseline at audit time: `main` @ `54d2b98`, build/fmt/clippy green, 253 tests green.

### Acted on: F1–F4, F7, F9–F11 (table above)

### Ruled out, with reasons

| Candidate | Verdict | Reason |
|---|---|---|
| Engine loop `&mut` threading (`scan_equal_run`, `flush_eql`, `check`) | keep | C++ by-reference shape is the port policy (design §15, ITER-01); anchors documented |
| `p8()` per-call `String` allocation | keep | Ruled deferred to MSRV ≥ 1.97 (`NumBuffer::format_into`, design §15) |
| JOut `write_fmt(...).expect("write error")` (asc/rgn/bin) | keep | Diff path sits on `IgnoringWriter` (deviation 3); writers document the policy; patch path checks writes → exit 9 |
| `to_string_lossy()` on argv tokens (opts/config/error) | keep | "Lossy `c_atoi`" is part of the pinned grammar (design §2); operands/detached args stay raw `OsString` (spec line 566) |
| Pointer-format debug traces (`as_ptr().wrapping_add`, `as_ptr_range`) | keep | Safe (no deref), debug-feature-only, shape-pinned (jdebug.rs module docs) |
| `build_full_index`'s `let Self { org, hsh, verbose, .. } = self` | keep | Sanctioned field-disjoint borrow split in a focused fn; `search`'s instances were dissolved by the 0.9.0 refactor as planned |
| Ported narrowing `as` casts (labeled `C++: int assignment` sites, `putc(i32)`, `li_dbl as u8`) | keep | Byte-pinned C semantics, each documented at its site |
| Dead stores in `JPatcht::jpatch` | keep | Ported-as-written (`#[allow(unused_assignments)]`, extensively documented) |
| `dist()` division-by-zero panic | keep | §21-ruled SIGFPE parity; unreachable from shipped call sites |
| `opts.rs` `Vec<char>` collect per cluster char | keep | argv-scale cost; changing to byte-scan would alter pinned error text for multibyte inputs |
| `getbuf` out-parameter (trait level) | keep | §15: audited for removal in P3 and answered by the slice-length-speaking-for-len design |
| Integer overflow audit | clean | All ported wrap sites use `wrapping_*` (§21.6); i64 math elsewhere is range-safe; dev-profile tests are the canary |
| `LC_TST.lock().unwrap()` | fix (Task 7) | Debug-only, but inconsistent with the crate's own poison-recovery idiom |
| Cargo `future-incompatible` key | fix (Task 1) | Deprecated alias; the tree's only build warnings |

### Positive findings (patterns to preserve)

- Zero `unsafe`, zero `static mut` (AGENTS.md hard rules hold; `GB_DBG` uses
  `Mutex<[bool;16]>`, `jdebug.rs:84`).
- Borrow-checker cooperation, not bypass: `mem::take` (opts.rs:312),
  `mem::replace` for error-move-out (jdiff.rs:978), `PoisonError::into_inner`
  (jdebug.rs:91, lib.rs:83), field-disjoint destructuring (jdiff.rs:1065).
- Overflow-safe where C++ was not: `c_atoi` saturates, `is_prime` divides instead
  of `i*i`, `try_zeroed_vec` maps allocation failure to `JDiffError::Memory`
  (exit 10), `run.rs mb()` uses `TryFrom` + clamp.
- `wire.rs` single-source tier knowledge; `error.rs` exemplary thiserror surface
  (`#[non_exhaustive]`, `Send + Sync` lock test, byte-pinned Display tests).
