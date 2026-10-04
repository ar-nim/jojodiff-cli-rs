//! Matching table: selection of the best of the possibly matching regions
//! found between the two files, ported 1:1 from the 0.8.5 C++ `src/JMatchTable.cpp`
//! + `src/JMatchTable.h` (spec §18.E).
//!
//! The 0.8.5 table is **dynamic**: its size comes from the `-x` value
//! (`miMchSze = max(13, aiMchSze)`; the bucket prime derives from the
//! UNCLAMPED value, `getLowerPrime(aiMchSze * 2)`), elements are never freed
//! but live on the `mpNew`/`mpOld` aging lists and are reused when
//! `JMatchTable::is_old2_reuse` says so ("full" = no reusable element), and
//! the best match is tracked **incrementally** by `JMatchTable::is_best`
//! during `add`/`cleanup` instead of being rescanned on demand.
//!
//! Two bucket tables detect candidate joins: `mpCol` on `|delta| % pme`
//! (colliding matches, `JMatchTable.cpp:197` — the C `abs` macro shares one
//! bucket between a delta and its negation) and `mpGld` on `org % pme`
//! (gliding matches, `:215` — a hash-based detection now, where 0.8.1
//! chained a single last-match pointer).
//!
//! # Rust adaptations (mechanical, no behavior change)
//!
//! * The C++ constructor stores `mpHsh`/`mpFilOrg`/`mpFilNew` pointers; the
//!   engine owns them as sibling fields here, so `cleanup` receives the
//!   hashtable's reliability value (`mpHsh->get_reliability()`,
//!   `:377`) and the file readers are passed into `add`/`cleanup` (used by
//!   `check`) — the same pattern the 0.8.1 port established for `get`.
//! * Node pointers (`rMch *`) become `Option<usize>` arena indices; the
//!   arena is zero-initialized where the C++ `malloc`s garbage — `add` fills
//!   every field of a node before anyone reads it, and the "stale `ipNxt`"
//!   walks (`addNew` deliberately leaves the last node's next dangling) read
//!   values the C++ also wrote earlier, so behavior is identical.
//! * The ctor parameter `miAhdMax` (stored, never read) is dead at 0.8.5 and
//!   ported as dead code (spec §21.13).
//!
//! Debug prints (`DBGMCH`/`DBGCMP` sites) live behind the `debug` feature
//! with their exact C++ format strings (`JMatchTable.cpp:154,278,350,391,
//! 414,502,633,704` DBGMCH; `:826,858` DBGCMP — the compare prologue is
//! "Cmp Gld|Col (...)" now, was "Fnd (...)"). Positions use the debug-build
//! `P8zd` width 10 ([`crate::defs::p8`]).
//!
//! # Example
//!
//! ```
//! use jojodiff_cli_rs::jfile::JFileMem;
//! use jojodiff_cli_rs::jmatchtable::{JMatchTable, MchRet};
//!
//! // Two all-zero files; a hash hit claims a match org 1000 / new 500.
//! let mut org = JFileMem::new(vec![0u8; 4096]);
//! let mut new = JFileMem::new(vec![0u8; 4096]);
//! let mut tbl = JMatchTable::new(64, true, 1024).expect("doc table");
//! assert_eq!(tbl.add(1000, 500, 600, &mut org, &mut new), MchRet::Best);
//!
//! // cleanup verifies and elects the best; getbest returns the tracked
//! // best without rescanning: the run anchor the compare started from.
//! assert_eq!(tbl.cleanup(0, 600, 48, &mut org, &mut new), MchRet::Best);
//! assert_eq!(tbl.getbest(0, 600), Some((1100, 600)));
//! ```

use crate::defs::{ReadType, SMPSZE, get_lower_prime};
#[cfg(feature = "debug")]
use crate::defs::{p8, print_char};
use crate::error::JDiffError;
#[cfg(feature = "debug")]
use crate::jdebug::{DBGCMP, DBGMCH, dbg, dbg_print};
use crate::jfile::{ByteOrEof, JFile};
#[cfg(feature = "debug")]
use std::sync::atomic::{AtomicI64, Ordering};

use std::mem::size_of;

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

// Continuous runs of 8 (> 7) equal bytes are worth the jump
// Extend to 12 to explore, so we can prefer longer runs
// These settings provide a good tradeoff between maximum equal bytes and minimum overhead bytes
// (`JMatchTable.cpp:36-38`)
/// Run length worth a jump (`EQLSZE`, `JMatchTable.cpp:36`).
pub const EQLSZE: i32 = 8;
/// Minimum reported run length (`EQLMIN`, `JMatchTable.cpp:37`); `check`
/// returns a run only when it is strictly longer.
pub const EQLMIN: i32 = 4;
/// Equal-run cap of `check` (`EQLMAX`, `JMatchTable.cpp:38`).
pub const EQLMAX: i32 = 256;

/// Max compare distance (`MAXDST`, `JMatchTable.cpp:40`: `2 * 1024 * 1024`,
/// an int in the C++, used in `off_t` arithmetic) — on HDD +/-40ms at
/// 100Mb/s + 10ms seek time.
pub const MAXDST: i64 = 2 * 1024 * 1024;
/// Min compare distance (`MINDST`, `JMatchTable.cpp:41`) — on SSD +/- 4ms at
/// 1Gb/s + 1ms seek time.
pub const MINDST: i64 = 1024;

/// Fuzzy factor (`FZY`, `JMatchTable.cpp:46`): for differences smaller than
/// this number of bytes, take the longest looking sequence. Reason: control
/// bytes consume byte too, so taking the longer one is better.
pub const FZY: i64 = 0;

// Cmp codes (`JMatchTable.cpp:49-51`)
/// Compare result: invalid match, reusable (`CMPINV`).
const CMPINV: i32 = -1;
/// Compare result: skipped as too old, reactivable by a new hash hit (`CMPSKP`).
const CMPSKP: i32 = -2;
/// Compare result: end-of-buffer reached (`CMPEOB`) — also `check`'s
/// "EOB reached, no equal bytes found" return.
const CMPEOB: i32 = -3;

/// Match-node state (`Node.cmp`, `JMatchTable.h`): run length when
/// non-negative; the C++ sentinels as variants; `is_best`'s negated EOB
/// distance estimates (negative values that are NOT the sentinels) as
/// `Est`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmpVal {
    /// Validated run length (C++ `>= 0`).
    ///
    /// Invariant: the payload is always non-negative — negative legacy
    /// values are `Est`, never `Run`; `from_legacy_i32` guards `v >= 0`,
    /// and every direct construction is `Run(0)`.
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

impl CmpVal {
    /// The C++ `int iiCmp` scalar this state encodes: [`CmpVal::Run`] as
    /// itself, the sentinels as -1/-2/-3, [`CmpVal::Est`] as its raw
    /// (negative) payload. The `-dmch` debug traces print this to keep
    /// their bytes identical, as do the few genuinely scalar comparisons
    /// (`min(0)` in `is_best`, the `.abs()` distance checks in the
    /// `isOld` functions, and the threshold compares).
    fn as_legacy_i32(self) -> i32 {
        match self {
            CmpVal::Run(n) => n,
            CmpVal::Inv => CMPINV,
            CmpVal::Skp => CMPSKP,
            CmpVal::Eob => CMPEOB,
            CmpVal::Est(v) => v,
        }
    }

    /// Inverse of [`CmpVal::as_legacy_i32`]: a raw compare scalar (the
    /// return of `check`: 0, `CMPEOB` or a run length) as node state.
    /// Values below the sentinels — `is_best`'s negated estimates — map
    /// to [`CmpVal::Est`].
    fn from_legacy_i32(v: i32) -> Self {
        match v {
            CMPINV => CmpVal::Inv,
            CMPSKP => CmpVal::Skp,
            CMPEOB => CmpVal::Eob,
            v if v >= 0 => CmpVal::Run(v),
            v => CmpVal::Est(v),
        }
    }
}

/// Match-table return taxonomy (`eMatchReturn`, `JMatchTable.h:53`).
///
/// Declaration order defines the discriminants 0-6 printed by the `%d` of
/// the "Add ... ret=" debug trace (`JMatchTable.cpp:353`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MchRet {
    Error,    // Table in an unexpectedly full state (should not occur)
    Full,     // No free or reusable element: searching must stop
    Enlarged, // Existing solution has been enlarged
    Invalid,  // Match does not point to a valid solution
    Good,     // Good match found (EQLSZE <= compare < EQLMAX)
    Best,     // Perfect match found (compare >= EQLMAX)
    Valid,    // Solution added
}

/// One match-table element (`rMch`, `JMatchTable.h:106-119`). The C++ links
/// nodes with pointers; the port uses arena indices. `ipNxt` is the aging
/// list link whose stale value is deliberately never cleared on removal
/// (`addNew`'s "saves one assignment" trick, `:887`) — the arena never frees,
/// so the stale-chain walks read defined values exactly like the C++.
#[derive(Clone)]
struct Node {
    /// Next element on the pseudo-ordered aging stack (`ipNxt`).
    nxt: Option<usize>,
    /// Next element in collision bucket (`ipCol`).
    col: Option<usize>,
    /// Next element in gliding bucket list (`ipGld`).
    gld: Option<usize>,

    /// Number of colliding matches (= confirming matches) (`iiCnt`).
    cnt: i32,
    /// Gliding match recurrence, 0=no, <0=mixed, >0=glide (`iiGld`).
    gldcnt: i32,
    /// First found match, new file position (`izBeg`).
    beg: i64,
    /// Last found match, new file position (`izNew`).
    r#new: i64,
    /// Last found match, org file position (`izOrg`).
    org: i64,
    /// Delta: `izOrg = izNew + izDlt` (`izDlt`).
    dlt: i64,
    /// Result of last compare position (`izTst`).
    tst: i64,
    /// Result of last compare (`iiCmp`): [`CmpVal`] — the C++ sentinels
    /// [`CMPINV`]/[`CMPSKP`]/[`CMPEOB`], the last verified run length, or
    /// `is_best`'s negated EOB distance estimate.
    cmp: CmpVal,
}

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

/// JojoDiff matching table (`JMatchTable.h:33`): builds and maintains a table
/// of matching regions between two files and selects the "best" match.
pub struct JMatchTable {
    /// Table of matches (`msMch`): `miMchSze` elements, allocated once.
    nodes: Vec<Node>,
    /// Size of the matching table (`miMchSze`, const): `max(13, -x value)`.
    /// Only the #if debug sanity checks read it, like the C++ (plus the
    /// tests).
    #[cfg_attr(not(feature = "debug"), allow(dead_code))]
    mch_sze: i32,
    /// Free index (`miMchFre`): counts down the never-used elements; 0.8.5
    /// has no free list — exhausted, allocation reuses aging elements.
    mch_fre: i32,
    /// Size of the matching hashtables (`miMchPme`):
    /// `get_lower_prime(unclamped -x value * 2)` — the unclamped-value quirk.
    mch_pme: i32,
    /// Hashtable on `|izDlt|` for detecting colliding matches (`mpCol`).
    col_tbl: Vec<Option<usize>>,
    /// Hashtable on `izOrg` for detecting gliding matches (`mpGld`).
    gld_tbl: Vec<Option<usize>>,
    /// List of old elements (`mpOld`).
    mp_old: Option<usize>,
    /// List of new elements (`mpNew`).
    mp_new: Option<usize>,
    /// Last of new elements (`mpLst`).
    mp_lst: Option<usize>,
    /// Current best element (`mpBst`).
    mp_bst: Option<usize>,
    /// Current best source position (`mzBstOrg`).
    z_bst_org: i64,
    /// Current best destin position (`mzBstNew`).
    z_bst_new: i64,
    /// Current best (estimated) length (`miBstCmp`).
    i_bst_cmp: i32,
    /// Limit for being old (`mzOld`).
    z_old: i64,

    /// Compare all matches, even if data not in buffer? (`mbCmpAll`, const).
    cmp_all: bool,
    /// Lookahead & lookback range (`miAhdMax`, const).
    ///
    /// Dead code: the C++ constructor stores it but no 0.8.5 method reads it
    /// — kept for parity (spec §21.13).
    #[allow(dead_code)]
    ahd_max: i32,
    /// Current reliability range from the hashtable (`miRlb`), refreshed by
    /// `cleanup` (mechanically passed in — see the module docs).
    rlb: i32,
    /// Number of repaired hash hits, matches repaired by comparing
    /// (`miHshRpr`; the 0.8.1 global static retired per spec §18.E).
    hsh_rpr: i32,
}

impl JMatchTable {
    /// Construct a matching table (`JMatchTable.cpp:78-105`).
    ///
    /// `mch_sze` is the `-x` value: the table size is `max(13, mch_sze)`
    /// while the bucket prime is `get_lower_prime(mch_sze * 2)` from the
    /// **unclamped** value (`:85,97` — with `-x 5`: size 13, prime
    /// `get_lower_prime(10)` = 7). The C++ initializer `miMchFre(miMchSze)`
    /// reads the already-initialized (clamped) member — member initializers
    /// run in declaration order — so the free count starts at the clamped
    /// size too (verified against the real ctor shape with g++; the written
    /// spec §21.15 attributes the quirk to `miMchFre` instead).
    ///
    /// The C++ `malloc`s the node array (garbage fields, filled by `add`
    /// before any read) and `calloc`s the two bucket tables; the port
    /// zero-initializes all three (spec §21.5 deviation, deterministic).
    /// `mch_sze * 2` is computed in i64 and clamped to `i32::MAX` (the C++
    /// int multiply overflows UB for huge `-x`).
    ///
    /// `cmp_all` selects hard vs soft compare-ahead in `check`; `ahd_max` is
    /// stored but never read (dead, `:86`).
    pub fn new(mch_sze: i32, cmp_all: bool, ahd_max: i32) -> Result<Self, JDiffError> {
        // miMchSze(aiMchSze < 13 ? 13 : aiMchSze), miMchFre(miMchSze) (:85)
        let clamped = if mch_sze < 13 { 13 } else { mch_sze };

        // miMchPme = getLowerPrime(aiMchSze * 2) — the UNCLAMPED value (:97).
        let mch_pme = mch_pme_for(mch_sze);
        // calloc(miMchPme, sizeof(tMch*)) for negative/zero primes fails in
        // the C++ (null, then UB on first use — no throw in the oracle
        // build); the port panics instead of dereferencing null.
        assert!(
            mch_pme > 0,
            "JMatchTable: getLowerPrime({mch_sze} * 2) = {mch_pme} is not positive (C++ calloc failure / UB)"
        );

        Ok(JMatchTable {
            nodes: crate::try_zeroed_vec(
                clamped as usize,
                Node {
                    nxt: None,
                    col: None,
                    gld: None,
                    cnt: 0,
                    gldcnt: 0,
                    beg: 0,
                    r#new: 0,
                    org: 0,
                    dlt: 0,
                    tst: 0,
                    cmp: CmpVal::Run(0),
                },
            )?,
            mch_sze: clamped,
            mch_fre: clamped,
            mch_pme,
            col_tbl: crate::try_zeroed_vec(mch_pme as usize, None)?,
            gld_tbl: crate::try_zeroed_vec(mch_pme as usize, None)?,
            mp_old: None,
            mp_new: None,
            mp_lst: None,
            mp_bst: None,
            z_bst_org: 0,
            z_bst_new: 0,
            i_bst_cmp: 0,
            z_old: 0,
            cmp_all,
            ahd_max,
            rlb: 0,
            hsh_rpr: 0,
        })
    }

    /// Add given match to the array of matches (`JMatchTable::add`,
    /// `JMatchTable.cpp:188-363`):
    ///
    /// - Add to a colliding (equal delta) or gliding (equal org) match if
    ///   possible, or
    /// - Add at the end of the list from the free counter, or
    /// - Reuse (override) an old match if `JMatchTable::is_old2_reuse`
    ///   allows it.
    ///
    /// `red_new` is the current read position. The file readers back
    /// `check`'s compares (mechanically passed — the C++ stores pointers).
    pub fn add(
        &mut self,
        fnd_org: i64, // azFndOrgAdd: match to add
        fnd_new: i64, // azFndNewAdd
        red_new: i64, // azRedNew: current read position
        org: &mut dyn JFile,
        newf: &mut dyn JFile,
    ) -> MchRet {
        // Join colliding matches (:195-210)
        let dlt = fnd_org - fnd_new; // lzDlt: delta key of the match
        let idx_dlt = (dlt.abs() % i64::from(self.mch_pme)) as usize; // abs(lzDlt) % miMchPme (:197)
        let mut cur = self.join_colliding(dlt, fnd_new);

        // Join gliding matches (:212-237)
        if cur.is_none() {
            cur = self.join_gliding(fnd_org, fnd_new);
        }

        // remove first renewed item from the oldlist (:239-244)
        if let Some(ci) = cur {
            if self.mp_old == Some(ci) {
                self.mp_old = self.nodes[ci].nxt; // remove from oldlist
                self.nextold(red_new, org, newf); // puts a reusable element in front of mpOld
                self.add_new(ci); // add to the newlist
            }
        }

        // allocate new element (:246-306)
        if cur.is_none() {
            // get free element
            if self.mch_fre > 0 {
                // take unused element
                self.mch_fre -= 1;
                cur = Some(self.mch_fre as usize); // lpCur = &msMch[miMchFre]
            } else if let Some(oi) = self.mp_old {
                // sanity check (:254-263)
                #[cfg(feature = "debug")]
                {
                    let o = &self.nodes[oi];
                    if !matches!(o.cmp, CmpVal::Inv) // Invalids may be reused ?
                        && !matches!(o.cmp, CmpVal::Eob) // EOB with low iiCnt may be reused ?
                        && match o.cmp {
                            // (o.cmp != 0 && o.izNew >= azRedNew)
                            CmpVal::Run(0) => false,
                            // (o.cmp != 0 && ...) || (o.cmp > 0 && o.izTst + o.cmp > azRedNew)
                            CmpVal::Run(n) => {
                                o.r#new >= red_new
                                    || (n > 0 && o.tst + i64::from(n) > red_new)
                            }
                            // Skp/Est are != 0: only the first disjunct applies
                            _ => o.r#new >= red_new,
                        }
                    {
                        dbg_print(format_args!(
                            "Mch Add ({}>{}<{}) Reusing valid new element {} !\n",
                            p8(o.org),
                            p8(o.dlt),
                            p8(o.r#new),
                            o.cmp.as_legacy_i32()
                        ));
                    }
                }

                // reuse old element (:266-268)
                cur = Some(oi);
                self.mp_old = self.nodes[oi].nxt;
                self.nextold(red_new, org, newf); // prepare next old element

                // remove old element from gliding & colliding lists (:270-274)
                if self.nodes[oi].cnt == 1 || self.nodes[oi].gldcnt == 0 {
                    self.del_col(oi);
                }
                if self.nodes[oi].cnt == 1 || self.nodes[oi].gldcnt != 0 {
                    self.del_gld(oi);
                }

                // debug reporting (:277-284)
                #[cfg(feature = "debug")]
                if dbg(DBGMCH) {
                    let o = &self.nodes[oi];
                    dbg_print(format_args!(
                        "Del         [{:2}:{}>{}<{}~{}#{:4}+{:4}] bse={}\n",
                        o.gldcnt,
                        p8(o.org),
                        p8(o.dlt),
                        p8(o.beg),
                        p8(o.r#new),
                        o.cnt,
                        o.cmp.as_legacy_i32(),
                        red_new
                    ));
                }
            } else {
                return MchRet::Error; // should not occur
            }

            let ci = cur.unwrap();

            // fill out the form (:289-297)
            {
                let n = &mut self.nodes[ci];
                n.org = fnd_org;
                n.r#new = fnd_new;
                n.beg = fnd_new;
                n.dlt = dlt;
                n.cnt = 1;
                n.gldcnt = 0;
                n.cmp = CmpVal::Run(0);
                n.tst = -1;
            }

            // add to colliding hashtable (:299-301)
            self.nodes[ci].col = self.col_tbl[idx_dlt];
            self.col_tbl[idx_dlt] = Some(ci);

            // liIdxGld: the gliding scan in `join_gliding` always ran when
            // this allocation branch is reached (cur was null after both
            // joins), so the slot index is recomputed here via `gld_slot`.
            let idx_gld = self.gld_slot(fnd_org);

            // add to gliding hashtable (:303-305)
            self.nodes[ci].gld = self.gld_tbl[idx_gld];
            self.gld_tbl[idx_gld] = Some(ci);
        }

        // evaluate new (iiCnt==1) or skipped (iiCmp==-3) elements (:308-355)
        let mut ret = MchRet::Enlarged; // return code
        let ci = cur.unwrap();
        if self.nodes[ci].cnt == 1 || matches!(self.nodes[ci].cmp, CmpVal::Skp) {
            // reactivate skipped elements (:312-313)
            if matches!(self.nodes[ci].cmp, CmpVal::Skp) {
                self.nodes[ci].cmp = CmpVal::Run(0);
            }

            ret = self.is_good_or_best(red_new, ci, org, newf);
            match ret {
                MchRet::Invalid => {
                    if self.nodes[ci].tst >= self.nodes[ci].r#new {
                        // Invalids are marked -1 for reuse (unless they were
                        // incompletely evaluated) (:318-320)
                        self.hsh_rpr += 1; // miHshRpr++
                        self.nodes[ci].cmp = CmpVal::Inv; // mark as invalid for reuse

                        // put new invalid elements in front of the new list
                        // to be reused (:322-328)
                        if self.nodes[ci].cnt == 1 {
                            if self.mp_new.is_none() {
                                self.mp_lst = Some(ci);
                            }
                            self.nodes[ci].nxt = self.mp_new;
                            self.mp_new = Some(ci);
                        }
                    } else {
                        // Invalids that were not fully evaluated are treated
                        // like valids, so no break (:332-339)
                        if self.nodes[ci].cnt == 1 {
                            self.add_new(ci);
                        }
                    }
                }
                MchRet::Valid | MchRet::Good | MchRet::Best => {
                    // put new valid elements on the new elements list (:337-338)
                    if self.nodes[ci].cnt == 1 {
                        self.add_new(ci);
                    }
                }
                MchRet::Enlarged | MchRet::Error | MchRet::Full => {
                    // should not occur (:341-345)
                }
            } /* switch */

            // debug reporting (:349-354)
            #[cfg(feature = "debug")]
            if dbg(DBGMCH) {
                dbg_print(format_args!(
                    "Add         [  :{}>{}<{}] bse={} ret={}\n",
                    p8(fnd_org),
                    p8(dlt),
                    p8(fnd_new),
                    red_new,
                    ret as i32
                ));
            }
        } /* if enlarged else add */

        // Check if there's still room for new elements (:357-361)
        if self.mch_fre == 0 && self.mp_old.is_none() {
            MchRet::Full // table is full
        } else {
            ret // Good, bad or ugly :-)
        }
    } /* add() */

    /// Join colliding matches (`JMatchTable.cpp:195-210`): walk the delta
    /// bucket for a match with the same delta and merge into it. Returns the
    /// found node index, or `None` when the walk exhausts.
    fn join_colliding(&mut self, dlt: i64, fnd_new: i64) -> Option<usize> {
        let idx_dlt = (dlt.abs() % i64::from(self.mch_pme)) as usize; // abs(lzDlt) % miMchPme (:197)
        let mut cur = self.col_tbl[idx_dlt];
        while let Some(ci) = cur {
            if self.nodes[ci].dlt == dlt {
                // remove from gliding matches if single-counted (:201-202)
                if self.nodes[ci].cnt == 1 {
                    self.del_gld(ci);
                }

                // add to colliding match (:205-206)
                self.nodes[ci].cnt += 1;
                self.nodes[ci].r#new = fnd_new;

                return Some(ci);
            } /* if colliding */
            cur = self.nodes[ci].col;
        } /* for colliding */
        None
    }

    /// Join gliding matches (`JMatchTable.cpp:212-237`): walk the org slot
    /// for a match at the same org position and merge into it. Returns the
    /// found node index, or `None` when the walk exhausts.
    fn join_gliding(&mut self, fnd_org: i64, fnd_new: i64) -> Option<usize> {
        // liIdxGld is assigned whenever the gliding scan runs (lpCur == null),
        // which is also the only case the new-element linking below reads it.
        // C++ `%` on a negative org yields a negative index (UB); the
        // engine only adds matches at non-negative org positions.
        let idx_gld = self.gld_slot(fnd_org);
        let mut cur = self.gld_tbl[idx_gld];
        while let Some(gi) = cur {
            if self.nodes[gi].org == fnd_org {
                // remove from colliding matches (:219-220)
                if self.nodes[gi].cnt == 1 {
                    self.del_col(gi);
                }

                // add to gliding match (:223-224)
                self.nodes[gi].cnt += 1;
                self.nodes[gi].r#new = fnd_new;

                // set gliding recurrence (:227-232)
                if self.nodes[gi].gldcnt == 0 {
                    if fnd_new <= self.nodes[gi].beg + i64::from(SMPSZE) {
                        // C++: int assignment of an off_t difference.
                        self.nodes[gi].gldcnt = (fnd_new - self.nodes[gi].beg) as i32;
                    } else {
                        self.nodes[gi].gldcnt = SMPSZE;
                    }
                }

                return Some(gi);
            } /* if gliding */
            cur = self.nodes[gi].gld;
        } /* for gliding */
        None
    }

    /// Get the best (=nearest) optimized and valid match from the array of
    /// matches (`JMatchTable::getbest`, `JMatchTable.cpp:117-171`).
    ///
    /// Returns the tracked best `(org, new)` positions, or `None` when no
    /// solution has been found — the best was elected incrementally by
    /// `JMatchTable::is_best` during `add`/`cleanup`; no rescanning happens
    /// here. With `cmp_all` off, enlarged EOB elements are re-evaluated first
    /// (`:124-145`).
    pub fn getbest(&mut self, red_org: i64, red_new: i64) -> Option<(i64, i64)> {
        let _ = red_org; // azRedOrg: accepted, unused — as in the C++ body

        // Re-evaluate enlarged EOB's (because they are evaluated based on
        // iiCnt) (:124-145)
        if !self.cmp_all {
            // join old and new lists
            self.merge_new_into_old();

            // evaluate
            let mut bst_eob = false;
            let mut lp_cur = self.mp_old;
            while let Some(ci) = lp_cur {
                let cmp = self.nodes[ci].cmp;
                let tst = self.nodes[ci].tst;
                let nnew = self.nodes[ci].r#new;
                if Some(ci) != self.mp_bst
                    // EOB ? (`cmp <= CMPEOB`: the sentinel or an Est estimate
                    // below it — Est is stored only as values <= -4)
                    && (matches!(cmp, CmpVal::Eob)
                        || matches!(cmp, CmpVal::Est(v) if v <= CMPEOB))
                    && nnew > tst // Enlarged ? //@flawed !
                    && self.is_best(ci, red_new, 0, tst, cmp.as_legacy_i32())
                {
                    bst_eob = true;
                }
                lp_cur = self.nodes[ci].nxt;
            }

            // recalc mzBstOrg if needed (:142-144)
            if bst_eob && self.z_bst_org == 0 {
                if let Some(bst) = self.mp_bst {
                    let (mut o, mut n) = (self.z_bst_org, self.z_bst_new);
                    self.calc_pos_org(bst, &mut o, &mut n);
                    self.z_bst_org = o;
                    self.z_bst_new = n;
                }
            }
        }

        // get best match (:147-151)
        let ret = self.mp_bst.map(|_| (self.z_bst_org, self.z_bst_new));

        // debug feedback (:153-168)
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) {
            match self.mp_bst {
                None => {
                    dbg_print(format_args!("Match Failure at {}\n", red_new));
                }
                Some(b) => {
                    let bst_new = self.z_bst_new;
                    let bst_cmp = self.nodes[b].cmp.as_legacy_i32();
                    if red_new != bst_new {
                        dbg_print(format_args!(
                            "Suboptimal Match at {}: from {}({}), length {}\n",
                            red_new,
                            bst_new,
                            bst_new - red_new,
                            bst_cmp
                        ));
                    } else if bst_cmp < EQLSZE {
                        dbg_print(format_args!(
                            "Short Match at {}: from {}, length {}\n",
                            red_new, bst_new, bst_cmp
                        ));
                    } else {
                        dbg_print(format_args!(
                            "Optimal Match at {}: from {}, length {}\n",
                            red_new, bst_new, bst_cmp
                        ));
                    }
                }
            }
        }

        ret
    } /* getbest() */

    /// Cleanup, check free space and fastcheck best match
    /// (`JMatchTable::cleanup`, `JMatchTable.cpp:373-438`).
    ///
    /// `rlb` is the hashtable's current reliability (`mpHsh->get_reliability()`,
    /// `:377` — mechanically passed, see the module docs); `bse_org` is
    /// accepted but unused, like the C++ `azBseOrg` parameter.
    pub fn cleanup(
        &mut self,
        bse_org: i64, // azBseOrg: cleanup all matches before this position (unused, as in the C++)
        red_new: i64, // azRedNew: current reading position
        rlb: i32,
        org: &mut dyn JFile,
        newf: &mut dyn JFile,
    ) -> MchRet {
        let _ = bse_org;

        // get actual reliability distance (:377)
        self.rlb = rlb;

        // join old and new lists (:379-385)
        self.merge_new_into_old();

        // sanity checks (:387-397)
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) {
            self.dbg_check_table_size(false);
        }

        // evaluate existing entries (:399-407)
        self.mp_bst = None; // reset best pointer
        self.z_old = red_new;

        let mut lp_cur = self.mp_old;
        while let Some(ci) = lp_cur {
            if self.is_old2_skip(ci, red_new) {
                self.nodes[ci].cmp = CmpVal::Skp; // Mark very old elements as skipped
            } else {
                self.is_good_or_best(red_new, ci, org, newf);
            }
            lp_cur = self.nodes[ci].nxt;
        }

        // prepare the oldlist (:410)
        self.nextold(red_new, org, newf);

        // redo sanity checks (:413-423)
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) {
            self.dbg_check_table_size(true);
        }

        // issue return value (:425-437)
        if self.mp_old.is_none() && self.mch_fre == 0 {
            MchRet::Full //
        } else if self.mp_bst.is_none() {
            MchRet::Invalid //
        } else if self.z_bst_new != red_new {
            MchRet::Valid //
        } else if self.i_bst_cmp >= EQLMAX {
            MchRet::Best //
        } else if self.i_bst_cmp >= EQLSZE {
            MchRet::Good //
        } else {
            MchRet::Valid //
        }
    } /* cleanup() */

    /// Debug sanity walk (JMatchTable.cpp:387-397, also `:413-423`, bounded
    /// redo pass): counts the new and old
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

    /// Evaluate a match (`JMatchTable::isGoodOrBest`,
    /// `JMatchTable.cpp:443-538`).
    ///
    /// `lzDst` is only read by the #if debug doublecheck, like the C++ —
    /// release builds legitimately never read the assignment.
    #[cfg_attr(not(feature = "debug"), allow(unused_assignments))]
    fn is_good_or_best(
        &mut self,
        red_new: i64, // azRedNew: current read position
        cur: usize,   // lpCur: element to evaluate
        org: &mut dyn JFile,
        newf: &mut dyn JFile,
    ) -> MchRet {
        /* check if the match yields a solution on this position (:455) */
        let mut tst_new = red_new; // lzTstNew: start test at current read position

        /* calculate the test position on the original file by applying
         * izDlt (:458) */
        let mut tst_org: i64 = 0; // lzTstOrg
        let gliding = self.calc_pos_org(cur, &mut tst_org, &mut tst_new); // lbGld

        /* reuse earlier compare result (:461-498) */
        // lzDst: distance: number of bytes to compare before failing; -1
        // marks the reuse branches for the debug doublecheck. Only the
        // #if debug block reads it, like the C++.
        #[cfg_attr(
            not(feature = "debug"),
            allow(unused_mut, unused_variables, unused_assignments)
        )]
        let mut dst: i64 = -1;
        let mut cur_cmp: i32; // liCurCmp: current match compare state

        {
            let n = &self.nodes[cur];
            if tst_new <= n.tst {
                // The test position is still before the previous test result,
                // so reuse the previous test result. (:463-474)
                let mut cc = n.cmp;
                if matches!(cc, CmpVal::Skp | CmpVal::Inv) {
                    cc = CmpVal::Run(0);
                }
                if gliding {
                    tst_new = n.tst;
                    tst_org = n.org;
                } else {
                    tst_org += n.tst - tst_new;
                    tst_new = n.tst;
                }
                cur_cmp = cc.as_legacy_i32();
            } else {
                // Reuse candidate for the branch below. MSRV 1.85: let chains
                // stabilize in 1.88, so the chained condition
                // `!gliding && let CmpVal::Run(c) = n.cmp && c > 0 && …`
                // is hoisted into a precomputed match (same conjunct set —
                // every part is pure, so evaluation order is irrelevant).
                let reuse = if !gliding {
                    match n.cmp {
                        CmpVal::Run(c)
                            if c > 0 && n.tst - tst_new + i64::from(c) > i64::from(EQLMIN) =>
                        {
                            Some((n.tst - tst_new + i64::from(c)) as i32)
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(cur) = reuse {
                    // The new test position is within the previous test result.
                    // Report the remaining length (:476-478). C++: int assignment
                    // of an off_t expression.
                    cur_cmp = cur;
                } else {
                    // The previous test result cannot be reused: check (again)
                    // determine number of bytes to check (:482-486)
                    let mut d = n.beg - tst_new; // lzDst
                    // The C++ spells the clamp as if/else-if (:483-486) — kept 1:1.
                    #[allow(clippy::manual_clamp)]
                    if d < MINDST {
                        d = MINDST;
                    } else if d > MAXDST {
                        d = MAXDST;
                    }
                    dst = d;

                    // check (:489-490): cmp_all reads hard, otherwise soft
                    let sft = if self.cmp_all {
                        ReadType::HardAhead
                    } else {
                        ReadType::SoftAhead
                    };
                    let gld_arg = if gliding { n.gldcnt } else { 0 };
                    // C++ passes the off_t lzDst to check's int aiLen (truncating).
                    cur_cmp = check(
                        org,
                        newf,
                        &mut tst_org,
                        &mut tst_new,
                        d as i32,
                        gld_arg,
                        sft,
                    );

                    // store result (:493-497)
                    let n = &mut self.nodes[cur];
                    n.tst = tst_new;
                    if matches!(n.cmp, CmpVal::Inv) && cur_cmp <= 0 {
                        // don't erase an invalid marker
                    } else {
                        n.cmp = CmpVal::from_legacy_i32(cur_cmp);
                    }
                }
            }
        }

        // Debug doublecheck (:501-518): only for the reuse branches (dst == -1).
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) && dst == -1 {
            let mut chk_org = tst_org;
            let mut chk_new = tst_new;
            let sft = if self.cmp_all {
                ReadType::HardAhead
            } else {
                ReadType::SoftAhead
            };
            let chk_cmp = check(org, newf, &mut chk_org, &mut chk_new, 0, 0, sft);
            if (chk_cmp == 0 && cur_cmp == 0) || chk_cmp == CMPEOB {
                // that's ok
            } else if (chk_cmp != cur_cmp && self.nodes[cur].cmp.as_legacy_i32() < EQLMAX)
                || chk_org != tst_org
                || chk_new != tst_new
            {
                dbg_print(format_args!(
                    "Mch Chk Err :{}={}+{} Chk: {}={}+{}!\n",
                    tst_org, tst_new, cur_cmp, chk_org, chk_new, chk_cmp
                ));
            }
        }

        // If iiCmp>=EQLMAX, the test result probably extends till izNew (:521-523)
        if self.nodes[cur].cmp.as_legacy_i32() >= EQLMAX
            && self.nodes[cur].r#new > tst_new + i64::from(cur_cmp)
        {
            // C++: int += off_t (truncating).
            cur_cmp = cur_cmp.wrapping_add((self.nodes[cur].r#new - tst_new) as i32);
        }

        // evaluate: keep the best solution (:526)
        self.is_best(cur, red_new, tst_org, tst_new, cur_cmp);

        if cur_cmp == 0 {
            MchRet::Invalid //
        } else if tst_new != red_new {
            MchRet::Valid //
        } else if cur_cmp >= EQLMAX {
            MchRet::Best //
        } else if cur_cmp >= EQLSZE {
            MchRet::Good //
        } else {
            MchRet::Valid //
        }
    } /* isGoodOrBest */

    /// Check if given solution is the best one (`JMatchTable::isBest`,
    /// `JMatchTable.cpp:543-656`).
    fn is_best(
        &mut self,
        cur: usize,       // lpCur
        red_new: i64,     // azRedNew
        mut tst_org: i64, // lzTstOrg
        mut tst_new: i64, // lzTstNew
        mut cur_cmp: i32, // liCurCmp: legacy-encoded compare scalar (0, CMPEOB, a run length, or a negative EOB estimate)
    ) -> bool {
        let mut cur_cnt: i32 = -1; // liCurCnt: current match confirmation count

        /* Evaluate potential of EOB matches (:553-587) */
        if cur_cmp <= CMPEOB {
            // EOB was reached, so rely on info from the hashtable: iiCnt,
            // izBeg and izNew
            if cur_cnt < 0 {
                cur_cnt = if self.nodes[cur].gldcnt > 0 {
                    1 + self.nodes[cur].cnt / 2
                } else {
                    self.nodes[cur].cnt
                };
            }

            if tst_new <= self.nodes[cur].beg {
                // We're still before the first detected match, so a potential
                // solution probably starts at given match
                cur_cmp = cur_cnt;
                tst_new = self.nodes[cur].beg;
                tst_org = self.nodes[cur].org;
            } else if tst_new <= self.nodes[cur].r#new + i64::from(self.rlb) {
                // We're in between the first and last detected match:
                // Estimate the number of bytes needed to reach an equality.
                cur_cmp = cur_cnt;
                let d = 1 + i64::from(self.rlb) - i64::from(self.rlb.min(self.nodes[cur].cnt));
                tst_new += d;
                tst_org += d;
            } else {
                // The match is aging, reduce iiCnt by its age and estimate
                // the distance to an equality. C++: int assignment of an
                // off_t quotient (miRlb / 8 == 0 divides by zero in both).
                cur_cmp = cur_cnt
                    - 1
                    - ((tst_new - self.nodes[cur].r#new) / i64::from(self.rlb / 8)) as i32;
                let d = i64::from(cur_cnt - cur_cmp);
                tst_new += d;
                tst_org += d;
            }
            if cur_cmp < 1 {
                cur_cmp = 1; // something may be there, better than nothing
            } else {
                cur_cmp = 1 + EQLMAX.min(cur_cmp) / 2; // reduce hashtable match, real compares are better
            }

            // store result for isOld functions, negate to indicate EOB (:584-586)
            if cur_cmp > 3 {
                self.nodes[cur].cmp = CmpVal::Est(-cur_cmp);
            }
        }

        /* Elect the best one (:590-611) */
        if cur_cmp > 0 {
            if self.mp_bst.is_none() {
                self.mp_bst = Some(cur); // first one, take it
            } else if cur_cmp < 2 && self.i_bst_cmp > 4 {
                // do nothing to avoid using low-quality matches
                // (liCurCmp < 2 == low quality)
            } else if self.i_bst_cmp < 2 && cur_cmp > 4 {
                self.mp_bst = Some(cur); // avoid using low-quality matches (liBstCmp < 2 == low quality)
            } else if tst_new + FZY < self.z_bst_new {
                self.mp_bst = Some(cur); // new one is clearly better (nearer)
            } else if tst_new <= self.z_bst_new + FZY {
                // maybe better (nearer): check in more detail
                if tst_new - i64::from(cur_cmp) < self.z_bst_new - i64::from(self.i_bst_cmp) {
                    self.mp_bst = Some(cur); // new one is longer
                } else if tst_new - i64::from(cur_cmp) == self.z_bst_new - i64::from(self.i_bst_cmp)
                {
                    // If all else is equal, then rely on the hash counter
                    if cur_cnt < 0 {
                        // note: no `1 +` here, unlike the EOB branch above
                        cur_cnt = if self.nodes[cur].gldcnt > 0 {
                            self.nodes[cur].cnt / 2
                        } else {
                            self.nodes[cur].cnt
                        };
                    }
                    let b = self.mp_bst.expect("mpBst non-null in the elect chain");
                    let bst_cnt = if self.nodes[b].gldcnt > 0 {
                        self.nodes[b].cnt / 2
                    } else {
                        self.nodes[b].cnt
                    };
                    if cur_cnt > bst_cnt {
                        self.mp_bst = Some(cur); // higher hash-match counter = probably longer
                    }
                }
            }

            if self.mp_bst == Some(cur) {
                self.z_bst_new = tst_new;
                self.z_bst_org = tst_org;
                self.i_bst_cmp = cur_cmp;

                // Determine the limit for being old (:619-626):
                // - current mpBst runs till izTst + iiCmp, so all matches
                //   before this point are useless
                // - except if a new mpBst is found that is earlier but shorter
                // - therefore, miRlb is used as safety range
                self.z_old = self.nodes[cur].tst
                    + i64::from(self.nodes[cur].cmp.as_legacy_i32().min(0))
                    - i64::from(self.rlb);
                if self.z_old < red_new {
                    self.z_old = red_new;
                }
            }
        } /* if liCurCmp > 0 */

        // debug feedback (:632-653)
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) {
            let n = &self.nodes[cur];
            let val_old_inv = if cur_cmp > 0 {
                "Val"
            } else if n.r#new < red_new {
                "Old"
            } else {
                "Inv"
            };
            dbg_print(format_args!(
                "{} {:5} {} [{:2}:{}>{}<{}~{}#{:4}:{}+{:4}] bse={} fnd={}={}({})\n",
                val_old_inv,
                cur_cmp,
                if self.mp_bst == Some(cur) { '*' } else { ' ' },
                n.gldcnt,
                p8(n.org),
                p8(n.dlt),
                p8(n.beg),
                p8(n.r#new),
                n.cnt,
                p8(n.tst),
                n.cmp.as_legacy_i32(),
                red_new,
                tst_org,
                tst_new,
                tst_new - red_new,
            ));

            // Measure old distance (function-local static in the C++)
            let dist = red_new - n.r#new;
            if cur_cmp > 0 && dist > LL_OLD_MAX.load(Ordering::Relaxed) {
                LL_OLD_MAX.store(dist, Ordering::Relaxed);
                dbg_print(format_args!("Mch Old Max Distance = {}\n", dist));
            }
        }

        self.mp_bst == Some(cur)
    } /* isBest */

    /// Prepare next reusable old element (`JMatchTable::nextold`,
    /// `JMatchTable.cpp:662-724`). Returns true = found, false = notfound.
    fn nextold(&mut self, red_new: i64, org: &mut dyn JFile, newf: &mut dyn JFile) -> bool {
        // The file readers back only the debug-verify block below, like the
        // C++ #if debug — silent in release builds.
        #[cfg(not(feature = "debug"))]
        let _ = (org, newf);

        // find first old item on old list (:669-680)
        while let Some(head) = self.mp_old {
            if self.is_old2_reuse(head, red_new) {
                break;
            } else {
                // not an old item: remove from oldlist
                self.mp_old = self.nodes[head].nxt;

                // add to newlist
                self.add_new(head);
            }
        }

        // reuse new invalid items (marked with iiCmp == -1) (:682-700)
        if self.mp_old.is_none() && self.mp_new.is_some() {
            let lst = self
                .mp_lst
                .expect("mpNew non-null implies mpLst non-null (C++ invariant)");
            self.nodes[lst].nxt = None; // mpLst->ipNxt = null
            let mut lp_cur = self.mp_new;
            while let Some(ci) = lp_cur {
                if !matches!(self.nodes[ci].cmp, CmpVal::Inv) {
                    break;
                }
                // Remove from new list
                self.mp_new = self.nodes[ci].nxt;
                let nxt = self.nodes[ci].nxt;

                if self.nodes[ci].cnt > 1 && self.nodes[ci].r#new > self.nodes[ci].tst {
                    // Reactivate an enlarged invalid: move to end of newlist
                    self.nodes[ci].cmp = CmpVal::Run(0);
                    self.add_new(ci);
                } else {
                    // Move to old list
                    self.nodes[ci].nxt = self.mp_old;
                    self.mp_old = Some(ci);
                    break;
                }
                // The for-increment follows the stale next link, exactly like
                // the C++ (the arena never frees, so the value is defined).
                lp_cur = nxt;
            }
        }

        // debug-verify (:702-721)
        #[cfg(feature = "debug")]
        if dbg(DBGMCH) {
            if let Some(head) = self.mp_old {
                let mut chk_new = red_new;
                let mut chk_org = if self.nodes[head].gldcnt > 0 {
                    self.nodes[head].org
                } else {
                    chk_new + self.nodes[head].dlt
                };
                let sft = if self.cmp_all {
                    ReadType::HardAhead
                } else {
                    ReadType::SoftAhead
                };
                let cmp = check(org, newf, &mut chk_org, &mut chk_new, 32, 0, sft);
                if cmp > 0 {
                    let n = &self.nodes[head];
                    dbg_print(format_args!(
                        "Mch Nxt Err [{:2}:{}>{}<{}~{}#{:4}:{}+{:4}] bse={} tst:{}-{}({})={} is not invalid !\n",
                        n.gldcnt,
                        p8(n.org),
                        p8(n.dlt),
                        p8(n.beg),
                        p8(n.r#new),
                        n.cnt,
                        p8(n.tst),
                        n.cmp.as_legacy_i32(),
                        red_new,
                        p8(chk_org),
                        p8(chk_new),
                        (chk_new - red_new) as i32,
                        cmp,
                    ));
                }
            }
        }

        self.mp_old.is_some()
    }

    /// Check if a match can be skipped (`isOld2Skip`,
    /// `JMatchTable.cpp:733-743`).
    ///
    /// Skipping is mainly done for performance reasons. Skipped items however
    /// are dropped from the matching table if they are not renewed, so
    /// skipping may improve accuracy. Adversely, skipping a useful match will
    /// reduce accuracy, so we need to be careful.
    fn is_old2_skip(&self, cur: usize, red_new: i64) -> bool {
        let n = &self.nodes[cur];
        match n.cmp {
            CmpVal::Skp => true,
            CmpVal::Inv | CmpVal::Run(0) => n.r#new + MAXDST <= red_new,
            // CMPEOB and default (any other compare result)
            other => {
                (n.r#new + MAXDST <= red_new)
                    && (n.tst + i64::from(other.as_legacy_i32().abs()) < red_new)
            }
        }
    }

    /// Check if a match can be reused (deleted) (`isOld2Reuse`,
    /// `JMatchTable.cpp:755-768`).
    ///
    /// Matches are never deleted but instead reused (overwritten) by new
    /// matches. The matchtable is "full" when no more matches can be reused.
    /// If the matchtable is full, searching must stop, which is bad.
    /// Reusing (overwriting) a still usable match however is also bad.
    ///
    /// A match is considered still usable if may contain information beyond
    /// the current best match. This is flawed, because a next best match may
    /// be shorter than the current. So reusing valid matches is risky but
    /// necessary to maximize the search.
    fn is_old2_reuse(&self, cur: usize, red_new: i64) -> bool {
        let _ = red_new; // azRedNew: accepted, unused — as in the C++ body
        let n = &self.nodes[cur];
        match n.cmp {
            CmpVal::Skp => true,
            CmpVal::Inv => true,
            CmpVal::Eob => Some(cur) != self.mp_bst && n.r#new < self.z_old,
            CmpVal::Run(0) => n.r#new < n.tst || n.r#new < self.z_old,
            other => {
                Some(cur) != self.mp_bst
                    && n.r#new < self.z_old
                    && n.tst + i64::from(other.as_legacy_i32().abs()) < self.z_old
            }
        }
    }

    /// Calculate position on original file corresponding to given new file
    /// position (`calcPosOrg`, `JMatchTable.cpp:777-799`). Returns
    /// true = gliding offsets, false = normal offsets.
    fn calc_pos_org(&self, cur: usize, tst_org: &mut i64, tst_new: &mut i64) -> bool {
        /* calculate the test position on the original file by applying izDlt */
        if self.nodes[cur].gldcnt > 0 && *tst_new >= self.nodes[cur].beg {
            // we're within a gliding match
            *tst_org = self.nodes[cur].org;
            true
        } else {
            // we're before or after a gliding match
            // or on a colliding match
            if *tst_new + self.nodes[cur].dlt >= 0 {
                *tst_org = *tst_new + self.nodes[cur].dlt;
            } else {
                // azTstOrg would become negative, so advance azTstNew till
                // azTstOrg == 0 (:794-795)
                *tst_new = -self.nodes[cur].dlt;
                *tst_org = 0;
            }
            false
        }
    }

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

    /// Add element to the newlist (`addNew`, `JMatchTable.cpp:882-888`).
    ///
    /// The last node's `nxt` is deliberately left dangling ("saves one
    /// assignment", `:887`) — stale-chain walks rely on it, exactly as in
    /// the C++.
    fn add_new(&mut self, cur: usize) {
        if self.mp_new.is_none() {
            self.mp_new = Some(cur);
        } else {
            let lst = self
                .mp_lst
                .expect("mpNew non-null implies mpLst non-null (C++ invariant)");
            self.nodes[lst].nxt = Some(cur);
        }
        self.mp_lst = Some(cur);
    }

    /// Gliding-hashtable slot for `fnd_org`. The C++ computes this inline at
    /// both the gliding walk and the node insert (`JMatchTable.cpp:212-237`,
    /// `:308+`); one definition here keeps the two sites from drifting.
    fn gld_slot(&self, fnd_org: i64) -> usize {
        (fnd_org % i64::from(self.mch_pme)) as usize
    }

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

    /// Get number of hash repairs (matches repaired by comparing)
    /// (`getHshRpr`, `JMatchTable.cpp:930-932`). Instance counter — the
    /// 0.8.1 process-global static is retired (spec §18.E).
    pub fn get_hsh_rpr(&self) -> i32 {
        self.hsh_rpr
    }
}

/// Debug-only "Mch Old Max Distance" high-water mark (the C++ keeps a
/// function-local `static long llOldMax` inside `isBest`'s debug block,
/// `JMatchTable.cpp:645`).
#[cfg(feature = "debug")]
static LL_OLD_MAX: AtomicI64 = AtomicI64::new(0);

/// Verify and optimize matches (`JMatchTable::check`,
/// `JMatchTable.cpp:817-877`).
///
/// Searches at the given positions for a run of equal bytes, continuing for
/// `len` bytes unless soft reading stops at the end-of-buffer. On a mismatch
/// before EQLSZE, a gliding compare (`gld != 0`) rewinds the original
/// position to its anchor (`azPosOrg -= liEql`) and lets only the new
/// position slide; a non-gliding compare advances both.
///
/// Returns `0` = no equal bytes found (also for hard EOF),
/// [`CMPEOB`] = EOB reached, no equal bytes found, or the number of equal
/// bytes when strictly greater than [`EQLMIN`] (both positions rewound to
/// the run anchor).
fn check(
    org: &mut dyn JFile,
    newf: &mut dyn JFile,
    pos_org: &mut i64, // azPosOrg: in/out
    pos_new: &mut i64, // azPosNew: in/out
    mut len: i32,      // aiLen: number of bytes to compare
    gld: i32,          // aiGld: gliding match recurrence
    sft: ReadType,     // aiSft: 1=hard read, 2=soft read
) -> i32 {
    let mut lc_org = ByteOrEof::Byte(0); // lcOrg: byte from source file (C++ zero-init)
    let mut lc_new = ByteOrEof::Byte(0); // lcNew: byte from destination file
    let mut eql: i32 = 0; // liEql: equal bytes counter

    /* Debug: compare prologue (:825-830) */
    #[cfg(feature = "debug")]
    if dbg(DBGCMP) {
        dbg_print(format_args!(
            "Cmp {} ({},{},{:4},{}): ",
            if gld != 0 { "Gld" } else { "Col" },
            p8(*pos_org),
            p8(*pos_new),
            len,
            sft as i32,
        ));
    }

    /* Compare bytes (:833-855): the two break branches are separate
     * conditions in the C++ (`liEql >= EQLSZE` / `aiLen <= 0`) — kept 1:1. */
    #[allow(clippy::if_same_then_else)]
    while eql < EQLMAX {
        lc_org = org.get(*pos_org, sft);
        /* `lcOrg < 0` (:835): EOF, EOB (soft reads) and the error sentinels
         * end the compare; the sentinel stays in `lc_org` for the EOB check
         * below. */
        let ByteOrEof::Byte(lc_o) = &lc_org else {
            break;
        };
        lc_new = newf.get(*pos_new, sft);
        /* `lcNew < 0` (:838): same sentinels on the destination read. */
        let ByteOrEof::Byte(lc_n) = &lc_new else {
            break;
        };
        if lc_o == lc_n {
            *pos_org += 1;
            *pos_new += 1;
            eql += 1;
        } else if eql >= EQLSZE {
            break;
        } else if len <= 0 {
            break;
        } else {
            *pos_new += 1;
            if gld != 0 {
                *pos_org -= i64::from(eql); // glide: rewind to the anchor
            } else {
                *pos_org += 1;
            }
            eql = 0;
        }
        len -= 1;
    }

    /* Debug: compare result (:857-864) */
    #[cfg(feature = "debug")]
    if dbg(DBGCMP) {
        // boundary: the trace prints the raw i32 channel values
        // (`%02x`/print_char), including EOF/EOB sentinels.
        dbg_print(format_args!(
            "{} {} {:2} {} ({}){:02x} == ({}){:02x}\n",
            p8(*pos_org - i64::from(eql)),
            p8(*pos_new - i64::from(eql)),
            eql,
            if eql >= EQLMIN {
                "OK!"
            } else if matches!(&lc_org, ByteOrEof::Eob) || matches!(&lc_new, ByteOrEof::Eob) {
                "EOB"
            } else {
                "NOK"
            },
            print_char(lc_org.to_i32()),
            lc_org.to_i32() as u8,
            print_char(lc_new.to_i32()),
            lc_new.to_i32() as u8,
        ));
    }

    if eql > EQLMIN {
        *pos_org -= i64::from(eql);
        *pos_new -= i64::from(eql);
        eql
    } else if matches!(&lc_org, ByteOrEof::Eob) || matches!(&lc_new, ByteOrEof::Eob) {
        // EOB reached
        CMPEOB
    } else {
        // No equal bytes found
        0
    }
} /* check() */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jfile::{JFileAhead, JFileMem};
    use std::io::Cursor;

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

    /// Both files all-zero: every compare succeeds, so `check` runs to the
    /// EQLMAX cap and verified matches classify as Best.
    fn zeros(n: usize) -> JFileMem {
        JFileMem::new(vec![0u8; n])
    }

    /// Brief step-1 test: two-table bucket math (spec §18.E). The collision
    /// table hashes `|delta| % pme` (`JMatchTable.cpp:197`), the gliding
    /// table `org % pme` (`:215`); pme = get_lower_prime(2*sze) = 127 for a
    /// 64-element table. The C `abs` macro shares one bucket between a delta
    /// and its negation (−960 and +960 both land in bucket 71).
    #[test]
    fn two_table_bucket_math() {
        // 2 * 64 = 128 -> get_lower_prime = 127.
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(m.mch_pme, 127);
        let mut org = zeros(4096);
        let mut new = zeros(4096);

        // delta 960 -> col bucket 960 % 127 = 71; org 1000 -> gld bucket 111.
        assert_eq!(m.add(1000, 40, 50, &mut org, &mut new), MchRet::Best);
        let n0 = m.col_tbl[71].unwrap();
        assert_eq!(m.gld_tbl[111], Some(n0));
        assert_eq!(m.nodes[n0].dlt, 960);
        assert_eq!(m.nodes[n0].cnt, 1);
        assert_eq!(m.nodes[n0].beg, 40);
        assert_eq!(m.nodes[n0].r#new, 40);
        assert_eq!(m.nodes[n0].org, 1000);

        // delta -960: the C `abs` macro maps it to the same bucket 71; a new
        // node is prepended (960 != -960), gliding bucket on org 40 (org 40
        // is below pme 127, so bucket = 40).
        assert_eq!(m.add(40, 1000, 1050, &mut org, &mut new), MchRet::Best);
        let n1 = m.col_tbl[71].unwrap();
        assert_ne!(n1, n0);
        assert_eq!(m.nodes[n1].col, Some(n0));
        assert_eq!(m.nodes[n1].dlt, -960);
        assert_eq!(m.gld_tbl[40], Some(n1));
    }

    /// Brief step-1 test: gliding joins (0.8.5 style) happen on an equal ORG
    /// position (`JMatchTable.cpp:216-236`), not — as in 0.8.1 — on a
    /// decrementing delta. The first join removes the node from the collision
    /// table (cnt was 1, `:219-220`) and sets the gliding recurrence
    /// (`:227-232`): `fnd_new - beg` when within beg + SMPSZE, else SMPSZE.
    #[test]
    fn gliding_join_sets_recurrence() {
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        let mut org = zeros(8192);
        let mut new = zeros(8192);

        // Fresh match org 1000 / new 500 (delta 500).
        assert_eq!(m.add(1000, 500, 600, &mut org, &mut new), MchRet::Best);
        let n0 = m.gld_tbl[1000 % 127].unwrap();
        assert_eq!(m.col_tbl[500 % 127], Some(n0));

        // Same org again: gliding join, cnt 2, recurrence capped at SMPSZE.
        assert_eq!(m.add(1000, 700, 800, &mut org, &mut new), MchRet::Enlarged);
        assert_eq!(m.nodes[n0].cnt, 2);
        assert_eq!(m.nodes[n0].gldcnt, SMPSZE); // 700 > 500 + 32 -> SMPSZE
        assert_eq!(m.nodes[n0].r#new, 700);
        assert_eq!(m.nodes[n0].beg, 500); // unchanged
        // Removed from the collision table at the join (cnt was 1).
        assert_eq!(m.col_tbl[500 % 127], None);
        assert_eq!(m.gld_tbl[1000 % 127], Some(n0));

        // Second glide join: cnt 3, recurrence untouched (already set).
        assert_eq!(m.add(1000, 750, 800, &mut org, &mut new), MchRet::Enlarged);
        assert_eq!(m.nodes[n0].cnt, 3);
        assert_eq!(m.nodes[n0].gldcnt, SMPSZE);

        // A fresh gliding pair within beg + SMPSZE records the small offset.
        assert_eq!(m.add(2000, 3000, 3100, &mut org, &mut new), MchRet::Best);
        let n1 = m.gld_tbl[2000 % 127].unwrap();
        assert_eq!(
            m.add(2000, 3020, 3100, &mut org, &mut new),
            MchRet::Enlarged
        );
        assert_eq!(m.nodes[n1].gldcnt, 20); // 3020 - 3000
    }

    /// Brief step-1 test: the table is never "full" while old elements exist —
    /// they are reactivated (`isOld2Reuse`, `JMatchTable.cpp:755-768`), not an
    /// error. 13 adds exhaust the free counter; `cleanup` moves them onto the
    /// aging list (all marked CMPINV by EOF-refuted compares, hence always
    /// reusable); a 14th add reuses the aging head instead of failing.
    #[test]
    fn aging_list_reuse_when_full() {
        let mut m = JMatchTable::new(13, true, 1024).expect("test table"); // pme = get_lower_prime(26) = 23
        assert_eq!(m.mch_pme, 23);
        let mut org = zeros(4096);
        let mut new = zeros(4096);

        // 13 distinct deltas 0..=12 with matches behind red_new = 5000 (past
        // EOF): every compare refutes -> CMPINV marks -> always reusable.
        for i in 0..13i64 {
            let ret = m.add(200 + i * 100 + i, 200 + i * 100, 5000, &mut org, &mut new);
            if i < 12 {
                assert_eq!(ret, MchRet::Invalid);
            } else {
                // The 13th add consumed the last free element: Full.
                assert_eq!(ret, MchRet::Full);
            }
        }
        assert_eq!(m.mch_fre, 0);
        assert!(m.mp_old.is_none());

        // cleanup: CMPINV elements are reusable, so the aging head survives
        // (nextold breaks) and the best pointer stays null -> Invalid.
        assert_eq!(m.cleanup(0, 5000, 48, &mut org, &mut new), MchRet::Invalid);
        assert!(m.mp_old.is_some());

        // The 14th add reuses the aging head (node 0, the last allocated):
        // delta 100 -> col bucket |100| % 23 = 8, org 6100 -> gld bucket 5.
        assert_eq!(m.add(6100, 6000, 5000, &mut org, &mut new), MchRet::Invalid);
        assert_eq!(m.mch_fre, 0); // still no fresh element: reused instead
        assert_eq!(m.nodes[0].dlt, 100);
        assert_eq!(m.nodes[0].org, 6100);
        assert_eq!(m.nodes[0].r#new, 6000);
        // The reused node left its old collision bucket (12) and now heads
        // bucket 8 ahead of the delta-8 node (node 4, add i=8).
        assert_eq!(m.col_tbl[12], None);
        assert_eq!(m.col_tbl[8], Some(0));
        assert_eq!(m.nodes[0].col, Some(4));
        assert_eq!(m.nodes[4].dlt, 8);
    }

    /// Brief step-1 test, `-x 5` (spec §21.15, ruling 3): with the ctor value
    /// below 13, `miMchSze` is clamped to 13 (`JMatchTable.cpp:85`) while the
    /// bucket prime is derived from the UNCLAMPED value (`miMchPme =
    /// getLowerPrime(aiMchSze * 2)`, `:97` — get_lower_prime(10) = 7).
    ///
    /// Note: the written spec/ruling describe the quirk as "miMchFre from the
    /// unclamped x". The C++ initializer `miMchFre(miMchSze)` reads the
    /// already-initialized (clamped) member — member initializers run in
    /// declaration order — verified with g++ against the real ctor shape:
    /// for x = 5, miMchFre is 13, not 5. C++ wins over prose (task
    /// directive), so this test pins the C++-true values; the unclamped
    /// value's observable effect is the bucket prime below.
    #[test]
    fn mch_quirks_at_x5() {
        let mut m = JMatchTable::new(5, true, 1024).expect("test table");
        assert_eq!(m.mch_sze, 13); // max(13, 5)
        assert_eq!(m.mch_fre, 13); // miMchFre(miMchSze): clamped, per the C++
        assert_eq!(m.mch_pme, 7); // get_lower_prime(5 * 2): the unclamped quirk

        // Behavioral pin: 13 fresh elements are consumed before reuse — the
        // free count really started at 13, not at 5 (with 5, the 6th add
        // would already return Full).
        let mut org = zeros(16384);
        let mut new = zeros(16384);
        for i in 0..12i64 {
            let ret = m.add(2000 + i * 50 + i, 2000 + i * 50, 1000, &mut org, &mut new);
            assert_eq!(ret, MchRet::Best, "add {i}");
        }
        assert_eq!(m.add(2650, 2600, 1000, &mut org, &mut new), MchRet::Full);
        assert_eq!(m.mch_fre, 0);
    }

    /// Brief step-1 test: incremental best-tracking (`isBest` during
    /// add/cleanup, `JMatchTable.cpp:543-656`) — `getbest` returns the
    /// tracked best without rescanning. Two verified candidates tie on the
    /// test position (red_new) and length (EQLMAX), so the hash-confirmation
    /// counter decides (`:603-610`).
    #[test]
    fn best_tracking_via_getbest() {
        let mut org = zeros(8192);
        let mut new = zeros(8192);

        // Scenario 1: counters tied at 1 — the first-elected candidate (A,
        // delta 0) keeps the best; B (delta 2000) would answer (4500, 2500).
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(m.add(2000, 2000, 2500, &mut org, &mut new), MchRet::Best); // A
        assert_eq!(m.add(3500, 1500, 2500, &mut org, &mut new), MchRet::Best); // B
        assert_eq!(m.cleanup(0, 2500, 48, &mut org, &mut new), MchRet::Best);
        assert_eq!(m.getbest(0, 2500), Some((2500, 2500)));

        // Scenario 2: B is confirmed twice (cnt 2) and wins the tiebreak.
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(m.add(2000, 2000, 2500, &mut org, &mut new), MchRet::Best); // A
        assert_eq!(m.add(3500, 1500, 2500, &mut org, &mut new), MchRet::Best); // B
        assert_eq!(
            m.add(3500, 1500, 2500, &mut org, &mut new),
            MchRet::Enlarged
        );
        assert_eq!(m.cleanup(0, 2500, 48, &mut org, &mut new), MchRet::Best);
        assert_eq!(m.getbest(0, 2500), Some((4500, 2500)));

        // No candidates at all: no solution.
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(m.cleanup(0, 2500, 48, &mut org, &mut new), MchRet::Invalid);
        assert_eq!(m.getbest(0, 2500), None);
    }

    /// Brief step-1 test: the cleanup return taxonomy (`JMatchTable.cpp:425-437`):
    /// Invalid (no best), Valid (best shorter than EQLSZE or away from the
    /// read position), Good (>= EQLSZE), Best (>= EQLMAX).
    #[test]
    fn cleanup_return_taxonomy() {
        // Invalid: nothing in the table, room left.
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        let mut org = zeros(4096);
        let mut new = zeros(4096);
        assert_eq!(m.cleanup(0, 1000, 48, &mut org, &mut new), MchRet::Invalid);

        // Best: a fully verified 256-byte run at the read position.
        assert_eq!(m.add(1500, 1000, 1000, &mut org, &mut new), MchRet::Best);
        assert_eq!(m.cleanup(0, 1000, 48, &mut org, &mut new), MchRet::Best);

        // Good: the run stops at 100 bytes (EQLSZE <= cmp < EQLMAX). A fresh
        // table: the delta-500 match of the Best case above would otherwise
        // swallow this add as a colliding Enlarged join.
        let mut m = JMatchTable::new(64, true, 1024).expect("test table");
        let mut org = {
            let mut d = vec![1u8; 4096];
            d[1500..1600].fill(7);
            JFileMem::new(d)
        };
        let mut new = {
            let mut d = vec![2u8; 4096];
            d[1000..1100].fill(7);
            JFileMem::new(d)
        };
        assert_eq!(m.add(1500, 1000, 1000, &mut org, &mut new), MchRet::Good);
        assert_eq!(m.cleanup(0, 1000, 48, &mut org, &mut new), MchRet::Good);

        // Valid: a 6-byte run (EQLMIN < cmp < EQLSZE) ending at EOF.
        let mut org = JFileMem::new(vec![5u8; 6]);
        let mut new = JFileMem::new(vec![5u8; 6]);
        assert_eq!(m.add(0, 0, 0, &mut org, &mut new), MchRet::Valid);
        assert_eq!(m.cleanup(0, 0, 48, &mut org, &mut new), MchRet::Valid);
    }

    /// Brief step-1 test: Full taxonomy — with the whole table parked on
    /// non-reusable EOB elements, `cleanup` reports Full and a further `add`
    /// answers Error ("should not occur", `JMatchTable.cpp:286`). The EOB
    /// marks need soft-ahead reads past the look-ahead window
    /// (`get_frombuffer` answers EOB beyond `pos_bse + buf_sze - blk_sze`).
    #[test]
    fn cleanup_full_when_nothing_reusable() {
        // cmp_all = false: the compares read SOFT ahead, which is what EOBs
        // at the window bound (with cmp_all the hard reads would sail past
        // the window and verify 256-byte runs instead).
        let mut m = JMatchTable::new(13, false, 1024).expect("test table");
        let mut org =
            JFileAhead::new(Cursor::new(vec![0u8; 8192]), "Tst", 1024, 16).expect("test alloc");
        let mut new =
            JFileAhead::new(Cursor::new(vec![0u8; 8192]), "Tst", 1024, 16).expect("test alloc");

        // 13 matches ahead of red_new = 2000: soft reads at 2000+ exceed the
        // fresh window (0 + 1024 - 16) and EOB, so every candidate is stored
        // with the CMPEOB mark and estimated length 1 (cnt 1).
        for i in 0..13i64 {
            let ret = m.add(2100 + i * 100 + i, 2100 + i * 100, 2000, &mut org, &mut new);
            if i < 12 {
                assert_eq!(ret, MchRet::Valid, "add {i}");
            } else {
                assert_eq!(ret, MchRet::Full, "add {i}");
            }
        }
        assert_eq!(m.mch_fre, 0);
        // All 13 kept the CMPEOB mark (-3): the EOB estimate of a cnt-1
        // candidate is 1, which is not stored back (> 3 required).
        assert!(m.nodes[..13].iter().all(|n| n.cmp == CmpVal::Eob));

        // cleanup: every element is either the elected best (never reusable)
        // or younger than mzOld = red_new (never reusable) -> the aging list
        // drains completely -> Full.
        assert_eq!(m.cleanup(0, 2000, 48, &mut org, &mut new), MchRet::Full);
        assert!(m.mp_old.is_none());

        // A further add has neither a free element nor an old one: Error.
        assert_eq!(m.add(4100, 4000, 2000, &mut org, &mut new), MchRet::Error);
    }

    /// Brief step-1 test: the repair counter lives on the instance
    /// (`miHshRpr`, `JMatchTable.cpp:319`; getter `:930-932`) — the 0.8.1
    /// global static is retired. Every compare-refuted match whose test
    /// reached its last-found position increments the counter.
    #[test]
    fn repairs_counter_on_instance() {
        let mut org = zeros(64);
        let mut new = zeros(64);

        // red_new 50 > last-found 10: the EOF-refuted compare marks the
        // element CMPINV and repairs the hash hit.
        let mut a = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(a.get_hsh_rpr(), 0);
        assert_eq!(a.add(100, 10, 50, &mut org, &mut new), MchRet::Invalid);
        assert_eq!(a.get_hsh_rpr(), 1);

        // A second instance is untouched — no process-global state.
        let b = JMatchTable::new(64, true, 1024).expect("test table");
        assert_eq!(b.get_hsh_rpr(), 0);
    }

    /// Brief step-1 test: glide realignment in `check` (`JMatchTable.cpp:843-854`)
    /// — on a mismatch before EQLSZE, a gliding compare rewinds the original
    /// position to its anchor (`azPosOrg -= liEql`) and lets only the new
    /// position slide, while a non-gliding compare advances both.
    #[test]
    fn check_glide_realignment() {
        // org: 1 2 3 4 4 4 4 4 9 9   new: 0 1 2 3 5 4 4 4 4 4 9 9
        // The 3-byte run 1 2 3 misaligns at new[4] = 5.
        let mut org = JFileMem::new(vec![1, 2, 3, 4, 4, 4, 4, 4, 9, 9]);
        let mut new = JFileMem::new(vec![0, 1, 2, 3, 5, 4, 4, 4, 4, 4, 9, 9]);

        // Gliding: after the mismatch the org anchor stays at 0 and slides
        // never resync (org[0] = 1 never reappears) — the new position walks
        // to the very end, the compare answers 0.
        let (mut po, mut pn) = (0i64, 1i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                1,
                ReadType::HardAhead
            ),
            0
        );
        assert_eq!((po, pn), (0, 12));

        // Non-gliding: both positions advance on the mismatch; the compare
        // resyncs onto the 4-run and walks to org's EOF with eql 1 (< EQLMIN).
        let (mut po, mut pn) = (0i64, 1i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                0,
                ReadType::HardAhead
            ),
            0
        );
        assert_eq!((po, pn), (10, 11));
    }

    /// Brief step-1 test (rewritten from the 0.8.1 24-byte pin): the equal-run
    /// cap is EQLMAX 256 (`JMatchTable.cpp:833`) and a found run rewinds both
    /// positions to the run anchor (`:866-869`).
    #[test]
    fn check_caps_at_eqlmax() {
        let mut org = zeros(4096);
        let mut new = zeros(4096);

        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                4096,
                0,
                ReadType::HardAhead
            ),
            256
        );
        assert_eq!((po, pn), (0, 0)); // advanced 256, rewound 256

        // Exactly EQLMIN equal bytes are NOT a solution (liEql > EQLMIN
        // required): 5 equals then EOF answer 5; 4 answer 0.
        let mut org = JFileMem::new(vec![7u8; 5]);
        let mut new = JFileMem::new(vec![7u8; 5]);
        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                0,
                ReadType::HardAhead
            ),
            5
        );
        assert_eq!((po, pn), (0, 0));

        let mut org = JFileMem::new(vec![7u8; 4]);
        let mut new = JFileMem::new(vec![7u8; 4]);
        let (mut po, mut pn) = (0i64, 0i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                0,
                ReadType::HardAhead
            ),
            0
        );
        assert_eq!((po, pn), (4, 4)); // no rewind: not a solution
    }

    /// Brief step-1 test (rewritten from the 0.8.1 pin): a soft read beyond
    /// the look-ahead window answers EOB, which `check` reports as CMPEOB
    /// ("maybe, buffer ended") with the positions untouched; a hard read at
    /// the real end of file answers EOF, which is 0 ("surely unequal").
    #[test]
    fn check_cmpeob_on_soft_eob() {
        let mut org =
            JFileAhead::new(Cursor::new(vec![0u8; 8192]), "Tst", 1024, 16).expect("test alloc");
        let mut new =
            JFileAhead::new(Cursor::new(vec![0u8; 8192]), "Tst", 1024, 16).expect("test alloc");
        let (mut po, mut pn) = (2000i64, 2000i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                0,
                ReadType::SoftAhead
            ),
            CMPEOB
        );
        assert_eq!((po, pn), (2000, 2000));

        let mut org = zeros(10);
        let mut new = zeros(10);
        let (mut po, mut pn) = (10i64, 10i64);
        assert_eq!(
            check(
                &mut org,
                &mut new,
                &mut po,
                &mut pn,
                64,
                0,
                ReadType::HardAhead
            ),
            0
        );
        assert_eq!((po, pn), (10, 10));
    }
}
