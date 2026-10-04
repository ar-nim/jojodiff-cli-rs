# Refactor Phase 2 — Engine Structural Surgery — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the engine's worst shapes — `SearchState` extraction, `get_fromfile` arm split, `JMatchTable::add` step split, loop unifications — with zero behavior change.

**Architecture:** Four extractions inside the existing modules (no renames, no protocol changes, no new files). The borrow-checker workarounds die with `SearchState`; the long methods become readable matches over named steps.

**Tech Stack:** Rust (edition 2024, MSRV 1.85), std only — no new dependencies in this phase.

**Spec:** `docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md` §6 (this phase), §3 (invariants), §10 (rails).

## Global Constraints

- Same core rules as the Phase 1 plan (see `2026-10-03-refactor-p1-dedup.md`): worktree root `/home/arnim/Projects/jojodiff-cli-rs/.worktrees/refactor-idiomatic-rust`, branch `refactor/idiomatic-rust`; bytes/exit codes pinned; never touch `tests/fixtures/`; per-commit gate `cargo fmt --all && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-features`; conventional commits.
- **Line numbers in this plan reference the tree before Phase 1 landed.** Phase 1 shifts lines. Locate every site by the identifiers given (function/field names, quoted code) and verify with `rg` before editing; the quoted code blocks are the authority, the line numbers are hints.
- Ported field/identifier names do not change in this phase (the type overhaul is Phase 3).
- Split boundaries yield to C++ anchor spans: if a proposed step boundary would split a `J*.cpp:N-M` anchor's range mid-behavior, keep that block inline and note it in the commit body (spec §6.3).

---

### Task 1: `SearchState` extraction in `jdiff.rs`

**Files:**
- Modify: `src/jdiff.rs` — struct `JDiff` (fields at ~111-139), `search` (~515-942, the two `let Self {...}` destructures at ~551-562 and ~596-614), and every `self.<state-field>` reference in the file

**Interfaces:**
- Produces (private, in `src/jdiff.rs`):

```rust
/// Search-ahead state (`JDiff.h:264-274`): the rolling window `search`
/// advances. Extracted from `JDiff` so the scan methods can borrow the
/// state and the file readers disjointly — the C++ reaches into these
/// members through pointer aliasing, which the two field-disjoint
/// destructures used to emulate.
struct SearchState {
    /// Current ahead position on the original file (`mzAhdOrg`).
    az_org: i64,
    /// Current ahead position on the new file (`mzAhdNew`).
    az_new: i64,
    /// Current hash value for the original file (`mlHshOrg`).
    hsh_org: u32,
    /// Current hash value for the new file (`mlHshNew`).
    hsh_new: u32,
    /// Previous file value, original (`miPrvOrg`).
    prv_org: i32,
    /// Current file value, new (`miValNew`).
    val_new: i32,
    /// Previous file value, new (`miPrvNew`).
    prv_new: i32,
    /// Equal-run counter in the current sample, original (`miEqlOrg`).
    eql_org: i32,
    /// Equal-run counter in the current sample, new (`miEqlNew`).
    eql_new: i32,
    /// Reliability range for the current hashtable (`miRlb`).
    rlb: i32,
    /// Number of false hash hits (`miHshErr`), wrapping (spec §21.6).
    hsh_err: i32,
}
```

- `JDiff` gains `sst: SearchState`; the eleven listed fields move into it. `hsh_err()` accessor reads `self.sst.hsh_err`.

- [ ] **Step 1: Introduce the struct and move the fields**

Add `SearchState` (above), add `sst: SearchState` to `JDiff` in place of the eleven fields (keep each field's doc comment on the struct), initialize it in `JDiff::new` with the same values the fields get today. Then mechanically rewrite every reference: `self.az_org` → `self.sst.az_org`, `self.az_new` → `self.sst.az_new`, `self.hsh_org` → `self.sst.hsh_org`, `self.hsh_new` → `self.sst.hsh_new`, `self.prv_org` → `self.sst.prv_org`, `self.val_new` → `self.sst.val_new`, `self.prv_new` → `self.sst.prv_new`, `self.eql_org` → `self.sst.eql_org`, `self.eql_new` → `self.sst.eql_new`, `self.rlb` → `self.sst.rlb`, `self.hsh_err` → `self.sst.hsh_err`. Enumerate with `rg -n 'self\.(az_org|az_new|hsh_org|hsh_new|prv_org|val_new|prv_new|eql_org|eql_new|rlb|hsh_err)\b' src/jdiff.rs` — the mechanical rewrite must leave zero hits. Compile: `cargo check`. (The destructures will fail to compile — that is Step 2's input.)

- [ ] **Step 2: Kill both destructures in `search`**

The first block (~551-562) becomes direct field access — disjoint borrows now work because `self.sst` and `self.org`/`self.hsh` are different fields:

```rust
        } else if self.src_scn == 0 {
            // Set lookahead base position and determine lookahead range
            self.org.set_lookahead_base(red_org);
            let mut li_scan: i32 = if self.src_bkt {
                // Backtrace allowed: go ahead as far as possible
                self.ahd_max
            } else {
                // Backtrace not allowed:
                // - keep (mzAhdMax - azRedOrg) == miAhdMax / 2
                // - except at the start of the file (azRedOrg < miAhdMax)
                if self.sst.az_org < i64::from(self.ahd_max) / 2 {
                    // C++: int assignment of the off_t difference.
                    (self.ahd_max as i64 - self.sst.az_org) as i32
                } else {
                    (i64::from(self.ahd_max) / 2 - (self.sst.az_org - red_org)) as i32
                }
            };

            // scan ahead till EOB or EOF
            while li_scan > 0 {
                let lc_org = self.org.get(self.sst.az_org, ReadType::SoftAhead);
                if lc_org <= EOF {
                    break;
                }
                self.sst.hsh_org = hash_key(self.sst.hsh_org, &mut self.sst.prv_org, lc_org, &mut self.sst.eql_org);
                self.hsh.add(self.sst.hsh_org, self.sst.az_org, self.sst.eql_org);
                self.sst.az_org += 1;
                li_scan -= 1;
            }
            self.sst.rlb = self.hsh.reliability();
        } /* switch scan source file - build hashtable */
```

(Check `hash_key`'s actual parameter types at its definition — today's call passes `prv_org`/`eql_org` as `&mut` through the destructure; keep whatever the current call site does, only changing the paths.)

The second destructure (~596-614) is deleted outright; every use of its bindings (`mz_ahd_new`, `ml_hsh_new`, `mi_prv_new`, `mi_val_new`, `mi_eql_new`, `mi_rlb`, plus the collaborator names `org`, `r#new`, `hsh`, `mch`, `src_bkt`, `verbose`, `mch_max`, `mch_min`, `ahd_max`, and `_mz_ahd_org`) in the rest of `search` rewrites to the direct `self.…` / `self.sst.…` form: `*mz_ahd_new` → `self.sst.az_new`, `*ml_hsh_new` → `self.sst.hsh_new`, `*mi_prv_new` → `self.sst.prv_new`, `*mi_val_new` → `self.sst.val_new`, `*mi_eql_new` → `self.sst.eql_new`, `*mi_rlb` → `self.sst.rlb`, `*src_bkt` → `self.src_bkt`, `*verbose` → `self.verbose`, `*mch_max` → `self.mch_max`, `*mch_min` → `self.mch_min`, `*ahd_max` → `self.ahd_max`, `org` → `self.org`, `r#new` → `self.r#new`, `hsh` → `self.hsh`, `mch` → `self.mch`. Where the body calls `mch.add(...)` / `hsh.…` on the destructured references, the direct `self.mch.add(...)` now borrows `self` mutably for the call — if any single statement then also needs `&mut self.sst` simultaneously, split the statement into locals first (read state → call → write state). Preserve the `az_org: _mz_ahd_org` comment ("not reset on backtrack anymore (0.8.5)") at the corresponding point as a plain comment.

- [ ] **Step 3: Full gate + commit**

```bash
cargo fmt --all && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-features
git add src/jdiff.rs
git commit -m "refactor: extract JDiff SearchState, ending the field-disjoint destructures"
```

The 14 jdiff unit tests + golden/oracle suites pin the scan behavior on both `src_scn` paths.

---

### Task 2: `get_fromfile` strategy-arm split in `jfile/ahead.rs`

**Files:**
- Modify: `src/jfile/ahead.rs` — `get_fromfile` (~412-549)

**Interfaces:**
- Produces (private methods on `JFileAhead`): `fn reset_to(&mut self, pos: i64) -> BufDone`, `fn append_blocks(&mut self, pos: i64) -> BufDone`, `fn scroll_back(&mut self, pos: i64) -> BufDone`.

- [ ] **Step 1: Extract the three arms**

`get_fromfile` keeps its preparation/classification section verbatim (everything through the `match li_sek`) and the match becomes:

```rust
        match li_sek {
            BufOpr::Reset => self.reset_to(pos),
            BufOpr::Append => self.append_blocks(pos),
            BufOpr::Scrollback => self.scroll_back(pos),
        }
    }
```

The three new methods take the corresponding match-arm bodies **verbatim** (Reset = `JFileAhead.cpp:310-333` block incl. the sequential `pos_inp` branch; Append = `:335-339`; Scrollback = `:341-383` incl. the truncation-comment block and the final `@Seek`), each returning the arm's `BufDone` value and falling through to `BufDone::Added` at the end:

```rust
    /// Reset the buffer to serve `pos` (`JFileAhead.cpp:310-333`): seek to
    /// the block-aligned position and read anew.
    fn reset_to(&mut self, pos: i64) -> BufDone {
        /* … verbatim arm body … */
        BufDone::Added
    }
```

- [ ] **Step 2: Full gate + commit**

```bash
git add src/jfile/ahead.rs
git commit -m "refactor: split JFileAhead::get_fromfile into reset/append/scrollback arms"
```

The 18 ahead unit tests pin Reset/Append/Scrollback behavior including the negative-position EOF gate (§21.17).

---

### Task 3: `JMatchTable::add` step split

**Files:**
- Modify: `src/jmatchtable.rs` — `add` (~295-518)

**Interfaces:**
- Produces (private methods on `JMatchTable`):
  - `fn join_colliding(&mut self, dlt: i64, fnd_new: i64) -> Option<usize>` — the `:195-210` block; returns the found colliding node index.
  - `fn join_gliding(&mut self, fnd_org: i64, fnd_new: i64) -> Option<usize>` — the `:212-237` block; returns the found gliding node index.

- [ ] **Step 1: Extract the two join passes**

`add`'s prologue computes `dlt` and `idx_dlt`, then becomes:

```rust
        // Join colliding matches (:195-210)
        let mut cur = self.join_colliding(dlt, fnd_new);

        // Join gliding matches (:212-237)
        if cur.is_none() {
            cur = self.join_gliding(fnd_org, fnd_new);
        }
```

The method bodies are the existing blocks verbatim (the colliding walk returns `Some(ci)` at its `break`, `None` when the walk exhausts; same for gliding — the `idx_gld` local and its "assigned whenever the gliding scan runs" comment move into `join_gliding`). The allocation/evaluation remainder of `add` (the `:239-306` renewal + allocate + fill code and the `:308+` evaluation) stays in `add` for this task unless a boundary is anchor-clean — do not force it.

- [ ] **Step 2: Full gate + commit**

```bash
git add src/jmatchtable.rs
git commit -m "refactor: extract join_colliding/join_gliding from JMatchTable::add"
```

---

### Task 4: `build_full_index` loop unification in `jdiff.rs`

**Files:**
- Modify: `src/jdiff.rs` — `build_full_index` (~949-1039, the two prescan loops)

**Interfaces:**
- Produces: nothing new — the two loops collapse into one with a `feedback` flag.

- [ ] **Step 1: Verify same-decision, then unify**

Verification (do it, note result in commit body): the `verbose > 1` slow loop and the fast loop share identical read/hash/add bodies; the slow loop adds only (a) the `cfg(debug)` `DBGAHH` trace and (b) the every-32MB progress line. Unify:

```rust
        /* Build hashtable (JDiff.cpp:748-778): one loop; the user-feedback
         * extras run only under verbose>1 (the C++ slow version). */
        let feedback = *verbose > 1;
        while lc_val_org > EOF {
            lz_pos_org += 1;
            lc_val_org = org.get(lz_pos_org, ReadType::HardAhead);
            if lc_val_org <= EOF {
                break;
            }
            lk_hsh_org = hash_key(lk_hsh_org, &mut lc_val_prv, lc_val_org, &mut li_eql_org);
            hsh.add(lk_hsh_org, lz_pos_org, li_eql_org);

            /* Debug: hash trace (JDiff.cpp:758-762); the trailing field
             * is `%8d` of the literal 0 here, not a P8zd position. */
            #[cfg(feature = "debug")]
            if feedback && dbg(DBGAHH) {
                dbg_print(format_args!(
                    "ufHshAdd({:2x} -> {:8x}, {}, {:8})\n",
                    lc_val_org as u32,
                    lk_hsh_org,
                    crate::defs::p8(lz_pos_org),
                    0,
                ));
            }

            /* output position every 32MB (JDiff.cpp:764-767) */
            if feedback && (lz_pos_org & PGSMSK) == 0 {
                dbg_print(format_args!(
                    "\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}{:>12}Mb",
                    lz_pos_org / PGSMRK
                ));
            }
        }
```

If verification finds any further difference between the loops, do not unify — record the difference in the ledger and mark this task complete with the loops left separate (spec §6.4 rule).

- [ ] **Step 2: Full gate + commit** (the `-v`/`-vv` golden matrix pins both modes)

```bash
git add src/jdiff.rs
git commit -m "refactor: unify build_full_index slow/fast prescan loops behind a feedback flag"
```

---

### Task 5: `cleanup` debug walks behind `cfg(debug)` helpers

**Files:**
- Modify: `src/jmatchtable.rs` — `cleanup` (~617-722, the two sanity-walk blocks at ~642-661 and ~681-706)

**Interfaces:**
- Produces (private, debug-only):

```rust
    /// Debug sanity walk (JMatchTable.cpp:387-397): counts the new and old
    /// lists and reports when free + old + new != table size.
    #[cfg(feature = "debug")]
    fn dbg_check_table_size(&self, bounded: bool) {
        let (mut li_new, mut li_old) = (0, 0);
        // The redo pass (bounded) walks the new list with the extra bound
        // lpCur != mpLst->ipNxt (short-circuited away when mpNew is null).
        let lst_nxt = self.mp_lst.and_then(|l| self.nodes[l].nxt);
        let mut p = self.mp_new;
        while let Some(ci) = p {
            if bounded && Some(ci) == lst_nxt {
                break;
            }
            li_new += 1;
            p = self.nodes[ci].nxt;
        }
        p = self.mp_old;
        while let Some(ci) = p {
            li_old += 1;
            p = self.nodes[ci].nxt;
        }
        if self.mch_fre + li_new + li_old != self.mch_sze {
            dbg_print(format_args!(
                "Mch Cln Wrong table size {}+{}+{} != {} !\n",
                li_new, li_old, self.mch_fre, self.mch_sze
            ));
        }
    }
```

- [ ] **Step 1: Replace both inline blocks**

Both `#[cfg(feature = "debug")] if dbg(DBGMCH) { … }` walk blocks in `cleanup` become `#[cfg(feature = "debug")] if dbg(DBGMCH) { self.dbg_check_table_size(false); }` (pre-evaluation pass) and `…(true);` (redo pass). Keep the `:387-397` / `:413-423` anchor comments at the call sites. Behavior note for the reviewer: in the unbounded first pass the extra `lst_nxt` check is computed but never short-circuits (`bounded` is false) — same output as today; `mp_lst` is `None` post-merge there anyway.

- [ ] **Step 2: Full gate + commit** (debug-feature build must be part of the gate — it already is via `--all-features`)

```bash
git add src/jmatchtable.rs
git commit -m "refactor: cleanup debug sanity walks behind one cfg(debug) helper"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** §6.1 SearchState → Task 1; §6.2 get_fromfile → Task 2; §6.3 add split → Task 3 (with the anchor-yield rule carried); §6.4 build_full_index → Task 4 (verify-first); the cfg(debug) walk helper from §6.3's last bullet → Task 5.
- **Placeholder scan:** Task 2's "verbatim arm body" elisions reference blocks quoted in full in this plan's source material and present in the file — the executor copies them from the current `match` arms; no invented content. No TBDs.
- **Type consistency:** `SearchState` field names identical to the `JDiff` fields they absorb; `BufDone` return types match the arms' existing returns; `join_colliding`/`join_gliding` return types match the `cur` local's `Option<usize>`.
- **Ordering note:** Tasks are independent except Task 1 (do first — it rewrites `self.<field>` paths that Tasks 4 touches in `build_full_index`, which takes `org: &mut dyn JFile` parameters and mostly-local state, so interference is small but real).
