# Refactor Phase 1 — Mechanical De-duplication — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate the audit's nine duplication clusters (spec §5) as behavior-preserving, green-to-green transformations — no bytes change.

**Architecture:** One extraction per task, each landing as its own commit behind the full test gate. All code stays in its current module (the CLI restructure is Phase 4); only shared *decisions* move into single definitions (`print_char`, the wire-length tier classification, output-open/writer-wrap helpers, list-merge, equal-run scan, bucket unlink).

**Tech Stack:** Rust (edition 2024, MSRV 1.85), std only — **no new dependencies in this phase** (thiserror/anyhow arrive in Phase 3, tempfile/assert_cmd in Phase 4).

**Spec:** `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` — §5 (work list), §3 (invariants), §10 (rails). The port spec `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` §21 remains the behavioral authority.

## Global Constraints

- Work inside the worktree root: `/home/arnim/Projects/jojodiff-cli-rs/.worktrees/refactor-idiomatic-rust` (branch `refactor/idiomatic-rust`). Run every command from that directory; all file paths below are relative to it.
- **Bytes are pinned:** patch/listing/verbose output, stdout/stderr text, and exit codes must not change. **Never edit anything under `tests/fixtures/`** — a `git status` showing fixture changes means the task went wrong; revert.
- Per-commit gate (run all three before every commit, all must pass):
  `cargo fmt --all && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-features`
  (Full suite ≈ 3 min, 204 tests, 8 suites. Targeted runs during a task are fine; the gate is what commits.)
- Bounded loops only in any test touching temp dirs; no unbounded retries; cap generated artifacts.
- §21 do-not-fix markers stay: JOutRgn's 4/8 length-size return values (§21.14) remain a **local** deviation at its call site; dead branches not listed in Task 9 stay put.
- Conventional commits, one transformation per commit, imperative description.
- C++ anchor comments travel with the code they describe (metadata policy, spec §4).

---

### Task 1: Consolidate the printable-ASCII char filter into `defs::print_char`

**Files:**
- Modify: `src/defs.rs` (add `print_char` + test, in the helpers section after `c_atoi`)
- Modify: `src/jdebug.rs:103-113` (delete `c_chr`; update its caller and test)
- Modify: `src/jout/asc.rs:57-65` (delete `chr`; update call sites in `put`)
- Modify: `src/jpatcht.rs:144-149` (replace the inline filter in `uf_put_dta`)

**Interfaces:**
- Produces: `pub fn print_char(v: i32) -> char` in `crate::defs` — the C `%c` JojoDiff filter: the byte itself when `32 <= v <= 127`, a space otherwise.

- [ ] **Step 1: Write the failing test**

In `src/defs.rs` `mod tests`, add:

```rust
    /// The C `%c` printable-ASCII filter shared by the DBGCMP trace, the
    /// ASCII listing and the patch verbose trace (`JMatchTable.cpp:857-864`).
    #[test]
    fn print_char_matches_c_percent_c_filter() {
        assert_eq!(print_char(32), ' ');
        assert_eq!(print_char(65), 'A');
        assert_eq!(print_char(127), '\u{7f}');
        assert_eq!(print_char(31), ' ');
        assert_eq!(print_char(128), ' ');
        assert_eq!(print_char(-1), ' ');
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib defs::tests::print_char`
Expected: FAIL — `cannot find function print_char in this module`.

- [ ] **Step 3: Implement in `src/defs.rs`** (after `c_atoi`, before `mod tests`)

```rust
/// C `%c` with JojoDiff's printable-ASCII filter: the byte itself when
/// `32 <= v <= 127`, a space otherwise (shared by the DBGCMP result trace
/// `JMatchTable.cpp:857-864`, the ASCII listing `JOutAsc.cpp:53-54`, and
/// the patch verbose trace `JPatcht.cpp:104-106`).
pub fn print_char(v: i32) -> char {
    if (32..=127).contains(&v) {
        char::from_u32(v as u32).unwrap_or(' ')
    } else {
        ' '
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib defs::tests::print_char`
Expected: PASS.

- [ ] **Step 5: Swap the three call sites and delete the duplicates**

  1. `src/jout/asc.rs`: delete `fn chr` (lines 57-65). In `put`'s body every `Self::chr(x)` becomes `crate::defs::print_char(x)` (or add `print_char` to the existing `use crate::defs::{...}` list and call `print_char(x)`).
  2. `src/jpatcht.rs` `uf_put_dta`: replace the inline `if (32..=127).contains(&ai_dta) { ... } else { ' ' }` expression with `crate::defs::print_char(ai_dta)` (add to the existing `use crate::defs::{...}`).
  3. `src/jdebug.rs`: delete `pub fn c_chr` (lines 103-113). Its DBGCMP call site becomes `crate::defs::print_char(v)`. If a `jdebug` unit test covered `c_chr`, move that test's assertions into the `defs` test above and delete it.

- [ ] **Step 6: Full gate + commit**

Run the per-commit gate. Expected: 204 tests pass, clippy clean.
`git status` must show no fixture changes.

```bash
git add src/defs.rs src/jdebug.rs src/jout/asc.rs src/jpatcht.rs
git commit -m "refactor: consolidate the printable-ASCII filter into defs::print_char"
```

---

### Task 2: Shared wire-length tier module in `jout`

**Files:**
- Create: `src/jout/wire.rs`
- Modify: `src/jout/mod.rs` (add `pub mod wire;`)
- Modify: `src/jout/bin.rs:131-163` (`put_len` classification via the module)
- Modify: `src/jout/asc.rs:67-83` (`put_sze` becomes a one-liner over the module)
- Modify: `src/jout/rgn.rs:71-87` (`put_len` maps the tier to the §21.14 quirk values)
- Modify: `src/jpatcht.rs:90-124` (`uf_get_int` classifies via `LenTier::from_lead`)

**Interfaces:**
- Produces (in `crate::jout::wire`, `pub(crate)` visibility):

```rust
pub(crate) enum LenTier { L252, L508, L16, L32, L64 }
pub(crate) fn len_tier(len: i64) -> LenTier
impl LenTier {
    pub(crate) fn from_lead(lead: i64) -> LenTier
    /// Real encoded size: 1/2/3/5/9 bytes.
    pub(crate) fn size(self) -> i64
    /// Lead/marker byte for the multi-byte tiers: 252/253/254/255; L252 has
    /// no constant marker (its byte is the value itself).
    pub(crate) fn marker(self) -> Option<u8>
}
```

Tier semantics (boundaries are the shared decision, byte-exact with the C++):
`len <= 252` → L252; `<= 508` → L508; `<= 0xffff` → L16; `<= 0xffff_ffff` → L32; else L64.
`from_lead`: `lead < 252` → L252; `252` → L508; `253` → L16; `254` → L32; else (incl. 255 and negatives other than covered) → L64. **Note:** `from_lead(-1)` (the decoder's EOF arithmetic) must land in L252 so `uf_get_int` keeps computing `0` for it, exactly as today's `li_val < 252` arm does.

- [ ] **Step 1: Write the failing tests**

Create `src/jout/wire.rs` with only the tests (implementation comes in Step 3):

```rust
//! Shared wire-length tier knowledge for the patch format: the length
//! classification (`JOutBin::ufPutLen` `JOutBin.cpp:65-104`), the real
//! encoded sizes 1/2/3/5/9 (`JOutAsc::ufPutSze` `JOutAsc.cpp:107-126`), and
//! the decoder's lead-byte classification (`JPatcht::ufGetInt`
//! `JPatcht.cpp:50-85`). One definition so encoder, size calculators and
//! decoder cannot drift apart.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_tier_boundaries() {
        assert!(matches!(len_tier(1), LenTier::L252));
        assert!(matches!(len_tier(252), LenTier::L252));
        assert!(matches!(len_tier(253), LenTier::L508));
        assert!(matches!(len_tier(508), LenTier::L508));
        assert!(matches!(len_tier(509), LenTier::L16));
        assert!(matches!(len_tier(0xffff), LenTier::L16));
        assert!(matches!(len_tier(0x1_0000), LenTier::L32));
        assert!(matches!(len_tier(0xffff_ffff), LenTier::L32));
        assert!(matches!(len_tier(0x1_0000_0000), LenTier::L64));
    }

    #[test]
    fn real_sizes() {
        assert_eq!(LenTier::L252.size(), 1);
        assert_eq!(LenTier::L508.size(), 2);
        assert_eq!(LenTier::L16.size(), 3);
        assert_eq!(LenTier::L32.size(), 5);
        assert_eq!(LenTier::L64.size(), 9);
    }

    #[test]
    fn from_lead_classification() {
        assert!(matches!(LenTier::from_lead(0), LenTier::L252));
        assert!(matches!(LenTier::from_lead(251), LenTier::L252));
        assert!(matches!(LenTier::from_lead(-1), LenTier::L252)); // EOF arithmetic
        assert!(matches!(LenTier::from_lead(252), LenTier::L508));
        assert!(matches!(LenTier::from_lead(253), LenTier::L16));
        assert!(matches!(LenTier::from_lead(254), LenTier::L32));
        assert!(matches!(LenTier::from_lead(255), LenTier::L64));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib jout::wire`
Expected: FAIL — unresolved `LenTier`/`len_tier`.

- [ ] **Step 3: Implement the module** (above `mod tests` in `src/jout/wire.rs`)

```rust
/// The five wire-length tiers of the patch format (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LenTier {
    /// 1..=252 — encoded as the length minus one in a single byte.
    L252,
    /// 253..=508 — marker 252 plus one byte.
    L508,
    /// 16-bit tier — marker 253 plus two bytes.
    L16,
    /// 32-bit tier — marker 254 plus four bytes.
    L32,
    /// 64-bit tier (`JDIFF_LARGEFILE`, live in the oracle and the port) —
    /// marker 255 plus eight bytes.
    L64,
}

/// Which tier a length encodes into (`JOutBin.cpp:65-104` boundaries).
pub(crate) fn len_tier(len: i64) -> LenTier {
    if len <= 252 {
        LenTier::L252
    } else if len <= 508 {
        LenTier::L508
    } else if len <= 0xffff {
        LenTier::L16
    } else if len <= 0xffff_ffff {
        LenTier::L32
    } else {
        LenTier::L64
    }
}

impl LenTier {
    /// Which tier a decoder lead byte introduces (`JPatcht.cpp:50-85`).
    /// Negative leads classify as L252 so the C arithmetic
    /// (`li_val = EOF` → `li_val + 1 = 0`) is preserved.
    pub(crate) fn from_lead(lead: i64) -> LenTier {
        match lead {
            i64::MIN..=251 => LenTier::L252,
            252 => LenTier::L508,
            253 => LenTier::L16,
            254 => LenTier::L32,
            _ => LenTier::L64,
        }
    }

    /// Real encoded size in bytes: 1/2/3/5/9.
    pub(crate) fn size(self) -> i64 {
        match self {
            LenTier::L252 => 1,
            LenTier::L508 => 2,
            LenTier::L16 => 3,
            LenTier::L32 => 5,
            LenTier::L64 => 9,
        }
    }

    /// Marker byte of the multi-byte tiers; `None` for L252 (its single
    /// byte is the length itself, minus one).
    pub(crate) fn marker(self) -> Option<u8> {
        match self {
            LenTier::L252 => None,
            LenTier::L508 => Some(252),
            LenTier::L16 => Some(253),
            LenTier::L32 => Some(254),
            LenTier::L64 => Some(255),
        }
    }
}
```

Add `pub mod wire;` to `src/jout/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib jout::wire`
Expected: PASS (3 tests).

- [ ] **Step 5: Rewire the four sites**

  1. `src/jout/bin.rs` `put_len` (lines 131-163): classify once with `let tier = wire::len_tier(len);`, keep the existing `self.raw(...)` byte sequences and `self.stats.ctl` increments exactly as they are (the tier boundaries must now come from `tier`, e.g. `match tier { ... }` arms replacing the `if len <= ...` chain). Do **not** change any emitted byte.
  2. `src/jout/asc.rs` `put_sze` (lines 67-83) becomes:

```rust
    /// Length encoding size in the binary format (`JOutAsc::ufPutSze`,
    /// `JOutAsc.cpp:107-126`): 1/2/3/5/9 bytes.
    fn put_sze(len: i64) -> i64 {
        super::wire::len_tier(len).size()
    }
```

  3. `src/jout/rgn.rs` `put_len` (lines 71-87) becomes — the §21.14 quirk stays **local and documented**:

```rust
    /// `JOutRgn::ufPutLen` (`JOutRgn.cpp:120-139`): length-encoding size.
    /// Unlike the real encoding tiers (1/2/3/5/9) the 32/64-bit tiers here
    /// return **4** and **8** — an upstream inconsistency ported as written
    /// (spec §21.14), deliberately kept local to this quirk site.
    fn put_len(len: i64) -> i64 {
        match super::wire::len_tier(len) {
            super::wire::LenTier::L32 => 4,
            super::wire::LenTier::L64 => 8,
            tier => tier.size(),
        }
    }
```

  4. `src/jpatcht.rs` `uf_get_int` (lines 90-124): replace the `if li_val < 252 / == 252 / == 253 / == 254 / else` chain with a `match wire::LenTier::from_lead(li_val)` whose arms keep the exact existing arithmetic bodies (including the dead-code comment on the 64-bit arm). The `li_val + 1` of the first arm must use the original `li_val`, not the tier.

- [ ] **Step 6: Full gate + commit**

Run the per-commit gate. The golden85/oracle suites are the proof the bytes did not move.

```bash
git add src/jout/wire.rs src/jout/mod.rs src/jout/bin.rs src/jout/asc.rs src/jout/rgn.rs src/jpatcht.rs
git commit -m "refactor: one shared wire-length tier definition for encoder, sizes and decoder"
```

---

### Task 3: Characterization test — output-open failure path (test-first)

**Files:**
- Test: `tests/roundtrip.rs` (add one test near the other exit-code tests)

**Interfaces:**
- Produces: a pinned subprocess test proving the output-open failure message and exit code 5 (`-EXI_OUT`) that Task 4's helper must preserve.

- [ ] **Step 1: Write the test**

```rust
/// Output-open failure: unwritable output path prints the pinned message
/// and exits 5 (`-EXI_OUT`, `main.cpp:754-774`).
#[test]
fn output_open_failure_message_and_exit_code() {
    let dir = std::env::temp_dir().join(format!(
        "jdiff-outfail-{}",
        std::process::id()
    ));
    // A path under a directory that does not exist: File::create must fail.
    let out = dir.join("no").join("out.jdf");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .arg("a")
        .arg("b")
        .arg(&out)
        .output()
        .expect("spawn jdiff");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not open output file ")
            && stderr.contains(" for writing."),
        "unexpected stderr: {stderr:?}"
    );
    assert_eq!(output.status.code(), Some(5));
    // Bounded cleanup: the dir was never created, nothing to remove.
}
```

Use the file-pair argument style the surrounding tests use for inputs `a`/`b` — if other tests build input files first, mirror their fixture pattern (e.g. the `test2` fixtures) instead of literal `"a"`/`"b"`; the assertion only cares about the output path failing, so any existing input pair works.

- [ ] **Step 2: Run it to verify it passes against current code**

Run: `cargo test --test roundtrip output_open_failure`
Expected: PASS — this pins current behavior; it must stay green through Task 4.

- [ ] **Step 3: Full gate + commit**

```bash
git add tests/roundtrip.rs
git commit -m "test: pin output-open failure message and exit 5"
```

---

### Task 4: `open_output_file` and `wrap_buffered` helpers in the CLI

**Files:**
- Modify: `src/bin/jdiff.rs` — add both helpers near `open_inputs`; replace the three output-open blocks (lines 445-454, 658-667, 669-678) and the two writer-wrapping sites (lines 498-503, 688-693)

**Interfaces:**
- Consumes: Task 3's pinned test.
- Produces (private to the binary; both are absorbed into the `cli` module in Phase 4):

```rust
fn open_output_file(nam_out: &OsStr, append: bool) -> File
fn wrap_buffered<W: Write + 'static>(sink: W, out_is_stdout: bool) -> Box<dyn Write>
```

- [ ] **Step 1: Add the helpers** (after `open_inputs`)

```rust
/// Opens the output file like the C++ (`main.cpp:754-774`): on failure
/// prints the pinned message and exits `-EXI_OUT` (5). `append` selects
/// the `-t` reopen path (the same FILE* appended after the diff output).
fn open_output_file(nam_out: &OsStr, append: bool) -> File {
    let attempt = if append {
        File::options().append(true).open(nam_out)
    } else {
        File::create(nam_out)
    };
    match attempt {
        Ok(file) => file,
        Err(_) => {
            dbg_print(format_args!(
                "Could not open output file {} for writing.\n",
                nam_out.to_string_lossy()
            ));
            exit(-EXI_OUT);
        }
    }
}

/// The diff/patch writer-buffering decision: with `-c` and a stdout patch
/// the raw sink is used so patch bytes and verbose lines share one ordered
/// buffer (the C++ single `FILE*`); otherwise a `BufWriter` batches the
/// per-byte writes and flushes at scope end (the C++ exit-time flush).
fn wrap_buffered<W: Write + 'static>(sink: W, out_is_stdout: bool) -> Box<dyn Write> {
    if out_is_stdout && DBG_TO_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(sink)
    } else {
        Box::new(BufWriter::new(sink))
    }
}
```

Add `use std::ffi::OsStr;` if not already imported (`OsString` is; `OsStr` may not be).

- [ ] **Step 2: Replace the five sites**

  1. Diff-side open (lines 445-454): the `File::create(&nam_out)` match arm body becomes
     `Some(Sink::File(open_output_file(&nam_out, false)))` — keep the surrounding `out_is_stdout`/Dedup branches as they are.
  2. Patch-side Test reopen (lines 658-667): the `File::options().append(true).open(&nam_out)` match becomes `Sink::File(open_output_file(&nam_out, true))`.
  3. Patch-side create (lines 669-678): becomes `Sink::File(open_output_file(&nam_out, false))`.
  4. Diff writer wrap (lines 498-503): becomes

```rust
        let writer: Box<dyn Write> =
            wrap_buffered(IgnoringWriter { inner: out_sink }, out_is_stdout);
```

  (the `IgnoringWriter` stays at this call site — it is the diff-path policy, spec §3.)
  5. Patch writer wrap (lines 688-693): becomes

```rust
        let patch_writer: Box<dyn Write> = wrap_buffered(patch_sink, out_is_stdout);
```

- [ ] **Step 3: Full gate + commit** (Task 3's test must stay green)

```bash
git add src/bin/jdiff.rs
git commit -m "refactor: extract open_output_file and wrap_buffered in the CLI"
```

---

### Task 5: `JMatchTable::merge_new_into_old`

**Files:**
- Modify: `src/jmatchtable.rs` — add the method near `add_new` (line ~1193); replace the two verbatim blocks (lines 534-543 in `getbest`, 631-639 in `cleanup`)

**Interfaces:**
- Produces: `fn merge_new_into_old(&mut self)` — private method; `nextold`'s third variant (line ~1048) is deliberately **not** unified (unverified same-decision; spec §5.5).

- [ ] **Step 1: Add the method**

```rust
    /// Join the new list into the old list (`JMatchTable.cpp:379-385`,
    /// duplicated at `:124-131`): the old list is appended after the new
    /// list's last element, then the new-list bookkeeping resets.
    fn merge_new_into_old(&mut self) {
        if self.mp_new.is_some() {
            let lst = self
                .mp_lst
                .expect("mpNew non-null implies mpLst non-null (C++ invariant)");
            self.nodes[lst].nxt = self.mp_old;
            self.mp_old = self.mp_new;
            self.mp_new = None;
            self.mp_lst = None;
        }
    }
```

- [ ] **Step 2: Replace both blocks**

In `getbest` (the `if !self.cmp_all` arm) and in `cleanup` (after `self.rlb = rlb;`), replace the eight-line `if self.mp_new.is_some() { ... }` blocks with:

```rust
            self.merge_new_into_old();
```

Keep the preceding comments (`// join old and new lists` and its `:379-385` anchor) at the call sites.

- [ ] **Step 3: Full gate + commit**

The 11 `jmatchtable` unit tests plus the oracle suites pin this path.

```bash
git add src/jmatchtable.rs
git commit -m "refactor: extract JMatchTable::merge_new_into_old from getbest/cleanup"
```

---

### Task 6: Equal-run fast-loop extraction in `JDiff::jdiff`

**Files:**
- Modify: `src/jdiff.rs` — add `scan_equal_run`; replace the two loops (lines 324-352)

**Interfaces:**
- Produces:

```rust
fn scan_equal_run(
    &mut self,
    pos_org: &mut i64,
    val_org: &mut i32,
    pos_new: &mut i64,
    val_new: &mut i32,
    lap_sml: i64,
    index_src: bool,
) -> i64
```

- [ ] **Step 1: Add the method** (in `impl JDiff`, near `flush_eql`)

```rust
    /// The equal-run fast loop (`JDiff.cpp:201-224`): counts and consumes
    /// equal bytes up to the small-lap limit. `index_src` is the
    /// `src_scn == 0` mode's incremental source indexing — the only
    /// difference between the C++'s two loops.
    fn scan_equal_run(
        &mut self,
        pos_org: &mut i64,
        val_org: &mut i32,
        pos_new: &mut i64,
        val_new: &mut i32,
        lap_sml: i64,
        index_src: bool,
    ) -> i64 {
        let mut cnt: i64 = 0;
        while *val_org == *val_new && *val_new >= 0 && *pos_new < lap_sml {
            cnt += 1;
            if index_src && *pos_org == self.az_org {
                self.hash_add_org(*val_org);
            }
            *pos_org += 1;
            *val_org = self.org.get(*pos_org, ReadType::Read);
            *pos_new += 1;
            *val_new = self.r#new.get(*pos_new, ReadType::Read);
        }
        cnt
    }
```

- [ ] **Step 2: Replace the two loops**

The `else if self.src_scn == 0 { ... } else { ... }` pair (lines 324-352) becomes:

```rust
                } else {
                    /* fast loop (JDiff.cpp:201-224): the src_scn==0 variant
                     * incrementally indexes the source. */
                    let lz_cnt = self.scan_equal_run(
                        &mut lz_pos_org,
                        &mut lc_org,
                        &mut lz_pos_new,
                        &mut lc_new,
                        lz_lap_sml,
                        self.src_scn == 0,
                    );
                    lz_eql += lz_cnt; // increase equal counter
                    lz_ahd -= lz_cnt; // decrease ahead counter
                }
```

(Collapse the two arms into one — the mode difference is now the `index_src` argument.)

- [ ] **Step 3: Full gate + commit** (the `-s` on/off matrix in the golden suites covers both modes)

```bash
git add src/jdiff.rs
git commit -m "refactor: unify the equal-run fast loops into JDiff::scan_equal_run"
```

---

### Task 7: Parameterized bucket unlink (`del_gld`/`del_col`)

**Files:**
- Modify: `src/jmatchtable.rs` — add `Link` enum + `del_bucket` free function; `del_gld`/`del_col` (lines 1207-1243) become index-computing wrappers

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy)]
enum Link { Gld, Col }
impl Link {
    fn get(self, n: &Node) -> Option<usize>
    fn set(self, n: &mut Node, v: Option<usize>)
}
fn del_bucket(nodes: &mut [Node], tbl: &mut [Option<usize>], cur: usize, idx: usize, link: Link)
```

- [ ] **Step 1: Add the enum and function** (module scope, near `Node`)

```rust
/// The two bucket-chain link fields of [`Node`] — the gliding and the
/// colliding hashtable (`JMatchTable.h`), the only difference between
/// `delGld` and `delCol`.
#[derive(Clone, Copy)]
enum Link {
    Gld,
    Col,
}

impl Link {
    fn get(self, n: &Node) -> Option<usize> {
        match self {
            Link::Gld => n.gld,
            Link::Col => n.col,
        }
    }
    fn set(self, n: &mut Node, v: Option<usize>) {
        match self {
            Link::Gld => n.gld = v,
            Link::Col => n.col = v,
        }
    }
}

/// Unlink `cur` from the bucket chain rooted at `tbl[idx]`
/// (`delGld`/`delCol` common shape, `JMatchTable.cpp:893-926`). The last
/// node's dangling `nxt` convention is unaffected — this only relinks the
/// bucket chain.
fn del_bucket(nodes: &mut [Node], tbl: &mut [Option<usize>], cur: usize, idx: usize, link: Link) {
    if tbl[idx] == Some(cur) {
        tbl[idx] = link.get(&nodes[cur]);
    } else {
        let mut p = tbl[idx];
        while let Some(i) = p {
            if link.get(&nodes[i]) == Some(cur) {
                let nxt = link.get(&nodes[cur]);
                link.set(&mut nodes[i], nxt);
                break;
            }
            p = link.get(&nodes[i]);
        }
    }
}
```

- [ ] **Step 2: Rewrite both methods as wrappers**

```rust
    /// Delete element from gliding hashtable (`delGld`,
    /// `JMatchTable.cpp:893-907`).
    fn del_gld(&mut self, cur: usize) {
        // C++ `%` on a negative izOrg yields a negative index (UB); izOrg is
        // only ever filled from non-negative match positions.
        let idx = (self.nodes[cur].org % i64::from(self.mch_pme)) as usize;
        del_bucket(&mut self.nodes, &mut self.gld_tbl, cur, idx, Link::Gld);
    }

    /// Delete element from colliding hashtable (`delCol`,
    /// `JMatchTable.cpp:912-926`).
    fn del_col(&mut self, cur: usize) {
        let idx = (self.nodes[cur].dlt.abs() % i64::from(self.mch_pme)) as usize;
        del_bucket(&mut self.nodes, &mut self.col_tbl, cur, idx, Link::Col);
    }
```

- [ ] **Step 3: Full gate + commit**

```bash
git add src/jmatchtable.rs
git commit -m "refactor: parameterized bucket unlink shared by del_gld/del_col"
```

---

### Task 8: Remove the per-character `String` clone in `Getopt`

**Files:**
- Modify: `src/cli/opts.rs` — add `scan_cluster`; replace lines 266 and 290-300 in `next_opt`

**Interfaces:**
- Produces: `fn scan_cluster(&mut self) -> Option<Opt>` — private; moves (not clones) the cluster out for the `&mut self` scan and restores it.

- [ ] **Step 1: Add the helper** (near `scan_short`)

```rust
    /// Scans the next option from `self.cluster` without cloning it per
    /// character: the token is moved out so `scan_short` can borrow `&mut
    /// self` and the cluster text simultaneously, then restored.
    fn scan_cluster(&mut self) -> Option<Opt> {
        let tok = std::mem::take(&mut self.cluster);
        let opt = self.scan_short(&tok);
        self.cluster = tok;
        opt
    }
```

- [ ] **Step 2: Replace the two sites in `next_opt`**

  1. Line 266 (`match self.scan_short(&self.cluster.clone())`) becomes:

```rust
                match self.scan_cluster() {
```

  2. Lines 290-300 (the short-cluster start) — replace `self.cluster = bytes.clone();` + `if let Some(opt) = self.scan_short(&bytes)` with:

```rust
                self.nextchar = 1;
                self.cluster = bytes;
                if let Some(opt) = self.scan_cluster() {
                    return opt;
                }
                continue;
```

  (keeping the surrounding `self.optind += 1;` and comment lines exactly).

- [ ] **Step 3: Run the Getopt tests, then full gate + commit**

Run: `cargo test --lib cli::opts`
Expected: all 10 pass. Then the per-commit gate.

```bash
git add src/cli/opts.rs
git commit -m "refactor: Getopt scans short clusters without a per-character clone"
```

---

### Task 9: Dead-code removal (reachability-proven items only)

**Files:**
- Modify: `src/jfile/mod.rs:56-60` — delete the `get_buf_sze` trait method (and its doc comment)
- Modify: `src/jfile/ahead.rs:659` — delete the `JFileAhead::get_buf_sze` impl; `src/jfile/ahead.rs:904` — delete the `assert_eq!(f.get_buf_sze(), 1024)` line (keep the surrounding test if it asserts more; if that was its only assertion, delete the test)
- Modify: `src/jmatchtable.rs:86-91` — delete `pub const MAXGLD`; `src/jmatchtable.rs:31` — rewrite the module-doc bullet that mentions it (see below)
- **Not removed:** the `COLLISION_LOW` branch (`jhashpos.rs:203-207`) — the spec §21.13 unreachable-ruling notwithstanding, the unit test at `jhashpos.rs:468` constructs `eql_cnt 65` and pins the decrement; reachability fails, the branch stays. Note this in the commit body.

**Interfaces:**
- Consumes: nothing.
- Produces: a smaller public trait (breaking, covered by the 0.9.0 release).

- [ ] **Step 1: Remove `get_buf_sze`**

Delete from `src/jfile/mod.rs`:

```rust
    /// Size of the buffer, `-1` = no buffering (C++ virtual with a `-1`
    /// default, `JFile.h:133`).
    fn get_buf_sze(&self) -> i64 {
        -1
    }
```

Delete the `JFileAhead` impl (ahead.rs:659-661, same shape) and the test assertion at ahead.rs:904. Reachability: `rg -n 'get_buf_sze' src/` must return only these three sites before deletion, zero after (tests excluded after your edit).

- [ ] **Step 2: Remove `MAXGLD`**

Delete from `src/jmatchtable.rs`:

```rust
/// Max distance for gliding matches (`MAXGLD`, `JMatchTable.cpp:42`).
pub const MAXGLD: i32 = 128;
```

Rewrite the module-doc bullet at line 31 from (paraphrasing — match the actual sentence) "…`miAhdMax` (stored, never read) and `MAXGLD` are…" to mention only `miAhdMax`. Reachability: `rg -n 'MAXGLD' src/` returns only the const and the doc line before deletion, zero after.

- [ ] **Step 3: Full gate + commit**

```bash
git add src/jfile/mod.rs src/jfile/ahead.rs src/jmatchtable.rs
git commit -m "refactor: remove dead get_buf_sze and MAXGLD

get_buf_sze: zero non-test callers (the C++ virtual with -1 default was
parity surface only). MAXGLD: const with no readers. COLLISION_LOW
branch kept: pinned by the jhashpos eql_cnt-65 unit test, reachability
of the removal fails (spec 5.9 rule)."
```

---

### Task 10: Phase 1 close-out verification

**Files:** none (verification only)

- [ ] **Step 1: Confirm the acceptance surface**

```bash
git status --porcelain                    # must show NO tests/fixtures/ changes ever committed this phase
git log --oneline main..HEAD              # the nine refactor/test commits
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features                 # 205 tests (204 + Task 3's) pass
```

- [ ] **Step 2: Spot-check the duplication inventory**

```bash
rg -n "32..=127" src/                     # only defs::print_char remains
rg -n "len <= 508" src/                   # only jout/wire.rs remains
rg -n "Could not open output file" src/   # only open_output_file remains
rg -n "scan_short\(&self.cluster" src/    # zero hits
rg -n "nodes\[lst\].nxt = self.mp_old" src/  # only merge_new_into_old remains
```

- [ ] **Step 3: Report**

Commit nothing (or only the plan's checkbox updates if executing with checkbox tracking). Report: tests green, no fixture diffs, inventory consolidated — Phase 1 done, ready for the Phase 2 plan (engine surgery: `SearchState`, `get_fromfile` arms, `JMatchTable::add` split, `build_full_index` verification).

---

## Self-Review (completed during planning)

- **Spec coverage:** spec §5 items 1-8 → Tasks 1-8; §5.9 → Task 9 (with one reachability rejection documented); §10 rails → Global Constraints; Task 3 covers the "characterization-first" rule for the CLI helper extraction. Phases 2-5 are separate plans by design (scope check).
- **Placeholder scan:** no TBD/TODO; every code step carries the actual code; Task 4's "match the actual sentence" instruction for the module-doc rewrite is a verbatim-echo edit of one sentence, included as such because the executor must not paraphrase port docs freely.
- **Type consistency:** `print_char(i32) -> char` (Task 1) matches all three call sites; `LenTier::{len_tier, from_lead, size, marker}` (Task 2) used identically in Tasks 2's four sites; `scan_equal_run`'s `&mut` parameter shapes match the loop locals (`lz_pos_org: i64`, `lc_org: i32`, …); `Link::{get,set}` used by `del_bucket` only.
