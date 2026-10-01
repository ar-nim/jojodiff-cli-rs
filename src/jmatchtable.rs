//! Matching table: selection of the best of the possibly matching regions
//! found between the two files, ported 1:1 from C++ `src/JMatchTable.cpp` +
//! `headers/JMatchTable.h` (spec §8).
//!
//! Because of the statistical nature of the hash table, the best solution is
//! not necessarily found first. The table therefore memorizes a number of
//! possibly matching positions (`add`), optimizes them by looking
//! `max(reliability, 1024)` bytes around each one (`get`) and selects the
//! best verified solution. A fixed pool of [`MCH_MAX`] nodes is kept on an
//! intrusive free list; collision chains are bucketed on
//! `delta = fnd_org - fnd_new` with the C trunc-mod then negate rule, so
//! `-1` and `+1` share bucket 1.
//!
//! The C++ constructor state (hashtable, both files, cmp-all flag) moved into
//! [`JMatchTable::get`] by design (controller ruling, ledger T5 ↔ spec §6):
//! [`JMatchTable::new`] takes no arguments.
//!
//! Debug prints (`DBGMCH`/`DBGCMP` sites) are Task 11 and intentionally
//! absent here.
//!
//! # Example
//!
//! ```
//! use jojodiff_cli_rs::jfile::JFileMem;
//! use jojodiff_cli_rs::jhashpos::JHashPos;
//! use jojodiff_cli_rs::jmatchtable::JMatchTable;
//!
//! // Two all-zero files; a hash hit claims a match org 100 / new 900.
//! let mut org = JFileMem::new(vec![0u8; 200]);
//! let mut new = JFileMem::new(vec![0u8; 1000]);
//! let mut tbl = JMatchTable::new();
//! assert_eq!(tbl.add(100, 900, 0, 0), 1); // new entry, space left
//!
//! // get verifies the match by comparing the files and returns the
//! // optimized (rewound to the 24-equal-run anchor) positions.
//! let hsh = JHashPos::new(251);
//! assert_eq!(
//!     tbl.get(900, 900, &hsh, &mut org, &mut new, true),
//!     Some((100, 900))
//! );
//! ```

use std::sync::atomic::{AtomicI32, Ordering};

use crate::defs::{EOF, MCH_MAX, MCH_PME, ReadType, SMPSZE};
use crate::jfile::JFile;
use crate::jhashpos::JHashPos;

/// Number of repaired hash hits (`siHshRpr`, `JMatchTable.cpp:58`): every
/// compare-refuted match decrements its node count and increments this
/// counter. The verbose statistics (Task 8/11) report it.
pub static HSH_RPR: AtomicI32 = AtomicI32::new(0);

/// One match-table element (`rMch`, `JMatchTable.h:108-119`).
struct Node {
    /// Next element in collision list / free list (`ipNxt`).
    next: Option<usize>,
    /// Number of colliding matches (`iiCnt`).
    cnt: i32,
    /// Type of match: 0=unknown, 1=colliding, -1=gliding (`iiTyp`).
    typ: i32,
    /// First found match, new file position (`izBeg`).
    beg: i64,
    /// Last found match, new file position (`izNew`).
    r#new: i64,
    /// Last found match, org file position (`izOrg`).
    org: i64,
    /// Delta key: `izOrg = izNew + izDlt` (`izDlt`).
    delta: i64,
}

/// JojoDiff matching table (`JMatchTable.h:36`): builds and maintains a table
/// of matching regions between two files and selects the "best" match.
pub struct JMatchTable {
    /// Fixed pool of match nodes (`msMch`).
    nodes: Vec<Node>,
    /// Hashtable on delta with match chains (`mpMch`).
    buckets: [Option<usize>; MCH_PME as usize],
    /// Free list of matches (`mpMchFre`).
    free: Option<usize>,
    /// Last gliding match (`mpMchGld`).
    gld: Option<usize>,
    /// Last gliding match's next delta (`mzGldDlt`).
    gld_delta: i64,
}

impl Default for JMatchTable {
    fn default() -> Self {
        Self::new()
    }
}

impl JMatchTable {
    /// Construct a matching table (`JMatchTable.cpp:61-85`): the node pool is
    /// chained into a free list (node i → i+1, last → none) headed at node 0,
    /// all 127 buckets start empty and there is no gliding match yet. The
    /// C++ mallocs the pool and leaves node fields uninitialized; they are
    /// zero-initialized here — `add` always writes them before any read.
    pub fn new() -> Self {
        // Initialize linked list of free nodes (JMatchTable.cpp:73-78).
        let mut nodes = Vec::with_capacity(MCH_MAX as usize);
        for idx in 0..MCH_MAX as usize {
            nodes.push(Node {
                next: if idx + 1 < MCH_MAX as usize {
                    Some(idx + 1)
                } else {
                    None
                },
                // malloc'ed garbage in the C++; `add` fills the fields of a
                // node before anyone reads them (JMatchTable.cpp:150-156).
                cnt: 0,
                typ: 0,
                beg: 0,
                r#new: 0,
                org: 0,
                delta: 0,
            });
        }
        JMatchTable {
            nodes,
            buckets: [None; MCH_PME as usize], // memset, JMatchTable.cpp:80
            free: Some(0),                     // mpMchFre = msMch
            gld: None,                         // mpMchGld = null
            gld_delta: 0,                      // mzGldDlt = 0; unused until first add
        }
    }

    /// Add given match to the table of matches (`JMatchTable.cpp:103-180`):
    /// add to the gliding match if the delta continues it, else to a colliding
    /// match with equal delta, else create a new node from the free list.
    ///
    /// Returns `2` if an existing entry has been enlarged, `1` if a new entry
    /// has been added with space left and `0` if a new entry has been added
    /// with the table now full (or not added at all).
    ///
    /// `base_new` and `eql_new` are unused exactly as in the C++ release
    /// build (they only feed a `DBGMCH` debug print, Task 11).
    pub fn add(&mut self, fnd_org: i64, fnd_new: i64, _base_new: i64, _eql_new: i32) -> i32 {
        let delta = fnd_org - fnd_new; // lzDlt

        // Add to gliding match (JMatchTable.cpp:116-126).
        if let Some(gld) = self.gld {
            if delta == self.gld_delta {
                let g = &mut self.nodes[gld];
                g.typ = -1;
                g.cnt += 1;
                g.r#new = fnd_new;
                self.gld_delta -= 1;
                return 2;
            }
            self.gld = None;
        }

        // Add or override colliding match (JMatchTable.cpp:128-142):
        // liIdx = lzDlt % MCH_PME; if (liIdx < 0) liIdx = -liIdx — the C
        // trunc-mod then negate, so -1 and +1 share bucket 1. Deliberately
        // not `rem_euclid`, which would map -1 to bucket 126 instead.
        let mut idx = delta % MCH_PME;
        if idx < 0 {
            idx = -idx;
        }
        let idx = idx as usize;

        let mut cur = self.buckets[idx];
        while let Some(ci) = cur {
            if self.nodes[ci].delta == delta {
                // Add to colliding match.
                let n = &mut self.nodes[ci];
                n.cnt += 1;
                n.typ = 1;
                n.r#new = fnd_new;
                n.org = fnd_org;
                return 2;
            }
            cur = self.nodes[ci].next;
        }

        // Create new match (JMatchTable.cpp:144-179).
        if let Some(ni) = self.free {
            // Remove from free-list.
            self.free = self.nodes[ni].next;

            // Fill out the form.
            let n = &mut self.nodes[ni];
            n.org = fnd_org;
            n.r#new = fnd_new;
            n.beg = fnd_new;
            n.delta = delta;
            n.cnt = 1;
            n.typ = 0;

            // Add to hashtable (prepend to the bucket chain).
            n.next = self.buckets[idx];
            self.buckets[idx] = Some(ni);

            // Potential gliding match.
            self.gld = Some(ni);
            self.gld_delta = delta - 1;

            if self.free.is_some() { 1 } else { 0 } // still place?
        } else {
            0 // not added
        }
    }

    /// Get the nearest optimized and valid match from the table
    /// (`JMatchTable.cpp:186-332`). Returns the verified best position on
    /// `(org, new)` files, or `None` when no candidate survives verification.
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        _red_org: i64, // azRedOrg: unused, as in the C++
        red_new: i64,
        hsh: &JHashPos,
        org: &mut dyn JFile,
        new: &mut dyn JFile,
        cmp_all: bool,
    ) -> Option<(i64, i64)> {
        const FZY: i64 = 0; // Fuzzy factor (JMatchTable.cpp:185)

        // Current reliability range, at least 1024 (JMatchTable.cpp:206-207).
        let rlb_raw = hsh.reliability();
        let mut rlb = rlb_raw;
        if rlb < 1024 {
            rlb = 1024;
        }

        let mut bst: Option<usize> = None; // lpBst
        let mut bst_org: i64 = 0; // azBstOrg / azBstNew are out-parameters in
        let mut bst_new: i64 = 0; // the C++; the caller seeds 0 (JDiff.cpp:292)
        let mut bst_cnt: i32 = 0; // liBstCnt
        let mut bst_cmp: i32 = 0; // liBstCmp: uninitialized in C++, only read
        // once a best exists

        // Loop on the table (JMatchTable.cpp:210-323).
        for &head in self.buckets.iter() {
            let mut cur = head;
            while let Some(ci) = cur {
                let (typ, cnt, beg, nnew, node_org, delta) = {
                    let n = &self.nodes[ci];
                    (n.typ, n.cnt, n.beg, n.r#new, n.org, n.delta)
                };
                let cur_cnt = if typ < 0 { 0 } else { cnt }; // liCurCnt

                // Skip empty or old entries (JMatchTable.cpp:214-218): the
                // "old" test uses the raw reliability, not the 1024 floor.
                if cnt == 0 || nnew + i64::from(rlb_raw) < red_new {
                    // do nothing: skip empty or old entries
                }
                // Else if potentially better (JMatchTable.cpp:219-223).
                else if bst.is_none()
                    || (beg - i64::from(rlb) < bst_new + FZY // probably nearer
                        && (red_new < bst_new + FZY // still possible to improve?
                            || cur_cnt > bst_cnt))
                // or probably longer
                {
                    // Calculate the test position (JMatchTable.cpp:225-234).
                    let mut tst_new = beg - i64::from(rlb);
                    let dst: i32 = if tst_new >= red_new {
                        rlb
                    } else {
                        tst_new = red_new;
                        // C++ narrows this off_t difference to int here
                        // ("TODO liDst may overflow ??" is kept verbatim).
                        let d = (beg - tst_new) as i32;
                        if d < rlb { rlb } else { d }
                    };

                    // Calculate the test position on the original file by
                    // applying delta (JMatchTable.cpp:236-257).
                    let mut tst_org;
                    if typ < 0 {
                        // We're on a gliding match.
                        if tst_new >= beg {
                            // Within gliding match.
                            tst_org = node_org;
                        } else {
                            // Before gliding match.
                            tst_org = tst_new + delta;
                            if tst_org < 0 {
                                tst_new -= tst_org;
                                tst_org = 0;
                            }
                        }
                    } else {
                        // Colliding match.
                        tst_org = tst_new + delta;
                        if tst_org < 0 {
                            tst_new -= tst_org;
                            tst_org = 0;
                        }
                    }

                    // Compare (JMatchTable.cpp:260): cmp_all reads hard (1),
                    // otherwise soft (2).
                    let mut cmp = check(org, new, &mut tst_org, &mut tst_new, dst, !cmp_all);

                    // Soft eof reached, then rely on hash function
                    // (JMatchTable.cpp:262-276).
                    if cmp == 1 {
                        if cnt < 2 {
                            cmp = 7; // most probably unequal
                        } else {
                            // Estimate a realistic "find" position.
                            if beg >= red_new {
                                tst_new = beg;
                            } else if nnew >= red_new {
                                tst_new = red_new;
                            } else {
                                cmp = 7;
                            }
                            tst_org = tst_new + delta;
                        }
                    }

                    // Remove false matches (JMatchTable.cpp:278-282).
                    if cmp >= 2 {
                        self.nodes[ci].cnt -= 1;
                        HSH_RPR.fetch_add(1, Ordering::Relaxed); // siHshRpr++
                    }

                    // Evaluate: keep the best solution (JMatchTable.cpp:284-299).
                    if cmp <= 1 {
                        let better = bst.is_none() // first found
                            || tst_new + FZY < bst_new // substantially nearer
                            || (tst_new <= bst_new + FZY // potentially longer
                                && cur_cnt > bst_cnt
                                && cmp <= bst_cmp);
                        if better {
                            // New solution seems to be better.
                            bst_org = tst_org;
                            bst_new = tst_new;
                            bst = Some(ci);
                            bst_cnt = cur_cnt;
                            bst_cmp = cmp;
                        }
                    }
                }
                // (The C++ else arm only prints a DBGMCH line — Task 11.)
                cur = self.nodes[ci].next;
            }
        }

        // Mch Err (DBGMCH, Task 11); return (lpBst != null).
        if bst.is_some() {
            Some((bst_org, bst_new))
        } else {
            None
        }
    }

    /// Cleanup & check if there is free space in the table of matches
    /// (`JMatchTable.cpp:337-371`): removes empty (`cnt == 0`) and old
    /// (`new < base_new`) nodes from every chain onto the free list; returns
    /// whether the free list is non-empty afterwards.
    pub fn cleanup(&mut self, base_new: i64) -> bool {
        // Loop on the table (JMatchTable.cpp:341-368).
        for head in self.buckets.iter_mut() {
            let mut prv: Option<usize> = None; // lpPrv
            let mut cur = *head; // lpCur
            while let Some(ci) = cur {
                // If bad or old.
                if self.nodes[ci].cnt == 0 || self.nodes[ci].r#new < base_new {
                    // Remove from list.
                    let nxt = self.nodes[ci].next;
                    match prv {
                        None => *head = nxt,
                        Some(p) => self.nodes[p].next = nxt,
                    }

                    // Add to free-list.
                    self.nodes[ci].next = self.free;
                    self.free = Some(ci);

                    // Next.
                    cur = match prv {
                        None => *head,
                        Some(p) => self.nodes[p].next,
                    };
                } else {
                    prv = Some(ci);
                    cur = self.nodes[ci].next;
                }
            }
        }

        self.free.is_some()
    }
}

/// Verify and optimize matches (`JMatchTable::check`, `JMatchTable.cpp:389-460`).
///
/// Searches at the given positions for a run of `SMPSZE - 8` (24) equal
/// bytes, continuing for `len` bytes unless soft reading (`soft = true`,
/// read-type 2) stops at the end-of-buffer. On success both positions are
/// rewound to the start of the equal run (the optimization anchor).
///
/// Returns `0` = run found, `1` = end-of-buffer reached, `2` = no run of
/// equal bytes found.
fn check(
    org: &mut dyn JFile,
    new: &mut dyn JFile,
    pos_org: &mut i64,
    pos_new: &mut i64,
    mut len: i32,
    soft: bool,
) -> i32 {
    let mut lc_org = EOF; // lcOrg
    let mut lc_new = EOF; // lcNew
    let mut eql = 0; // liEql
    let mut ret = 0; // liRet

    // Read type (aiSft): cmp_all passes 1=hard, else 2=soft
    // (JMatchTable.cpp:260).
    let sft = if soft {
        ReadType::SoftAhead
    } else {
        ReadType::HardAhead
    };

    // Compare bytes: mismatches do not fail here (JMatchTable.cpp:405-415).
    while len > SMPSZE - 8 && ret == 0 && eql < SMPSZE - 8 {
        lc_org = org.get(*pos_org, sft);
        *pos_org += 1;
        lc_new = new.get(*pos_new, sft);
        *pos_new += 1;
        len -= 1;

        if lc_org == lc_new {
            eql += 1;
        } else if lc_org < 0 || lc_new < 0 {
            ret = 1;
        } else {
            eql = 0;
        }
    }

    // Compare last 24 bytes: a mismatch fails (JMatchTable.cpp:417-428).
    while len > 0 && ret == 0 && eql < SMPSZE - 8 {
        lc_org = org.get(*pos_org, sft);
        *pos_org += 1;
        lc_new = new.get(*pos_new, sft);
        *pos_new += 1;
        len -= 1;

        if lc_org == lc_new {
            eql += 1;
        } else if lc_org < 0 || lc_new < 0 {
            ret = 1;
        } else {
            ret = 2;
        }
    }

    match ret {
        0 => {
            // Equality found: rewind both positions to the start of the
            // equal run (JMatchTable.cpp:439-443).
            *pos_org -= i64::from(eql);
            *pos_new -= i64::from(eql);
        }
        1 => {
            if lc_org == EOF || lc_new == EOF {
                // Surely different (hard eof reached) (JMatchTable.cpp:445-447).
                ret = 2;
            } else {
                // May be different (soft eof reached): skip the rest of the
                // window (JMatchTable.cpp:448-452).
                *pos_org += i64::from(len);
                *pos_new += i64::from(len);
            }
        }
        _ => {} // 2: surely different (JMatchTable.cpp:455-457)
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jfile::{JFileAhead, JFileMem};
    use std::io::Cursor;

    /// Deterministic pseudo-random filler byte stream (LCG, bits 8..=15).
    fn lcg_bytes(seed: u32, n: usize) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as u8
            })
            .collect()
    }

    /// Brief step-1 test: return values of `add`, the C trunc-mod bucket math
    /// (−1 and 128 share bucket 1 with 60 in bucket 60), node filling and
    /// free-list exhaustion (256 nodes → last successful add returns 0, the
    /// next add returns 0 without adding).
    #[test]
    fn add_returns_and_bucket_math() {
        let mut m = JMatchTable::new();

        // delta 60 → bucket 60; node filled; gliding primed.
        assert_eq!(m.add(100, 40, 0, 0), 1); // space left
        let n0 = m.buckets[60].unwrap();
        assert_eq!(m.nodes[n0].delta, 60);
        assert_eq!(m.nodes[n0].cnt, 1);
        assert_eq!(m.nodes[n0].typ, 0);
        assert_eq!(m.nodes[n0].beg, 40);
        assert_eq!(m.nodes[n0].r#new, 40);
        assert_eq!(m.nodes[n0].org, 100);
        assert_eq!(m.gld, Some(n0));
        assert_eq!(m.gld_delta, 59); // delta - 1

        // delta -1 → bucket |-1| = 1 (C trunc-mod, then negate)
        assert_eq!(m.add(39, 40, 0, 0), 1);
        let n1 = m.buckets[1].unwrap();
        assert_eq!(m.nodes[n1].delta, -1);
        assert_eq!(m.nodes[n1].beg, 40);

        // delta 128 → 128 % 127 = 1 → same bucket as -1: chain of 2, new
        // node prepended.
        assert_eq!(m.add(168, 40, 0, 0), 1);
        let n2 = m.buckets[1].unwrap();
        assert_eq!(m.nodes[n2].delta, 128);
        assert_eq!(m.nodes[n2].next, Some(n1));

        // Fill the remaining 253 free nodes with distinct deltas; the final
        // add pops the last free node and reports the table as full (0).
        let mut last = 1;
        for d in 200..=452 {
            last = m.add(d, 0, 0, 0);
        }
        assert_eq!(last, 0); // added, but table is now full
        assert!(m.free.is_none());

        // No free node: not added.
        assert_eq!(m.add(999, 0, 0, 0), 0);
        let mut total = 0;
        for &head in m.buckets.iter() {
            let mut cur = head;
            while let Some(ci) = cur {
                total += 1;
                cur = m.nodes[ci].next;
            }
        }
        assert_eq!(total, 256); // the rejected add did not grow the table
    }

    /// Brief step-1 test: a delta equal to `gld_delta` continues the gliding
    /// match (typ −1, cnt++, new updated, gld_delta decremented); a different
    /// delta or a colliding add ends the glide.
    #[test]
    fn gliding_match_decrements() {
        let mut m = JMatchTable::new();
        assert_eq!(m.add(100, 50, 0, 0), 1); // delta 50, gld_delta 49
        let n0 = m.buckets[50].unwrap();

        // delta 49 == gld_delta 49 → gliding continuation.
        assert_eq!(m.add(101, 52, 0, 0), 2);
        assert_eq!(m.nodes[n0].typ, -1);
        assert_eq!(m.nodes[n0].cnt, 2);
        assert_eq!(m.nodes[n0].r#new, 52);
        assert_eq!(m.nodes[n0].org, 100); // gliding does not touch org
        assert_eq!(m.nodes[n0].beg, 50); // ... nor beg
        assert_eq!(m.gld_delta, 48); // decremented

        // delta 48 == gld_delta 48 → glides again.
        assert_eq!(m.add(102, 54, 0, 0), 2);
        assert_eq!(m.nodes[n0].cnt, 3);
        assert_eq!(m.nodes[n0].r#new, 54);
        assert_eq!(m.gld_delta, 47);

        // delta 48 ≠ 47: the glide ends; 48 is not in bucket 48 → new node.
        assert_eq!(m.add(103, 55, 0, 0), 1);
        let n1 = m.buckets[48].unwrap();
        assert_eq!(m.nodes[n1].delta, 48);
        assert_eq!(m.gld, Some(n1));
        assert_eq!(m.gld_delta, 47);

        // A colliding add clears the gliding pointer.
        assert_eq!(m.add(300, 250, 0, 0), 2); // delta 50 → node0 enlarged
        assert!(m.gld.is_none());
        assert_eq!(m.nodes[n0].typ, 1);
        assert_eq!(m.nodes[n0].cnt, 4);
        assert_eq!(m.nodes[n1].typ, 0); // n1 untouched by the collision
    }

    /// Brief step-1 test: adding a second, non-adjacent match with the same
    /// delta enlarges the colliding node (cnt++, typ=1, last org/new updated,
    /// beg unchanged) and keeps a single chain node.
    #[test]
    fn colliding_match_enlarges() {
        let mut m = JMatchTable::new();
        assert_eq!(m.add(100, 40, 0, 0), 1); // delta 60
        let n0 = m.buckets[60].unwrap();

        assert_eq!(m.add(200, 140, 0, 0), 2); // same delta 60 → colliding
        assert_eq!(m.nodes[n0].cnt, 2);
        assert_eq!(m.nodes[n0].typ, 1);
        assert_eq!(m.nodes[n0].org, 200); // last found org
        assert_eq!(m.nodes[n0].r#new, 140); // last found new
        assert_eq!(m.nodes[n0].beg, 40); // beg unchanged
        assert_eq!(m.nodes[n0].delta, 60);

        assert_eq!(m.add(300, 240, 0, 0), 2);
        assert_eq!(m.nodes[n0].cnt, 3);

        // Still a single node in the bucket.
        assert_eq!(m.nodes[n0].next, None);
        assert_eq!(m.buckets[60], Some(n0));
    }

    /// Brief step-1 test: cleanup removes empty (cnt == 0) and old
    /// (new < base_new) nodes onto the free list, relinking the chains; it
    /// reports whether the free list is non-empty afterwards (false when the
    /// table is full and nothing was freed).
    #[test]
    fn cleanup_frees_old_and_empty() {
        let mut m = JMatchTable::new();
        // Fill the table: 256 distinct deltas, fnd_new = 0 for all.
        let mut last = 1;
        for d in 0..256 {
            last = m.add(d, 0, 0, 0);
        }
        assert_eq!(last, 0); // table full
        assert!(m.free.is_none());

        // Nothing to clean: new (0) < base_new (0) is false and all cnt are 1.
        assert!(!m.cleanup(0));
        assert!(m.free.is_none());

        // An empty node (cnt == 0) is removed even when not old. Bucket 10
        // holds deltas 10 and 137; empty the delta-137 node.
        let mut n137 = m.buckets[10].unwrap();
        while m.nodes[n137].delta != 137 {
            n137 = m.nodes[n137].next.unwrap();
        }
        m.nodes[n137].cnt = 0;
        assert!(m.cleanup(0));
        // The chain lost exactly the delta-137 node.
        let mut seen = Vec::new();
        let mut cur = m.buckets[10];
        while let Some(ci) = cur {
            seen.push(m.nodes[ci].delta);
            cur = m.nodes[ci].next;
        }
        assert_eq!(seen, vec![10]);

        // Old nodes (new = 0 < 1) all go: every bucket empties, the free
        // list holds all 256 nodes again, and a subsequent add works.
        assert!(m.cleanup(1));
        assert!(m.buckets.iter().all(|&h| h.is_none()));
        let mut free_cnt = 0;
        let mut cur = m.free;
        while let Some(ci) = cur {
            free_cnt += 1;
            cur = m.nodes[ci].next;
        }
        assert_eq!(free_cnt, 256);
        assert_eq!(m.add(5000, 0, 0, 0), 1);
    }

    /// Brief step-1 test: check finds the run of 24 equal bytes and rewinds
    /// both positions to the run anchor (pos − eql), exactly as the C++.
    #[test]
    fn check_finds_run_and_rewinds() {
        // 40 equal bytes (0x11) at offset 10; the first 10 bytes differ.
        let mut org_d = vec![0x55u8; 64];
        let mut new_d = vec![0xAAu8; 64];
        for b in &mut org_d[10..50] {
            *b = 0x11;
        }
        for b in &mut new_d[10..50] {
            *b = 0x11;
        }
        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);

        let (mut po, mut pn) = (10i64, 10i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, false), 0);
        assert_eq!((po, pn), (10, 10)); // read up to 34, rewound by eql 24

        // A leading mismatch does not fail (phase 1); the anchor is the start
        // of the 24-equal-run.
        let mut org_d = vec![0x55u8; 64];
        let mut new_d = vec![0x55u8; 64];
        org_d[0] = 0x01;
        new_d[0] = 0x02;
        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);

        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, false), 0);
        assert_eq!((po, pn), (1, 1)); // rewound from 25 by eql 24
    }

    /// Brief step-1 test: a mismatch inside the last 24 bytes (phase 2)
    /// fails with 2; the positions are left just past the failing byte.
    #[test]
    fn check_phase2_mismatch_fails() {
        // Mismatch at the first phase-2 byte (position 24): byte 0 differs,
        // bytes 1..24 are equal (eql 23 at the phase boundary).
        let mut org_d = vec![0x22u8; 48];
        let mut new_d = vec![0x22u8; 48];
        org_d[0] = 0x01;
        new_d[0] = 0x02;
        for b in &mut org_d[24..] {
            *b = 0x33;
        }
        for b in &mut new_d[24..] {
            *b = 0x44;
        }
        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);
        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, false), 2);
        assert_eq!((po, pn), (25, 25));

        // Mismatch deeper inside the last 24 bytes (position 30): bytes
        // 0..7 differ, 7..30 are equal (eql 23 again at pos 30).
        let mut org_d = vec![0x01u8; 48];
        let mut new_d = vec![0x02u8; 48];
        for b in &mut org_d[7..30] {
            *b = 0x22;
        }
        for b in &mut new_d[7..30] {
            *b = 0x22;
        }
        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);
        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, false), 2);
        assert_eq!((po, pn), (31, 31));
    }

    /// Brief step-1 test: a soft-ahead read beyond the look-ahead buffer
    /// returns EOB (not EOF) → check returns 1 and advances both positions by
    /// the remaining length; a hard read at the real end of file returns EOF,
    /// which upgrades the result to 2.
    #[test]
    fn check_soft_eob_returns_1() {
        // Soft: new-file position 1000 is far beyond the fresh 16-byte block
        // window, so the very first read EOBs.
        let mut org = JFileMem::new(vec![0x55u8; 128]);
        let mut new = JFileAhead::new(Cursor::new(vec![0xAAu8; 4096]), "Tst", 1024, 16);
        let (mut po, mut pn) = (0i64, 1000i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, true), 1);
        assert_eq!((po, pn), (48, 1048)); // 1 read + 47 remaining

        // Hard: the new file ends at 10, so reading position 10 hits EOF.
        let mut org = JFileMem::new(vec![0x55u8; 128]);
        let mut new = JFileAhead::new(Cursor::new(vec![0x55u8; 10]), "Tst", 1024, 16);
        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(check(&mut org, &mut new, &mut po, &mut pn, 48, false), 2);
        assert_eq!((po, pn), (11, 11));
    }

    /// Brief step-1 test: `get` verifies candidates by comparing the files
    /// and selects the verified match pointing at the block start; a
    /// refuted false hit is repaired (cnt−−, HSH_RPR++) and a farther verified
    /// hit loses to the accepted best.
    #[test]
    fn get_selects_nearest_verified() {
        // org: filler + 64-byte block B at 1000 and 3000.
        // new: filler + block B at 2000 and 2500.
        let block: Vec<u8> = (0..64).map(|i| 0xC0u8 + i as u8).collect();
        let mut org_d = lcg_bytes(12345, 8192);
        org_d[1000..1064].copy_from_slice(&block);
        org_d[3000..3064].copy_from_slice(&block);
        let mut new_d = lcg_bytes(98765, 4096);
        new_d[2000..2064].copy_from_slice(&block);
        new_d[2500..2564].copy_from_slice(&block);

        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);
        let hsh = JHashPos::new(251); // reliability 48 → rlb = max(48, 1024)

        let mut m = JMatchTable::new();
        // C: false hit, delta 2900 → bucket 106 (evaluated first).
        assert_eq!(m.add(4900, 2000, 0, 0), 1);
        // A: true hit at the block start, delta -1000 → bucket 111.
        assert_eq!(m.add(1000, 2000, 0, 0), 1);
        // B: second true hit, delta 500 → bucket 119 (farther).
        assert_eq!(m.add(3000, 2500, 0, 0), 1);

        let before = HSH_RPR.load(Ordering::Relaxed);
        assert_eq!(
            m.get(1000, 2000, &hsh, &mut org, &mut new, true),
            Some((1000, 2000))
        );
        assert_eq!(HSH_RPR.load(Ordering::Relaxed) - before, 1); // C repaired

        // The refuted candidate lost its count; the accepted one kept it.
        let nc = m.buckets[106].unwrap();
        assert_eq!(m.nodes[nc].cnt, 0);
        let na = m.buckets[111].unwrap();
        assert_eq!(m.nodes[na].cnt, 1);
    }

    /// Extra test (soft-EOF recovery of `get`, spec §8.2): when check stops
    /// at the end-of-buffer (cmp == 1), a node with cnt < 2 is judged "most
    /// probably unequal" (cmp 7 → repair), while a node with cnt ≥ 2 has its
    /// find position estimated from beg/new and can still win.
    #[test]
    fn get_soft_eof_recovers_positions() {
        // org as a fresh look-ahead file: every soft read beyond the 16-byte
        // block window EOBs, so all three candidates stop with cmp == 1.
        let mut org = JFileAhead::new(Cursor::new(vec![0u8; 8192]), "Tst", 1024, 16);
        let mut new = JFileMem::new(vec![0u8; 4096]);
        let hsh = JHashPos::new(251);

        let mut m = JMatchTable::new();
        // C: delta 2000 → bucket 95; twice → cnt 2, typ 1, beg 3000.
        assert_eq!(m.add(5000, 3000, 0, 0), 1);
        assert_eq!(m.add(5100, 3100, 0, 0), 2);
        // B: delta 1500 → bucket 103; cnt 1.
        assert_eq!(m.add(3000, 1500, 0, 0), 1);
        // A: delta 1000 → bucket 111; twice → cnt 2, typ 1, beg 1000.
        assert_eq!(m.add(2000, 1000, 0, 0), 1);
        assert_eq!(m.add(2600, 1600, 0, 0), 2);

        // red_new = 1100; rlb = 1024.
        assert_eq!(
            m.get(1000, 1100, &hsh, &mut org, &mut new, false),
            Some((2100, 1100))
        );

        // B was repaired (cnt 1 < 2 → cmp 7 → cnt−−); C (beg ≥ red_new →
        // tst_new = beg) and A (new ≥ red_new → tst_new = red_new) were not.
        let nb = m.buckets[103].unwrap();
        assert_eq!(m.nodes[nb].cnt, 0);
        let nc = m.buckets[95].unwrap();
        assert_eq!(m.nodes[nc].cnt, 2);
        let na = m.buckets[111].unwrap();
        assert_eq!(m.nodes[na].cnt, 2);
    }

    /// Extra test (gliding branch of `get`, spec §8.2): a gliding node
    /// (typ −1) counts as 0 (`cnt_now = 0`) and, when the test position is
    /// within the glide (`tst_new >= beg`), is verified at the last found org
    /// position.
    #[test]
    fn get_gliding_uses_last_org() {
        // org: filler + 64-byte block at 2000; new: filler + block at 1100.
        let block: Vec<u8> = (0..64).map(|i| 0xC0u8 + i as u8).collect();
        let mut org_d = lcg_bytes(24680, 4096);
        org_d[2000..2064].copy_from_slice(&block);
        let mut new_d = lcg_bytes(13579, 4096);
        new_d[1100..1164].copy_from_slice(&block);

        let mut m = JMatchTable::new();
        assert_eq!(m.add(2000, 1000, 0, 0), 1); // delta 1000 → bucket 111
        assert_eq!(m.add(2010, 1011, 0, 0), 2); // delta 999 == gld 999
        assert_eq!(m.add(2100, 1102, 0, 0), 2); // delta 998 == gld 998
        let na = m.buckets[111].unwrap();
        assert_eq!(m.nodes[na].typ, -1);
        assert_eq!(m.nodes[na].cnt, 3);
        assert_eq!(m.nodes[na].org, 2000); // gliding never updates org
        assert_eq!(m.nodes[na].r#new, 1102);

        let mut org = JFileMem::new(org_d);
        let mut new = JFileMem::new(new_d);
        let hsh = JHashPos::new(251);
        // tst_new = 1100 ≥ beg 1000 → within the glide: tst_org = org (2000);
        // the files match there for 64 bytes → verified and rewound.
        assert_eq!(
            m.get(1000, 1100, &hsh, &mut org, &mut new, true),
            Some((2000, 1100))
        );
        assert_eq!(m.nodes[na].cnt, 3); // verified, not repaired
    }
}
