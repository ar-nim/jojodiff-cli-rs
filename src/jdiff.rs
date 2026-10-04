//! JDiff engine: the main diffing loop, ported 1:1 from 0.8.5 C++
//! `src/JDiff.cpp` + `src/JDiff.h` (spec §18.E).
//!
//! [`JDiff::jdiff`] compares both files byte by byte and, on a mismatch,
//! calls `JDiff::search` to find the nearest equal region ahead:
//! `JDiff::build_full_index` (or the incremental scan with `src_scn == 0`,
//! `-ff`/`-p`) fills a [`JHashPos`] hashtable with 32-byte samples of the
//! original file, `search` probes it with samples of the new file and hands
//! the hits to a [`JMatchTable`], whose best verified match is turned into
//! DEL/BKT/INS instructions plus an ahead counter. The engine drives a
//! [`JOut`] sink; the `EQL` return-value contract (byte mode until the sink
//! grants length mode) is respected exactly as in the C++.
//!
//! # 0.8.5 engine shape (spec §18.E)
//!
//! * `int liFnd` is **live**: `search`'s negative return aborts `jdiff`
//!   immediately (`JDiff.cpp:277-279`) — the 0.8.1 `bool lbFnd` collapse
//!   (Part I §15.9) is fixed. Read errors surface through it and through the
//!   final `lcNew < EOB || lcOrg < EOB` check (`:330-332`).
//! * A "found" solution that pointed nowhere (`liFnd == 1 && lzAhd == 0`)
//!   counts `miHshErr` in **release** builds too (wrapping i32, spec §21.6)
//!   and, at verbose>2 with compare-all, prints
//!   `"\nInaccurate solution at positions %zd/%zd!\n"` (`:247-261`). The C++
//!   member is never initialized (constructor does not touch it), so its
//!   baseline is garbage there; this port deterministically starts at 0 and
//!   replicates the real increments.
//! * With `src_scn == 0` the source index builds incrementally: at the top
//!   of the compare loop, inside both equal-run fast loops (`:185-224`) and
//!   via the SoftAhead prescan bounded by `miAhdMax`/`mzAhdOrg` in `search`
//!   (`:419-447`).
//! * `search` (`:389-718`): lookahead budget
//!   `miAhdMax - (mzAhdNew - azRedNew)` floored at the cached reliability
//!   `miRlb` (`:464-470`); look-back capped at `miRlb + 2*SMPSZE - 1`
//!   (`:481-487`); the new-file hash re-initialization terminates early once
//!   `miEqlNew != liIdx` proves it correct (`:546-573`); the add switch is
//!   driven by the [`MchRet`] taxonomy (Full stops, Best/Good shorten to
//!   `miRlb*2`/`miRlb`, Valid counts toward `miMchMin`/`miMchMax` with
//!   soft-read switching); the backtrack clamp uses `getBufPos()` when
//!   backtracking is disabled (`:491,704-712`) and `mzAhdOrg` is **not**
//!   reset on backtrack anymore.
//! * `buildFullIndex` (`:726-793`) replaces the 0.8.1 prescan: no OpenMP,
//!   32 MiB progress marks (`PGSMRK`/`PGSMSK`) and a verbose>2
//!   hashtable distribution.
//! * Constructor (`:103-125`): `hsh_sze` in MB,
//!   `mch_min = mch_min > mch_max ? mch_max - 1 : mch_min`,
//!   `ahd_max = max(ahd_max, 1024)`.
//!
//! # Debug prints (spec §14, `debug` feature)
//!
//! The `#if debug` sites are ported with their exact C++ format strings
//! (debug `P8zd` width 10 via [`crate::defs::p8`]): the DBGPRG "Input" and
//! "Current position" traces (`JDiff.cpp:179-181,284-285`), the DBGAHD
//! "Findahead on" line and the "\nForcing skip of SMPSZE bytes\n" line
//! (`:281-283,681-683`), the debug ESC flush when DBGAHD or DBGMCH is set
//! (`:270-274`), the unconditional-in-debug "Matchtable overflow at" line
//! (`:600-601`) and the DBGAHH `ufHshAdd` lines in `buildFullIndex`'s
//! verbose>1 loop (`:758-762`). The 0.8.1 DBGHSK hash trace is gone at
//! 0.8.5 (the hash moved into this module, no print site) and the DBGDST
//! distribution is now verbose-driven (`:324-327,785-787`, release builds
//! included) — `-d dst` has zero sites (spec §18.G).

use crate::defs::{EOF, MAX_OFF_T, Op, ReadType, SMPSZE};
use crate::error::JDiffError;
use crate::jdebug::dbg_print;
#[cfg(feature = "debug")]
use crate::jdebug::{DBGAHD, DBGAHH, DBGMCH, DBGPRG, dbg};
use crate::jfile::{ByteOrEof, JFile};
use crate::jhashpos::JHashPos;
use crate::jmatchtable::{JMatchTable, MchRet};
use crate::jout::{JOut, OutStats};

/// Progress mark: show progress in Mb (`JDiff.cpp:95`,
/// `1024 * 1024 or 0x400 x 0x400`).
const PGSMRK: i64 = 0x100000;

/// Progress mask: show progress every 32Mb when `(lzPos & PGSMSK) == 0`
/// (`JDiff.cpp:96`).
const PGSMSK: i64 = 0x1ffffff;

/// Search-ahead state (`JDiff.h:264-274`): the rolling window `search`
/// advances. Extracted from `JDiff` so the scan methods can borrow the
/// state and the file readers disjointly — the C++ reaches into these
/// members through pointer aliasing, which the two field-disjoint
/// destructures used to emulate.
struct SearchState {
    /// Current ahead position on the original file (`mzAhdOrg`). Not reset on
    /// backtrack anymore (0.8.5).
    az_org: i64,
    /// Current ahead position on the new file (`mzAhdNew`).
    az_new: i64,
    /// Current hash value for the original file (`mlHshOrg`).
    hsh_org: u32,
    /// Current hash value for the new file (`mlHshNew`).
    hsh_new: u32,
    /// Previous file value, original (`miPrvOrg`).
    prv_org: i32,
    /// Current file value, new (`miValNew`). Typed read result: a data
    /// byte, or the sentinel (`EOF`/`EOB`/error) of the read that ended the
    /// scan — checked by the error gate after the scan loops.
    val_new: ByteOrEof,
    /// Previous file value, new (`miPrvNew`).
    prv_new: i32,
    /// Equal-run counter in the current sample, original (`miEqlOrg`).
    eql_org: i32,
    /// Equal-run counter in the current sample, new (`miEqlNew`).
    eql_new: i32,
    /// Reliability range for the current hashtable (`miRlb`, cached by
    /// `search` after each prescan; the 085ac tuning commit, spec §18.E).
    rlb: i32,

    /// Number of false hash hits (`miHshErr`): incremented on a "solution"
    /// that pointed nowhere, **in release builds too** (wrapping i32, spec
    /// §21.6). The C++ never initializes the member (constructor `:103-125`
    /// does not mention it) — its baseline is stack garbage there; this port
    /// deterministically starts at 0 and replicates the real increments.
    hsh_err: i32,
}

/// JDiff engine (`JDiff.h:149`): owns the two file readers, the output sink,
/// the hashtable and the matching table.
pub struct JDiff<'a> {
    /// Original file to read (`mpFilOrg`).
    org: Box<dyn JFile + 'a>,
    /// New file to read (`mpFilNew`).
    r#new: Box<dyn JFile + 'a>,
    /// Output handler (`mpOut`).
    out: Box<dyn JOut + 'a>,
    /// Hashtable containing hashes from `org` (`gpHsh`).
    hsh: JHashPos,
    /// Table of matches (`gpMch`).
    mch: JMatchTable,

    /// Verbosity level 0=no, 1=normal, 2=high (`miVerbse`).
    verbose: i32,
    /// Allow backtrack on the original file? (`mbSrcBkt`; C++ int, Rust bool).
    src_bkt: bool,
    /// Max number of matches to find (`miMchMax`).
    mch_max: i32,
    /// Min number of matches to find (`miMchMin`; clamped in the ctor,
    /// `JDiff.cpp:119`).
    mch_min: i32,
    /// Max number of bytes to look ahead (`miAhdMax`; C++ stores an int,
    /// raised to at least 1024 by the ctor, `JDiff.cpp:120`).
    ahd_max: i32,
    /// Compare all matches, even if data not in buffer? (`mbCmpAll`; gates
    /// the verbose>2 "Inaccurate solution" report, `JDiff.cpp:255`).
    cmp_all: bool,
    /// Prescan original file: 0=no, 1=yes, 2=done (`miSrcScn`).
    src_scn: i32,

    /// Search-ahead state (`JDiff.h:264-274`).
    sst: SearchState,
}

impl<'a> JDiff<'a> {
    /// Create JDiff for working on the specified files (`JDiff.cpp:103-125`).
    ///
    /// `hsh_sze` is the hashtable size in **MB** (0.8.5 `aiHshSze`, passed
    /// straight to [`JHashPos::new`], which converts MB to elements).
    /// `mch_min` clamps to `mch_max - 1` only when strictly greater
    /// (`miMchMin(aiMchMin > miMchMax ? miMchMax - 1 : aiMchMin)`, `:119`);
    /// `ahd_max` is raised to at least 1024 (`miAhdMax(aiAhdMax<1024?1024:
    /// aiAhdMax)`, `:120`) after the CLI-facing `i64` narrows like the C++
    /// int constructor parameter would. `src_scn` becomes the C++ `miSrcScn`
    /// int (false = 0, true = 1) and is set to 2 by `JDiff::search` after
    /// the full index build. The matching table receives the **unclamped**
    /// `ahd_max`, like the C++ passes the raw `aiAhdMax` to JMatchTable
    /// (`:124`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        org: Box<dyn JFile + 'a>,
        r#new: Box<dyn JFile + 'a>,
        out: Box<dyn JOut + 'a>,
        hsh_sze: i32,
        verbose: i32,
        src_bkt: bool,
        src_scn: bool,
        mch_max: i32,
        mch_min: i32,
        ahd_max: i64,
        cmp_all: bool,
    ) -> Self {
        // The C++ ctor parameter is `const int aiAhdMax`; the CLI cannot
        // produce values beyond the i32 range (atoi clamps), so narrowing
        // saturates only for values the C++ would already have truncated.
        let ahd_max = i32::try_from(ahd_max).unwrap_or(i32::MAX);
        JDiff {
            org,
            r#new,
            out,
            hsh: JHashPos::new(hsh_sze),
            mch: JMatchTable::new(mch_max, cmp_all, ahd_max),
            verbose,
            src_bkt,
            mch_max,
            mch_min: if mch_min > mch_max {
                mch_max - 1
            } else {
                mch_min
            },
            ahd_max: if ahd_max < 1024 { 1024 } else { ahd_max },
            cmp_all,
            src_scn: i32::from(src_scn),
            sst: SearchState {
                az_org: 0,
                az_new: 0,
                hsh_org: 0,
                hsh_new: 0,
                prv_org: 0,
                val_new: ByteOrEof::Byte(0),
                prv_new: 0,
                eql_org: 0,
                eql_new: 0,
                rlb: 0,
                hsh_err: 0,
            },
        }
    }

    /// Hashtable accessor for the post-run statistics (`getHsh`,
    /// `JDiff.h:201`).
    pub fn hash(&self) -> &JHashPos {
        &self.hsh
    }

    /// Number of false hash hits (`getHshErr`, `JDiff.h:203`): incremented
    /// per "solution" that pointed nowhere, in release builds too (spec
    /// §18.E). The C++ member's baseline is uninitialized garbage; this port
    /// starts at 0.
    pub fn hsh_err(&self) -> i32 {
        self.sst.hsh_err
    }

    /// Number of repaired hash hits (`getHshRpr`, `JMatchTable.h:100` /
    /// `JMatchTable.cpp:930-932`): matches repaired by comparing. Instance
    /// counter since 0.8.5 — the 0.8.1 global static is retired (spec
    /// §18.E); the verbose statistics read it through here.
    pub fn hsh_rpr(&self) -> i32 {
        self.mch.get_hsh_rpr()
    }

    /// Output statistics snapshot (`lpOut->gzOutByt*`): the C++ `main` reads
    /// these fields on its own output pointer after `jdiff()` returns
    /// (`main.cpp:546-567,610`); here the engine owns the writer, so the
    /// statistics are served through this accessor instead.
    pub fn out_stats(&self) -> OutStats {
        self.out.stats()
    }

    /// Out-of-order access count of the original-file reader
    /// (`lpFilOrg->seekcount()`, read by the CLI at `main.cpp:557`).
    pub fn org_seekcount(&self) -> i64 {
        self.org.seekcount()
    }

    /// Out-of-order access count of the new-file reader
    /// (`lpFilNew->seekcount()`, read by the CLI at `main.cpp:557`).
    pub fn new_seekcount(&self) -> i64 {
        self.r#new.seekcount()
    }

    /// Incremental source scan (`JDiff.cpp:185-188`, repeated verbatim at
    /// `:205-208` and `:212-215`): hash the compare-loop byte into the
    /// original stream's rolling key and add the sample to the hashtable.
    /// Only reached while `miSrcScn == 0 && lzPosOrg == mzAhdOrg`.
    fn hash_add_org(&mut self, lc_org: i32) {
        self.sst.hsh_org = hash_key(
            self.sst.hsh_org,
            &mut self.sst.prv_org,
            lc_org,
            &mut self.sst.eql_org,
        );
        self.hsh
            .add(self.sst.hsh_org, self.sst.az_org, self.sst.eql_org);
        self.sst.az_org += 1;
    }

    /// The equal-run fast loop (`JDiff.cpp:201-224`): counts and consumes
    /// equal bytes up to the small-lap limit. `index_src` is the
    /// `src_scn == 0` mode's incremental source indexing — the only
    /// difference between the C++'s two loops.
    ///
    /// The C condition `*valOrg == *valNew && *valNew >= 0 && *posNew <
    /// lapSml` (all sub-expressions side-effect free) continues exactly
    /// while both reads are equal data bytes and the new-file position
    /// stays within the lap; EOF, `EOB` and the error sentinels end it.
    fn scan_equal_run(
        &mut self,
        pos_org: &mut i64,
        val_org: &mut ByteOrEof,
        pos_new: &mut i64,
        val_new: &mut ByteOrEof,
        lap_sml: i64,
        index_src: bool,
    ) -> i64 {
        let mut cnt: i64 = 0;
        while let (ByteOrEof::Byte(lc_o), ByteOrEof::Byte(lc_n)) = (&*val_org, &*val_new) {
            if lc_o != lc_n || *pos_new >= lap_sml {
                break;
            }
            cnt += 1;
            if index_src && *pos_org == self.sst.az_org {
                self.hash_add_org(i32::from(*lc_o));
            }
            *pos_org += 1;
            *val_org = self.org.get(*pos_org, ReadType::Read);
            *pos_new += 1;
            *val_new = self.r#new.get(*pos_new, ReadType::Read);
        }
        cnt
    }

    /// Difference function (`JDiff::jdiff`, `JDiff.cpp:150-335`): compares
    /// both files byte by byte and writes the differences to the output
    /// handler.
    ///
    /// Returns `Ok(())` on success or the read-error `JDiffError`, which
    /// reaches the return statement either through the **live** `liFnd`
    /// check after `search` (`:277-279`) or through the final EOB check
    /// (`:330-332`).
    pub fn jdiff(&mut self) -> Result<(), JDiffError> {
        let mut lc_org: ByteOrEof; /* byte from original file */
        let mut lc_new: ByteOrEof; /* byte from new file */
        let mut lz_pos_org: i64 = 0;
        let mut lz_pos_new: i64 = 0;

        let mut lb_eql = false; /* accumulate equal bytes? */
        let mut lz_eql: i64 = 0; /* accumulated equal bytes */

        let mut li_fnd: i32 = 0; /* offsets are pointing to a valid solution (= equal regions)? */
        let mut lz_ahd: i64 = 0; /* number of bytes to advance on both files to reach the solution */
        let mut lz_skp_org: i64 = 0; /* number of bytes to skip on original file to reach the solution */
        let mut lz_skp_new: i64 = 0; /* number of bytes to skip on new file to reach the solution */
        /* lap for reducing the number of progress messages for -vv */
        let mut lz_lap_sml: i64 = MAX_OFF_T;

        if self.verbose > 0 {
            dbg_print(format_args!("Comparing : ...           "));
            if self.verbose > 1 {
                lz_lap_sml = PGSMRK;
            }
        }

        /* Take one byte from each file ... (JDiff.cpp:174-175) */
        lc_org = self.org.get(lz_pos_org, ReadType::Read);
        lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
        /* while (lcNew >= 0) (JDiff.cpp:176): the comparison loop runs on
         * data bytes only — EOF, EOB (never returned by a plain read) and
         * the error sentinels are all negative. */
        while let ByteOrEof::Byte(lc_new_val) = &lc_new {
            /* Debug: input trace (JDiff.cpp:178-182). The C++ prints
             * `lzPosOrg - 1`, so the very first line reports position -1. */
            #[cfg(feature = "debug")]
            if dbg(DBGPRG) {
                // boundary: the trace formats the raw i32 channel values
                // (`%2x`), including EOF/error sentinels on lc_org.
                dbg_print(format_args!(
                    "Input {}->{:2x} {}->{:2x}.\n",
                    crate::defs::p8(lz_pos_org - 1),
                    lc_org.to_i32() as u32,
                    crate::defs::p8(lz_pos_new - 1),
                    u32::from(*lc_new_val),
                ));
            }

            /* Incremental source scan (JDiff.cpp:184-189) */
            if self.src_scn == 0 && lz_pos_org == self.sst.az_org {
                // boundary: the rolling hash computes with the raw i32
                // channel value (EOF included, like the C `ufHshAdd(lcOrg)`).
                self.hash_add_org(lc_org.to_i32());
            }

            /* Compare and process... (JDiff.cpp:192): lcNew is a data byte
             * here, so the equality is a byte comparison; lcOrg may still
             * be a non-byte when the new file runs past the original. */
            match &lc_org {
                /* Output or count equals (JDiff.cpp:193-224) */
                ByteOrEof::Byte(lc_org_val) if *lc_org_val == *lc_new_val => {
                    if !lb_eql {
                        // the first bytes may be kept in reserve, then switch to
                        // counting asap
                        lb_eql = self.out.put(
                            Op::Eql,
                            1,
                            i32::from(*lc_org_val),
                            i32::from(*lc_new_val),
                            lz_pos_org,
                            lz_pos_new,
                        );
                        lz_ahd -= 1; // decrease ahead counter

                        lz_pos_org += 1;
                        lc_org = self.org.get(lz_pos_org, ReadType::Read);
                        lz_pos_new += 1;
                        lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
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
                }
                /* Output accumulated equals (JDiff.cpp:227) */
                _ if lz_ahd > 0 => {
                    self.flush_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);

                    /* Output difference (JDiff.cpp:229-245): `lcOrg < 0`
                     * here means "not a data byte" — EOF (original file
                     * exhausted) or an error sentinel. */
                    if !matches!(&lc_org, ByteOrEof::Byte(_)) {
                        self.out.put(
                            Op::Ins,
                            1,
                            lc_org.to_i32(),
                            i32::from(*lc_new_val),
                            lz_pos_org,
                            lz_pos_new,
                        );
                        lz_ahd -= 1; // decrease ahead counter

                        /* Take next byte from destination file ... */
                        lz_pos_new += 1;
                        lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                    } else {
                        /* `lcOrg != lcNew && lcOrg >= 0 && lcNew >= 0 &&
                         * lzAhd > 0`: continue exactly on two unequal data
                         * bytes while ahead budget remains. */
                        while let (ByteOrEof::Byte(lc_o), ByteOrEof::Byte(lc_n)) =
                            (&lc_org, &lc_new)
                        {
                            if lc_o == lc_n || lz_ahd <= 0 {
                                break;
                            }
                            self.out.put(
                                Op::Mod,
                                1,
                                i32::from(*lc_o),
                                i32::from(*lc_n),
                                lz_pos_org,
                                lz_pos_new,
                            );
                            lz_ahd -= 1; // decrease ahead counter

                            /* Take next byte from each file ... */
                            lz_pos_org += 1;
                            lc_org = self.org.get(lz_pos_org, ReadType::Read);
                            lz_pos_new += 1;
                            lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                        }
                    }
                }
                /* Oops: the "found" solution did not point to an equal region
                 * (JDiff.cpp:247-261). This may happen, especially for
                 * non-compared solutions, but we should hope this does not
                 * happen too much. */
                _ if li_fnd == 1 && lz_ahd == 0 => {
                    li_fnd = 0;

                    /* Report the miss to the user: counted in release builds too
                     * (wrapping i32, spec §21.6). */
                    self.sst.hsh_err = self.sst.hsh_err.wrapping_add(1);
                    if self.verbose > 2 && self.cmp_all {
                        dbg_print(format_args!(
                            "\nInaccurate solution at positions {}/{}!\n",
                            lz_pos_org, lz_pos_new
                        ));
                        dbg_print(format_args!("Comparing : ...           "));
                    }

                    /* v083x: advance depending on hashtable overloading */
                    lz_ahd = i64::from(self.hsh.reliability() / 2);
                }
                /* Look for a new solution (JDiff.cpp:263-305) */
                _ => {
                    /* Output accumulated equals (JDiff.cpp:266-267) */
                    self.flush_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);

                    /* Flush output buffer in debug (JDiff.cpp:269-274) */
                    #[cfg(feature = "debug")]
                    if dbg(DBGAHD) || dbg(DBGMCH) {
                        self.out.put(Op::Esc, 0, 0, 0, lz_pos_org, lz_pos_new);
                    }

                    /* Find a new equals-region (JDiff.cpp:276-279): the int
                     * return is LIVE at 0.8.5 — a negative error aborts the
                     * diff (the 0.8.1 bool collapse is fixed, spec §18.E). */
                    li_fnd = self.search(
                        lz_pos_org,
                        lz_pos_new,
                        &mut lz_skp_org,
                        &mut lz_skp_new,
                        &mut lz_ahd,
                    )?;

                    /* Debug: find-ahead result and progress traces
                     * (JDiff.cpp:280-286). */
                    #[cfg(feature = "debug")]
                    {
                        if dbg(DBGAHD) {
                            dbg_print(format_args!(
                                "Findahead on {} {} skip {} {} ahead {}\n",
                                lz_pos_org, lz_pos_new, lz_skp_org, lz_skp_new, lz_ahd,
                            ));
                        }
                        if dbg(DBGPRG) {
                            dbg_print(format_args!(
                                "Current position in new file= {}\n",
                                lz_pos_new
                            ));
                        }
                    }

                    /* Execute offsets (JDiff.cpp:288-305) */
                    if lz_skp_org > 0 {
                        self.out
                            .put(Op::Del, lz_skp_org, 0, 0, lz_pos_org, lz_pos_new);
                        lz_pos_org += lz_skp_org;
                        lc_org = self.org.get(lz_pos_org, ReadType::Read);
                    } else if lz_skp_org < 0 {
                        self.out
                            .put(Op::Bkt, -lz_skp_org, 0, 0, lz_pos_org, lz_pos_new);
                        lz_pos_org += lz_skp_org;
                        lc_org = self.org.get(lz_pos_org, ReadType::Read);
                    }
                    if lz_skp_new > 0 {
                        /* `lcNew > EOF` (JDiff.cpp:300): data bytes only —
                         * EOF and the error sentinels end the insertion. */
                        while lz_skp_new > 0 {
                            let ByteOrEof::Byte(lc_n) = &lc_new else {
                                break;
                            };
                            self.out
                                .put(Op::Ins, 1, 0, i32::from(*lc_n), lz_pos_org, lz_pos_new);
                            lz_skp_new -= 1;
                            lz_pos_new += 1;
                            lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                        }
                    }
                } /* if lcOrg == lcNew */
            }

            /* show progress (JDiff.cpp:308-312) */
            if self.verbose > 1 && lz_lap_sml <= lz_pos_new {
                dbg_print(format_args!("\rComparing : {:>12}Mb", lz_pos_new / PGSMRK));
                lz_lap_sml = lz_pos_new + PGSMRK;
            }
        } /* while lcNew >= 0 */

        /* Flush output buffer (JDiff.cpp:315-317) */
        self.flush_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);
        self.out.put(Op::Esc, 0, 0, 0, lz_pos_org, lz_pos_new);

        /* Show progress (JDiff.cpp:319-322) */
        if self.verbose > 0 {
            dbg_print(format_args!(
                "\rComparing : {:>12}Mb",
                (lz_pos_new + PGSMRK / 2) / PGSMRK
            ));
        }

        /* Show final hashtable distribution in case of incremental source
         * scanning (JDiff.cpp:324-327) */
        if self.verbose > 2 && self.src_scn == 0 {
            self.hsh.dist(lz_pos_org, 10);
        }

        /* Return code (JDiff.cpp:329-334): `lcNew < EOB || lcOrg < EOB`
         * holds exactly for the error sentinels (EOF and EOB are not
         * `< EOB`); the lower of the two channel values is the error code.
         * Both locals are owned here and dead after this match, so the
         * error itself is moved out — the SAME error the reader produced,
         * not a re-wrapping. */
        match (lc_new, lc_org) {
            (ByteOrEof::Err(e), ByteOrEof::Err(f)) => {
                if e.exit_code() <= f.exit_code() {
                    Err(e)
                } else {
                    Err(f)
                }
            }
            (ByteOrEof::Err(e), _) | (_, ByteOrEof::Err(e)) => Err(e),
            _ => Ok(()),
        }
    } /* jdiff */

    /// Flush pending output (`JDiff::flushEql`, `JDiff.cpp:340-347`).
    ///
    /// `lz_eql`/`lb_eql` are the accumulation state of [`JDiff::jdiff`], which
    /// the C++ passes by reference (`off_t &lzEql, bool &lbEql`).
    fn flush_eql(&mut self, pos_org: i64, pos_new: i64, lz_eql: &mut i64, lb_eql: &mut bool) {
        /* Output accumulated equals (JDiff.cpp:342-345) */
        if *lz_eql > 0 {
            self.out
                .put(Op::Eql, *lz_eql, 0, 0, pos_org - *lz_eql, pos_new - *lz_eql);
            *lz_eql = 0;
        }
        *lb_eql = false;
    } /* flushEql */

    /// Find Ahead function (`JDiff::search`, `JDiff.cpp:389-718`): reads
    /// ahead on both files until an equal series of 32-byte samples is found
    /// and calculates the displacement vector between the files — positive
    /// if characters need to be inserted in the original file, negative if
    /// they need to be removed from it.
    ///
    /// Returns `Ok(0)` = no solution found, `Ok(1)` = solution found, or
    /// the `JDiffError` (propagated live by [`JDiff::jdiff`]). The 0/1
    /// payload stays: jdiff's "inaccurate solution" arm consumes it.
    fn search(
        &mut self,
        red_org: i64,
        red_new: i64,
        skp_org: &mut i64,
        skp_new: &mut i64,
        ahd: &mut i64,
    ) -> Result<i32, JDiffError> {
        let mut lz_fnd_org: i64 = 0; /* Found position within original file;
         * the C++ also declares lzFndNew (out
         * parameter of gpMch->getbest), which the
         * Option return replaces here. */
        let mut lz_lap: i64 = 0; /* Stop-lap for progress counter */

        let mut li_max: i32; /* Max number of bytes to look ahead */
        let mut li_bck: i32; /* Number of bytes to look back */

        /* Set Lap for progress counter (JDiff.cpp:404-405) */
        if self.verbose > 1 {
            lz_lap = red_new + PGSMRK;
        }

        /* Prescan the source file to build the hashtable
         * (switch miSrcScn, JDiff.cpp:407-448) */
        if self.src_scn == 1 {
            /* do a full prescan */
            self.build_full_index()?;
            self.src_scn = 2;
            self.sst.rlb = self.hsh.reliability();
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
                    (i64::from(self.ahd_max) - self.sst.az_org) as i32
                } else {
                    (i64::from(self.ahd_max) / 2 - (self.sst.az_org - red_org)) as i32
                }
            };

            // scan ahead till EOB or EOF (JDiff.cpp:458): `lcOrg <= EOF`
            // catches EOF, EOB (soft look-ahead past the window) and the
            // error sentinels — everything but a data byte.
            while li_scan > 0 {
                let lc_org = match self.org.get(self.sst.az_org, ReadType::SoftAhead) {
                    ByteOrEof::Byte(lc_org) => lc_org,
                    _ => break,
                };
                self.sst.hsh_org = hash_key(
                    self.sst.hsh_org,
                    &mut self.sst.prv_org,
                    i32::from(lc_org),
                    &mut self.sst.eql_org,
                );
                self.hsh
                    .add(self.sst.hsh_org, self.sst.az_org, self.sst.eql_org);
                self.sst.az_org += 1;
                li_scan -= 1;
            }
            self.sst.rlb = self.hsh.reliability();
        } /* switch scan source file - build hashtable */

        /*
         * How many bytes to look ahead (search) ? (JDiff.cpp:450-470)
         * As far as possible, but going too far makes no sense: the
         * unreliability range is only an estimate of the average number of
         * bytes needed to find a solution, so using the whole buffer may
         * solve a situation where a solution needs more bytes to be found
         * than indicated by the reliability range. Once a minimum number of
         * potential solutions is found, the lookahead may again be reduced
         * to the reliability range (see below).
         */
        li_max = if self.sst.az_new > red_new {
            // C++: int assignment of the off_t difference.
            (i64::from(self.ahd_max) - (self.sst.az_new - red_new)) as i32
        } else {
            self.ahd_max
        };

        if li_max < self.sst.rlb {
            li_max = self.sst.rlb; // search at least the reliability distance
        }

        /*
         * How many bytes to look back ? (JDiff.cpp:472-487)
         * In theory: none, it makes no sense to look back.
         * In practice:
         * - looking back avoids the need to reinitialize the hash function
         * - re-initialization of the hash function can take up to SMPSZE * 2 bytes
         * - looking back allows to keep the existing match-table up-to-date
         * Therefore, we allow for some look back.
         */
        li_bck = (red_new - self.sst.az_new) as i32; // C++ int assignment
        if li_bck < 0 {
            // mzAhdNew is stil ahead of azRedNew from a previous lookahead
            // continue where the previous left off
            li_bck = 0;
        } else if li_bck > self.sst.rlb + 2 * SMPSZE - 1 {
            li_bck = self.sst.rlb + 2 * SMPSZE - 1; // 2 * SMPSZE to anticipate a reinitialization
        }

        /* Do not backtrace before lzBseOrg (JDiff.cpp:490-491) */
        let lz_bse_org: i64 = if self.src_bkt {
            0
        } else {
            self.org.get_buf_pos()
        };

        /* Cleanup the old matches (JDiff.cpp:493-508): Full means the table
         * has no reusable element; Error lands there too. Best/Good mean a
         * good match is already available and shorten the lookahead. */
        let mut li_fnd: i32 = 0; /* Number of matches found */
        match self.mch.cleanup(
            lz_bse_org,
            red_new,
            self.sst.rlb,
            &mut *self.org,
            &mut *self.r#new,
        ) {
            MchRet::Error | MchRet::Full => {
                li_fnd = self.mch_max; // table is full
            }
            // a good match is already available : reduce search (but not to
            // zero); the guard is the C++ `if (liMax > miRlb * 2)` — when it
            // fails the arm does nothing, like the C++ break.
            MchRet::Best | MchRet::Good if li_max > self.sst.rlb * 2 => {
                li_max = self.sst.rlb * 2;
            }
            _ => {}
        }

        /* If there's room to work (JDiff.cpp:510-647) */
        if li_fnd < self.mch_max {
            // Set lookahead base position
            self.r#new.set_lookahead_base(red_new);

            // Switch to soft reading if the minimum number of matches is obtained
            let mut li_sft_new = if li_fnd >= self.mch_min {
                ReadType::SoftAhead
            } else {
                ReadType::HardAhead
            };

            /*
             * Re-Initialize hash function (read 31 or 63 bytes) if
             * - ahead position has been reset, or
             * - read position has jumped over the ahead position
             * (JDiff.cpp:518-574)
             */
            if self.sst.az_new == 0 || self.sst.az_new + i64::from(li_bck) < red_new {
                // Don't go back more than the buffer allows (to avoid EOB)
                self.sst.az_new = self.r#new.get_buf_pos();

                // Set looking back position, but never before the buffer
                if red_new > self.sst.az_new + i64::from(li_bck) {
                    self.sst.az_new = red_new - i64::from(li_bck);
                    if self.sst.az_new < 0 {
                        self.sst.az_new = 0;
                    }
                }

                // Initialize hash: at the start of the file (mzAhdNew == 0),
                // SMPSZE suffices to initialize, but within the file
                // (mzAhdNew > 0), in a worst case, we first need SMPSZE to
                // initialize miEqlNew and then another SMPSZE to initialize
                // the hash
                if self.sst.az_new == 0 {
                    li_bck = SMPSZE - 1; // to initialize mkHsh (miEql=0 is correct)
                } else {
                    li_bck = SMPSZE * 2 - 1; // to initialize mkHsh and miEql
                }
                self.sst.az_new -= 1; // switch to pre-increments
                self.sst.hsh_new = 0;
                self.sst.eql_new = 0;
                self.sst.prv_new = EOF;
                let mut li_idx: i32 = 0;
                while li_idx < li_bck {
                    self.sst.val_new = self.r#new.get(self.sst.az_new + 1, li_sft_new); // ++mzAhdNew
                    self.sst.az_new += 1;
                    /* `miVal <= EOF` (JDiff.cpp:531-534): EOF, EOB (soft
                     * look-ahead past the window) and the error sentinels
                     * end the initialization with the position rolled back;
                     * the raw sentinel stays in `val_new` for the error
                     * check after the scan. */
                    let ByteOrEof::Byte(lc_val) = &self.sst.val_new else {
                        self.sst.az_new -= 1;
                        break;
                    };
                    self.sst.hsh_new = hash_key(
                        self.sst.hsh_new,
                        &mut self.sst.prv_new,
                        i32::from(*lc_val),
                        &mut self.sst.eql_new,
                    );

                    // The following line needs some explication.
                    // The goal of this line is to terminate the initialization ASAP.
                    // To explain, consider SMPSZE == 8, then we need 7 valid miEql's to initialize.
                    // For example, consider an initialization starting at position 4 (hex data)
                    //    mzAhd :   4 5 6 7 8 9 A B C D E F ...
                    //    miVal :   0 0 0 0 7 6 5 4 3 2 1 0 4 9 7 4  ...
                    //    miPrv : EOF 0 0 0 0 7 6 5 4 3 2 1 0 4 9 7 4 ...
                    //    miEql :   0 1 2 3 0 0 0 0 0 0 0 ...
                    //    liIdx :   0 1 2 3 4 5 6 7 8 9 A B C ...
                    //    init            +-----------+
                    // We don't know if position 3 is 0 or not, so we don't know what value miEql
                    // at position 4 should have. So the first four bytes cannot be used,
                    // because miEql may not be correct.
                    // As soon as miEql is reset to 0 by miPrv != miVal, miEql becomes correct
                    // and initialization will be ok after SMPSZE-1 bytes (position D in the example)
                    // Reset can be detected by miEql != liIdx. Hence, when miEql != liIdx,
                    // we can reduce liMax to liIdx + SMPSZE - 1.
                    if li_idx != self.sst.eql_new && li_bck > li_idx + (SMPSZE - 1) {
                        li_bck = li_idx + (SMPSZE - 1);
                    }
                    li_idx += 1;
                }
            }

            /* Add the resulting look-back to liMax (JDiff.cpp:576-578) */
            if self.sst.az_new < red_new {
                li_max += (red_new - self.sst.az_new) as i32; // C++ int assignment
            }

            /*
             * Build the table of matches (JDiff.cpp:580-646)
             */
            while li_max > 0 {
                /* hash the new value */
                self.sst.val_new = self.r#new.get(self.sst.az_new + 1, li_sft_new); // ++mzAhdNew
                self.sst.az_new += 1;
                /* `miVal <= EOF` (JDiff.cpp:584-587): EOF, EOB (soft
                 * look-ahead past the window) and the error sentinels end
                 * the scan with the position rolled back; the raw sentinel
                 * stays in `val_new` for the error check after the loop. */
                let ByteOrEof::Byte(lc_val) = &self.sst.val_new else {
                    self.sst.az_new -= 1;
                    break;
                };
                self.sst.hsh_new = hash_key(
                    self.sst.hsh_new,
                    &mut self.sst.prv_new,
                    i32::from(*lc_val),
                    &mut self.sst.eql_new,
                );
                li_max -= 1;

                /* lookup the new value in the hashtable and add it to the
                 * table of matches... (JDiff.cpp:594) */
                if self.hsh.get(self.sst.hsh_new, &mut lz_fnd_org) {
                    /* ...unless it's not usable because we've been instructed
                     * not to backtrack on source file (JDiff.cpp:596) */
                    if lz_fnd_org > lz_bse_org {
                        /* it's usable: add to the table of matches; the
                         * taxonomy switch (JDiff.cpp:598-637): Error falls
                         * through into Full ("no break"), Good/Best reduce
                         * the lookahead and fall through into Valid, which
                         * counts the match. */
                        match self.mch.add(
                            lz_fnd_org,
                            self.sst.az_new,
                            red_new,
                            &mut *self.org,
                            &mut *self.r#new,
                        ) {
                            MchRet::Error => {
                                // Table in an unexpectedly full state
                                #[cfg(feature = "debug")]
                                dbg_print(format_args!(
                                    "Matchtable overflow at {}\n",
                                    crate::defs::p8(self.sst.az_new)
                                ));
                                // no break: continue with next case
                                li_max = 0;
                                continue;
                            }
                            MchRet::Full => {
                                // Table is full
                                li_max = 0;
                                continue;
                            }
                            MchRet::Enlarged | MchRet::Invalid => {
                                // Existing solution enlarged / match invalid:
                                // do nothing
                            }
                            MchRet::Good | MchRet::Best => {
                                // This seems to be a very good solution.
                                // However, due to the unreliable nature of the
                                // checksums and the hash-table, the first good
                                // solution is not always the best one, but a
                                // better one should be found within the
                                // reliability range.
                                //
                                // Why ? Because the reliability range estimates
                                // the number of bytes to search before finding
                                // all solutions hidden behind the
                                // unreliability. So after the (estimated)
                                // reliability range, no better solution should
                                // be found anymore. Reduce the lookahead to be
                                // sure and to improve performance.
                                if li_max > self.sst.rlb {
                                    li_max = self.sst.rlb;
                                }
                                // no break: continue with next case
                                li_fnd += 1;
                                if self.sst.az_new > red_new {
                                    if li_fnd >= self.mch_min {
                                        li_sft_new = ReadType::SoftAhead; // switch to soft reading
                                    }
                                    if li_fnd >= self.mch_max {
                                        li_max = 0; // stop lookahead
                                        continue;
                                    }
                                }
                            }
                            MchRet::Valid => {
                                // solution added
                                li_fnd += 1;
                                if self.sst.az_new > red_new {
                                    if li_fnd >= self.mch_min {
                                        li_sft_new = ReadType::SoftAhead; // switch to soft reading
                                    }
                                    if li_fnd >= self.mch_max {
                                        li_max = 0; // stop lookahead
                                        continue;
                                    }
                                }
                            }
                        }
                    } /* if usable */
                } /* lookup */

                /* show progress (JDiff.cpp:641-645) */
                if self.verbose > 1 && lz_lap <= self.sst.az_new {
                    dbg_print(format_args!(
                        "+{:<12}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}",
                        (self.sst.az_new - red_new) / PGSMRK
                    ));
                    lz_lap += PGSMRK;
                }
            } /* while ! EOF */
        } /* if liFnd <= miMchMax */

        /* Check for errors (JDiff.cpp:649-652): `miValNew < EOB` holds
         * exactly for the error sentinels — EOF and EOB are not `< EOB`
         * and end the scan normally. The error is moved out of the state
         * (the field is never read again on this path: a failed search
         * aborts jdiff immediately); the SAME error the reader produced
         * propagates, not a re-wrapping. */
        if let ByteOrEof::Err(e) = std::mem::replace(&mut self.sst.val_new, ByteOrEof::Eof) {
            return Err(e);
        }

        /* show progress (JDiff.cpp:654-657) */
        if self.verbose > 1 && lz_lap > red_new + PGSMRK {
            dbg_print(format_args!(
                "+{:<12}...\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}",
                (self.sst.az_new - red_new) / PGSMRK
            ));
        }

        /*
         * Get the best match and calculate the offsets (JDiff.cpp:659-717).
         * 0.8.5: the table tracked the best incrementally during add/cleanup;
         * getbest only re-evaluates enlarged EOB elements (when !cmpAll) and
         * returns it — no files, no hashtable, no rescanning.
         */
        let lb_fnd = self.mch.getbest(red_org, red_new);

        /* clear search progress (JDiff.cpp:664-668) */
        if self.verbose > 1 && lz_lap > red_new + PGSMRK {
            dbg_print(format_args!(
                "                \u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}"
            ));
        }

        /* Calculate the resulting offsets (JDiff.cpp:670-717) */
        match lb_fnd {
            None => {
                // No solution has been found. Maybe the search window size is
                // too small, or the hashtable, or the buffers, or maybe the
                // files are simply different. Anyway, iterating over the same
                // search windows makes no sense, so jump forward for at least
                // SMPSZE bytes.
                *skp_org = 0;
                *skp_new = 0;
                *ahd = self.sst.az_new - red_new;
                if *ahd < i64::from(SMPSZE) {
                    #[cfg(feature = "debug")]
                    if dbg(DBGAHD) {
                        dbg_print(format_args!("\nForcing skip of SMPSZE bytes\n"));
                    }
                    *ahd = i64::from(SMPSZE);
                }
                Ok(0)
            }
            Some((lz_fnd_org, lz_fnd_new)) => {
                if lz_fnd_org >= red_org {
                    if lz_fnd_org - red_org >= lz_fnd_new - red_new {
                        /* go forward on original file (JDiff.cpp:691-694) */
                        *skp_org = lz_fnd_org - red_org + red_new - lz_fnd_new;
                        *skp_new = 0;
                        *ahd = lz_fnd_new - red_new;
                    } else {
                        /* go forward on new file (JDiff.cpp:696-700) */
                        *skp_org = 0;
                        *skp_new = lz_fnd_new - red_new + red_org - lz_fnd_org;
                        *ahd = lz_fnd_org - red_org;
                    }
                } else {
                    /* backtrack on original file (JDiff.cpp:702-713) */
                    *skp_org = red_org - lz_fnd_org + lz_fnd_new - red_new;
                    if *skp_org <= red_org - lz_bse_org {
                        *skp_new = 0;
                        *skp_org = -*skp_org;
                        *ahd = lz_fnd_new - red_new;
                    } else {
                        /* do not backtrace before beginning of file */
                        *skp_new = *skp_org - (red_org - lz_bse_org);
                        *skp_org = lz_bse_org - red_org;
                        *ahd = (lz_fnd_new - red_new) - *skp_new;
                    }
                    /* 0.8.5: mzAhdOrg is NOT reset on backtrack anymore. */
                }

                Ok(1)
            }
        }
    } /* search */

    /// Build the full source index (`JDiff::buildFullIndex`,
    /// `JDiff.cpp:726-793`): calculates a hash-key for every 32-byte sample
    /// in the source file and stores them with their position in the
    /// hashtable. Serial port of the 0.8.1 OpenMP block (the pragma was only
    /// active in the never-used `make parallel` target and is gone at 0.8.5).
    fn build_full_index(&mut self) -> Result<(), JDiffError> {
        let Self {
            org, hsh, verbose, ..
        } = self;

        let mut lk_hsh_org: u32 = 0; // Current hash value for original file
        let mut li_eql_org: i32 = 0; // Number of times current value occurs in hash value
        let mut lc_val_org = ByteOrEof::Byte(0); // Current  file value (C: int zero-init)
        let mut lc_val_prv: i32 = EOF; // Previous file value
        let mut lz_pos_org: i64 = -1; // Position within original file

        let mut li_idx: i32;

        if *verbose > 0 {
            dbg_print(format_args!("\nIndexing  : ...           "));
        }

        /* Read SMPSZE-1 bytes (31 or 63) to initialize the hash function
         * (JDiff.cpp:740-746) */
        li_idx = 0;
        while li_idx < SMPSZE - 1 {
            lz_pos_org += 1; // ++lzPosOrg
            lc_val_org = org.get(lz_pos_org, ReadType::HardAhead);
            /* `lcValOrg <= EOF` (JDiff.cpp:743): EOF and the error
             * sentinels end the hash initialization; the raw sentinel
             * stays in `lc_val_org` for the return check below. */
            let ByteOrEof::Byte(lc_val) = &lc_val_org else {
                break;
            };
            lk_hsh_org = hash_key(
                lk_hsh_org,
                &mut lc_val_prv,
                i32::from(*lc_val),
                &mut li_eql_org,
            );
            li_idx += 1;
        }

        /* Build hashtable (JDiff.cpp:748-778): one loop; the user-feedback
         * extras run only under verbose>1 (the C++ slow version). */
        let feedback = *verbose > 1;
        while let ByteOrEof::Byte(_) = &lc_val_org {
            // lcValOrg > EOF
            lz_pos_org += 1;
            lc_val_org = org.get(lz_pos_org, ReadType::HardAhead);
            /* `lcValOrg <= EOF` (JDiff.cpp:751): EOF and the error
             * sentinels end the prescan. */
            let ByteOrEof::Byte(lc_val) = &lc_val_org else {
                break;
            };
            lk_hsh_org = hash_key(
                lk_hsh_org,
                &mut lc_val_prv,
                i32::from(*lc_val),
                &mut li_eql_org,
            );
            hsh.add(lk_hsh_org, lz_pos_org, li_eql_org);

            /* Debug: hash trace (JDiff.cpp:758-762); the trailing field
             * is `%8d` of the literal 0 here, not a P8zd position. */
            #[cfg(feature = "debug")]
            if feedback && dbg(DBGAHH) {
                dbg_print(format_args!(
                    "ufHshAdd({:2x} -> {:8x}, {}, {:8})\n",
                    u32::from(*lc_val),
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

        if *verbose > 0 {
            /* output final position (JDiff.cpp:780-784) */
            dbg_print(format_args!(
                "\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}{:>12}Mb\n",
                lz_pos_org / PGSMRK
            ));
            dbg_print(format_args!("Comparing : ...           "));
        }
        if *verbose > 2 {
            hsh.dist(lz_pos_org, 10);
        }

        /* (JDiff.cpp:789-792): `lcValOrg < EOB` holds exactly for the
         * error sentinels — EOF and EOB end the prescan normally. The
         * owned local moves the SAME error the reader produced out. */
        match lc_val_org {
            ByteOrEof::Err(e) => Err(e),
            _ => Ok(()),
        }
    } /* buildFullIndex */
}

/// The hash function (`JDiff::hash`, `JDiff.cpp:361-371`): generate a new
/// hash value by adding a new byte. Old bytes are shifted out from the hash
/// value in such a way that the new value corresponds to a sample of 32 bytes
/// (the lowest bit of the 32'th byte still influences the highest bit of the
/// hash value).
///
/// 0.8.5 moved this function from `JHashPos` into `JDiff` and added the
/// equal-run counter into the value — this alone changes match decisions (and
/// thus patch bytes) vs 0.8.1 (spec §18.E):
///
/// * `old == new`: `eql` increments while it is below `SMPSZE` (capped at 32);
/// * otherwise the caller's `old` tracker becomes `new` (`acOld = acNew`,
///   `JDiff.cpp:367` — the C++ takes `int &acOld` in-out, mirrored here with
///   `&mut`) and `eql` resets to 0;
/// * the result is `(cur*2) + new + eql` ("multiplication by 2 is faster
///   than `<< 2`", C++ comment). The u32 arithmetic wraps exactly like the
///   32-bit C++ `hkey` of the oracle build.
pub fn hash_key(cur: u32, old: &mut i32, r#new: i32, eql: &mut i32) -> u32 {
    if *old == r#new {
        if *eql < SMPSZE {
            *eql += 1;
        }
    } else {
        *old = r#new;
        if *eql != 0 {
            // improves performance
            *eql = 0;
        }
    }
    cur.wrapping_mul(2)
        .wrapping_add(r#new as u32)
        .wrapping_add(*eql as u32) // multiplication by 2 is faster than << 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{EXI_RED, EXI_SEK, MINEQL};
    use crate::jfile::JFileMem;
    use crate::jout::{JOutBin, OutStats};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// `hash_key` (`JDiff::hash`, `JDiff.cpp:361-371`): the equal-run counter
    /// is added into the hash value, so the key diverges from the 0.8.1 pure
    /// `*2 + byte` as soon as a second equal byte enters the run (spec
    /// §18.E). The stream "aaaa" from key 0: the first byte differs from the
    /// initial `old` (-1) and contributes no eql term; from the second byte
    /// on, eql counts up and is added. `old` is in-out like the C++
    /// `int &acOld`.
    #[test]
    fn hash_key_adds_eql_term() {
        let mut eql = 0i32;
        let mut old = -1i32; // no byte equals -1: the first byte differs
        let mut k = 0u32;
        let mut keys = Vec::new();
        for _ in 0..4 {
            k = hash_key(k, &mut old, i32::from(b'a'), &mut eql);
            keys.push(k);
        }
        assert_eq!(keys, vec![97, 292, 683, 1466]);
        assert_eq!(eql, 3);
        // The 0.8.1 key of the same stream would be 291 at index 1 — the eql
        // term is the 0.8.5 divergence vector.
        assert_ne!(keys[1], 97u32.wrapping_mul(2).wrapping_add(97));
    }

    /// `hash_key` resets eql to 0 on a differing byte (`JDiff.cpp:368-369`)
    /// and updates the caller's `old` tracker (`acOld = acNew`).
    #[test]
    fn hash_key_resets_eql_on_differ() {
        let mut eql = 0i32;
        let mut old = -1i32;
        let mut k = 0u32;
        let mut keys = Vec::new();
        for b in b"aaxa" {
            k = hash_key(k, &mut old, i32::from(*b), &mut eql);
            keys.push(k);
        }
        // 'aa' builds eql 1 (key 292), 'x' resets it: 292*2 + 120 + 0, then
        // 'a' differs from 'x': 704*2 + 97 + 0.
        assert_eq!(keys, vec![97, 292, 704, 1505]);
        assert_eq!(eql, 0);
        assert_eq!(old, i32::from(b'a'), "old tracks the last byte");
    }

    /// `hash_key` caps eql at SMPSZE (`JDiff.cpp:364-365`: only increment
    /// while `eql < SMPSZE`); once capped the eql term stays 32.
    #[test]
    fn hash_key_caps_eql_at_smpsze() {
        let mut eql = 0i32;
        let mut old = -1i32;
        let mut k = 0u32;
        for _ in 0..40 {
            k = hash_key(k, &mut old, i32::from(b'a'), &mut eql);
        }
        assert_eq!(eql, SMPSZE);
        let before = k;
        k = hash_key(k, &mut old, i32::from(b'a'), &mut eql);
        assert_eq!(k, before.wrapping_mul(2).wrapping_add(97 + SMPSZE as u32));
        assert_eq!(eql, SMPSZE);
    }

    /// Deterministic pseudo-random filler byte stream (LCG, bits 8..=15),
    /// identical to the C++ oracle harness used to pin the expected sequences
    /// (`s = s * 1664525 + 1013904223; (s >> 8) & 0xff`).
    fn lcg_bytes(seed: u32, n: usize) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as u8
            })
            .collect()
    }

    /// One recorded operand: (opr, len, org, new).
    type OpLog = Vec<(Op, i64, i32, i32)>;

    /// Shared op log so the test can retrieve the (opr, len, org, new) tuples
    /// after the recorder has been moved into the engine.
    #[derive(Clone, Default)]
    struct Ops(Rc<RefCell<OpLog>>);

    /// Recording [`JOut`] that logs `(opr, len, org, new)` tuples and mimics
    /// the `JOutBin` return contract exactly (`JOutBin.cpp:165-227`): EQL
    /// buffers up to `MINEQL` (2) bytes (returning true once 2 are pending,
    /// thereafter always true); every non-EQL operand flushes (and thereby
    /// resets) the pending EQL bytes and returns false.
    struct RecordingOut {
        ops: Ops,
        eql_cnt: i32,
    }

    impl RecordingOut {
        fn new(ops: Ops) -> Self {
            RecordingOut { ops, eql_cnt: 0 }
        }
    }

    impl JOut for RecordingOut {
        fn put(
            &mut self,
            opr: Op,
            len: i64,
            org: i32,
            new: i32,
            _pos_org: i64,
            _pos_new: i64,
        ) -> bool {
            self.ops.0.borrow_mut().push((opr, len, org, new));
            if opr == Op::Eql {
                if self.eql_cnt < MINEQL {
                    self.eql_cnt += 1;
                    self.eql_cnt >= MINEQL
                } else {
                    true
                }
            } else {
                self.eql_cnt = 0; // JOutBin flushes at every non-EQL operand
                false
            }
        }

        fn stats(&self) -> OutStats {
            OutStats::default()
        }
    }

    /// Always-failing original file: every `get` returns `Err(EXI_RED)` (a
    /// negative exit code at 0.8.5) — `JFile` implementations may report
    /// hard read errors, and the engine aborts with that code (the CLI
    /// prints "Error reading file !", exit 8).
    struct FailingJFile;

    impl JFile for FailingJFile {
        fn get(&mut self, _pos: i64, _typ: ReadType) -> ByteOrEof {
            ByteOrEof::from_raw(EXI_RED)
        }

        fn seekcount(&self) -> i64 {
            0
        }

        fn set_lookahead_base(&mut self, _base: i64) {}

        fn is_sequential(&self) -> bool {
            false
        }

        fn jeofpos(&mut self) -> i64 {
            i64::from(EXI_SEK)
        }
    }

    /// Engine with the CLI default settings (spec §4), hashtable 1 MB —
    /// 87381 elements → prime 87359 (spec §21.18), the smallest table the
    /// 0.8.5 MB ctor can build. For the fixture sizes used here every sample
    /// is stored regardless of the table prime (all-high-quality adds store
    /// while col_max is 4), so behavior is identical to the larger defaults
    /// (verified against the C++ 0.8.5 oracle with sizes 1/2/8/32 MB).
    fn engine<'a>(
        org: Box<dyn JFile + 'a>,
        r#new: Box<dyn JFile + 'a>,
        out: Box<dyn JOut + 'a>,
    ) -> JDiff<'a> {
        JDiff::new(org, r#new, out, 1, 0, true, true, 8, 4, 256 * 1024, true)
    }

    /// Drives the engine over the given file pair and returns (ret, ops).
    fn run(org: Vec<u8>, new: Vec<u8>) -> (Result<(), JDiffError>, OpLog) {
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(JFileMem::new(org)),
            Box::new(JFileMem::new(new)),
            Box::new(rec),
        );
        let ret = jd.jdiff();
        (ret, ops.0.borrow().clone())
    }

    const E: Op = Op::Eql; // 163, brevity in the pinned sequences below
    const M: Op = Op::Mod; // 166
    const I: Op = Op::Ins; // 165
    const X: Op = Op::Esc; // 167

    fn op(opr: Op, len: i64, org: i32, new: i32) -> (Op, i64, i32, i32) {
        (opr, len, org, new)
    }

    /// Brief step-1 test: identical files emit no difference operands — only
    /// the 2 byte-mode EQL calls (MINEQL=2), the accumulated EQL length flush
    /// and the final ESC (pinned against the C++ oracle), ret 0.
    #[test]
    fn identical_files_emit_nothing() {
        let data = b"hello world\n".to_vec();
        let (ret, ops) = run(data.clone(), data);
        assert!(ret.is_ok());
        assert_eq!(
            ops,
            vec![
                op(E, 1, 104, 104), // 'h' ×2 byte-mode EQL
                op(E, 1, 101, 101), // 'e' -> length mode granted (MINEQL=2)
                op(E, 10, 0, 0),    // flush_eql flush of the remaining 10
                op(X, 0, 0, 0),     // final ESC
            ]
        );
        // The brief's core intent: no difference operand was emitted.
        assert!(
            ops.iter()
                .all(|&(opr, ..)| matches!(opr, Op::Eql | Op::Esc))
        );
    }

    /// Brief step-1 test: "" → "abc" emits three byte-wise INS operands (org =
    /// EOF as in the C++ `put(INS, 1, lcOrg, lcNew, …)`) plus the final ESC;
    /// ret 0.
    #[test]
    fn pure_insert() {
        let (ret, ops) = run(Vec::new(), b"abc".to_vec());
        assert!(ret.is_ok());
        assert_eq!(
            ops,
            vec![
                op(I, 1, -1, 97), // 'a'
                op(I, 1, -1, 98), // 'b'
                op(I, 1, -1, 99), // 'c'
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Brief step-1 test: "abc" → "". The C++ emits NO delete here: the main
    /// loop never runs (`lcNew` starts at EOF) and the trailing original bytes
    /// are implicitly dropped at the end of the patch — the output is just the
    /// final ESC operand and ret 0 (pinned against the C++ oracle; the brief's
    /// "one DEL len 3" contradicts the C++, which is the binding authority).
    #[test]
    fn pure_delete() {
        let (ret, ops) = run(b"abc".to_vec(), Vec::new());
        assert!(ret.is_ok());
        assert_eq!(ops, vec![op(X, 0, 0, 0)]);
    }

    /// Brief step-1 test: "hello world" ×3 vs the XYZ modification. The
    /// engine ops must reproduce the reference patch bytes `a7 a3 11 58 59
    /// 5a 6c 6f 20 77 6f 72 6c 64 20 68 65 6c 6c 6f 0a` produced by the C++
    /// `jdiff` + `JOutBin` oracle for this fixture (org = new = 36 bytes,
    /// `hello world ` ×3 with the final space turned into a newline and the
    /// second word's `wor` replaced by `XYZ`). 0.8.5 wire format (spec
    /// §18.C): the MOD run rides the implicit MOD — the 0.8.1 bytes carried
    /// an extra `a7 a6` after the EQL record.
    #[test]
    fn modify_run() {
        let org = b"hello world hello world hello world\n".to_vec();
        let new = b"hello world hello XYZlo world hello\n".to_vec();
        let (ret, ops) = run(org, new);
        assert!(ret.is_ok());

        // Exact C++-pinned operand sequence (oracle2, fixture 3, scn=1;
        // re-verified against the 0.8.5 engine — identical match decisions;
        // with MINEQL=2 the recorder grants length mode after 2 byte-wise
        // EQLs instead of 4).
        assert_eq!(
            ops,
            vec![
                op(E, 1, 104, 104), // 'h' ×2 byte-mode EQL
                op(E, 1, 101, 101), // 'e' -> length mode granted (MINEQL=2)
                op(E, 16, 0, 0),    // flush_eql flush (positions 4..18)
                op(M, 1, 119, 88),  // 'w' -> 'X'
                op(M, 1, 111, 89),  // 'o' -> 'Y'
                op(M, 1, 114, 90),  // 'r' -> 'Z'
                op(E, 1, 108, 108), // single equal byte rides the MOD stream
                op(M, 1, 100, 111), // 'd' -> 'o'
                op(E, 1, 32, 32),
                op(M, 1, 104, 119), // 'h' -> 'w'
                op(M, 1, 101, 111), // 'e' -> 'o'
                op(M, 1, 108, 114), // 'l' -> 'r'
                op(E, 1, 108, 108),
                op(M, 1, 111, 100), // 'o' -> 'd'
                op(E, 1, 32, 32),
                op(M, 1, 119, 104), // 'w' -> 'h'
                op(M, 1, 111, 101), // 'o' -> 'e'
                op(M, 1, 114, 108), // 'r' -> 'l'
                op(E, 1, 108, 108),
                op(M, 1, 100, 111), // 'd' -> 'o'
                op(E, 1, 10, 10),   // '\n'
                op(X, 0, 0, 0),     // final ESC
            ]
        );

        // Replaying the recorded operands through JOutBin must reproduce the
        // reference patch bytes byte for byte (the recorder drops only the
        // positions, which JOutBin ignores).
        let mut bin = JOutBin::new(Vec::new());
        for &(opr, len, org, new) in &ops {
            bin.put(opr, len, org, new, 0, 0);
        }
        assert_eq!(
            bin.into_inner(),
            vec![
                0xa7, 0xa3, 0x11, 0x58, 0x59, 0x5a, 0x6c, 0x6f, 0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64,
                0x20, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x0a,
            ]
        );
    }

    /// Brief step-1 test: a duplicated block produces a BKT (backtrack)
    /// operand. org = 1000 LCG bytes; new = org[0..800] + org[300..800] +
    /// org[800..1000], so the block org[300..800] appears twice. The whole
    /// operand sequence is pinned against the C++ oracle (oracle3, fixture 1;
    /// re-verified against the 0.8.5 engine — identical op log, 128 hash
    /// hits, one repair).
    #[test]
    fn repeated_block_produces_bkt() {
        let org = lcg_bytes(1, 1000);
        let mut new = Vec::with_capacity(1500);
        new.extend_from_slice(&org[0..800]);
        new.extend_from_slice(&org[300..800]);
        new.extend_from_slice(&org[800..1000]);
        let (ret, ops) = run(org, new);
        assert!(ret.is_ok());
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133), // -> length mode granted (MINEQL=2)
                op(E, 798, 0, 0),
                op(Op::Bkt, 500, 0, 0), // backtrack 500 bytes on the original file
                op(E, 1, 190, 190),
                op(E, 1, 145, 145), // -> length mode granted (MINEQL=2)
                op(E, 698, 0, 0),
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Brief step-1 test: removing 100 bytes at offset 500 produces a DEL of
    /// exactly 100 right after the first 500 equal bytes. org = 1000 LCG
    /// bytes; new = org minus [500..600). Whole sequence pinned against the
    /// C++ oracle (oracle3, fixture 0; re-verified against the 0.8.5 engine —
    /// identical op log, 128 hash hits, one repair).
    #[test]
    fn small_shift_produces_del_or_ins() {
        let org = lcg_bytes(1, 1000);
        let mut new = Vec::with_capacity(900);
        new.extend_from_slice(&org[0..500]);
        new.extend_from_slice(&org[600..1000]);
        let (ret, ops) = run(org, new);
        assert!(ret.is_ok());
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133),     // -> length mode granted (MINEQL=2)
                op(E, 498, 0, 0),       // 500 equal bytes total
                op(Op::Del, 100, 0, 0), // delete the 100 shifted-out bytes
                op(E, 1, 103, 103),
                op(E, 1, 136, 136), // -> length mode granted (MINEQL=2)
                op(E, 398, 0, 0),   // remaining 400 equal bytes
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Brief step-1 test: read errors surface through the **live** `int liFnd`
    /// check (`JDiff.cpp:277-279` — the 0.8.1 `bool lbFnd` collapse of spec
    /// §18.E is FIXED at 0.8.5):
    ///
    /// * a failing **original** file aborts inside `search` — `buildFullIndex`
    ///   returns `EXI_RED`, `search` propagates it, `jdiff` returns it
    ///   immediately; no operands are emitted at all (the 0.8.1 engine kept
    ///   running and drained the new file as INS bytes);
    /// * a failing **new** file ends the main loop before it starts: the
    ///   final EOB check returns `min(EOF, EXI_RED)` = `EXI_RED` after the
    ///   trailing ESC.
    #[test]
    fn engine_error_propagates_via_li_fnd() {
        // Failing original: search's buildFullIndex fails -> jdiff returns
        // EXI_RED with an empty op log.
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(FailingJFile),
            Box::new(JFileMem::new(b"hello world\n".to_vec())),
            Box::new(rec),
        );
        assert_eq!(jd.jdiff().unwrap_err().exit_code(), EXI_RED);
        assert!(
            ops.0.borrow().is_empty(),
            "no operands may be emitted once liFnd goes negative: {:?}",
            ops.0.borrow()
        );

        // Failing new file: the loop never runs, the final EOB check returns
        // the error behind the trailing ESC.
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(JFileMem::new(b"hello world\n".to_vec())),
            Box::new(FailingJFile),
            Box::new(rec),
        );
        assert_eq!(jd.jdiff().unwrap_err().exit_code(), EXI_RED);
        assert_eq!(ops.0.borrow().clone(), vec![op(X, 0, 0, 0)]);
    }

    /// Brief step-1 test: with `src_scn = 0` (`-ff`) the source index builds
    /// **incrementally** — in the compare loop (top of the while + inside the
    /// equal-run fast loops, `JDiff.cpp:185-224`) and via the SoftAhead
    /// prescan in `search` (`JDiff.cpp:419-447`). Fixture: 3000 LCG bytes
    /// with the block org[1000..1200) deleted. The whole operand sequence is
    /// pinned against the 0.8.5 C++ engine (oracle harness, fixture scn,
    /// srcScn=0 — identical output with srcScn=1); `hash_hits()` proves the
    /// incrementally built index actually served the lookups (table size
    /// 1 MB: 87381 elements → prime 87359 on both the port and the C++
    /// 32-bit-hkey build, spec §21.18 — ops verified identical).
    #[test]
    fn src_scn_0_incremental_indexing_shifted_block() {
        let org = lcg_bytes(1, 3000);
        let mut new = Vec::with_capacity(2800);
        new.extend_from_slice(&org[0..1000]);
        new.extend_from_slice(&org[1200..3000]);

        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = JDiff::new(
            Box::new(JFileMem::new(org)),
            Box::new(JFileMem::new(new)),
            Box::new(rec),
            1,     // hsh_sze (MB)
            0,     // verbose
            true,  // src_bkt
            false, // src_scn = 0: incremental indexing, no full prescan
            8,     // mch_max
            4,     // mch_min
            256 * 1024,
            true, // cmp_all
        );
        assert!(jd.jdiff().is_ok());
        assert_eq!(
            ops.0.borrow().clone(),
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133), // -> length mode granted (MINEQL=2)
                op(E, 998, 0, 0),
                op(Op::Del, 200, 0, 0),
                op(E, 1, 49, 49),
                op(E, 1, 35, 35), // -> length mode granted (MINEQL=2)
                op(E, 1798, 0, 0),
                op(X, 0, 0, 0),
            ]
        );
        // The C++ 32-bit-hkey build's value (spec §21.18: both sides now
        // build the same 1 MB table, 87381 elements → prime 87359) — before
        // the divisor amendment the port's /16 table answered 124.
        assert_eq!(jd.hash().hash_hits(), 127);
    }

    /// Brief step-1 test: the constructor clamps per `JDiff.cpp:119-120` —
    /// `mch_min = mch_min > mch_max ? mch_max - 1 : mch_min` (note: `mch_min
    /// == mch_max` is kept, only `>` clamps) and `ahd_max` is raised to at
    /// least 1024. Also pins the MB hash wiring: `hsh_sze = 32` builds the
    /// 0.8.5 default table (32 MB -> 2796202 elements -> prime 2796181,
    /// spec §21.18).
    #[test]
    fn ctor_clamps_and_mb_hash_wiring() {
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = JDiff::new(
            Box::new(JFileMem::new(b"a".to_vec())),
            Box::new(JFileMem::new(b"b".to_vec())),
            Box::new(rec),
            32, // hsh_sze in MB: the 0.8.5 default
            0,
            true,
            true,
            32,  // mch_max
            99,  // mch_min > mch_max: clamps to mch_max - 1
            100, // ahd_max < 1024: clamps up to 1024
            true,
        );
        assert_eq!(jd.mch_min, 31, "mch_min clamps to mch_max - 1");
        assert_eq!(jd.ahd_max, 1024, "ahd_max is raised to 1024");
        assert_eq!(jd.hash().hash_prime(), 2_796_181, "hsh_sze=32 MB wiring");
        assert!(jd.jdiff().is_ok());
    }

    /// Two separated single-byte edits exercise two full search rounds, the
    /// prescan-once dispatch (src_scn 1 → 2) and the post-prescan lookahead
    /// budget branch. Sequence pinned against the C++ oracle (oracle5) and
    /// re-verified against the 0.8.5 engine — identical op log (761 hash
    /// hits there); the LCG bytes themselves double as a fixture cross-check.
    #[test]
    fn two_edits_two_find_ahead_rounds() {
        let mut org = lcg_bytes(1, 1000);
        let mut new = org.clone();
        new[250] = org[250] ^ 0xff;
        new[750] = org[750] ^ 0xff;
        org[0] = 89; // guard the LCG against accidental drift (see oracle)
        let (ret, ops) = run(org, new);
        assert!(ret.is_ok());
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133), // -> length mode granted (MINEQL=2)
                op(E, 248, 0, 0),
                op(M, 1, 60, 195), // edit 1: 60 ^ 0xff = 195
                op(E, 1, 196, 196),
                op(E, 1, 187, 187), // -> length mode granted (MINEQL=2)
                op(E, 497, 0, 0),
                op(M, 1, 124, 131), // edit 2: 124 ^ 0xff = 131
                op(E, 1, 38, 38),
                op(E, 1, 237, 237), // -> length mode granted (MINEQL=2)
                op(E, 247, 0, 0),
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Statistics and false-hit repairs: for the zero-block fixture the
    /// hashtable answers 196 lookups with a key hit and the engine repairs 0
    /// of them — both pinned against the 0.8.5 C++ engine (oracle harness,
    /// fixture zb; ops and hits verified table-size-independent at 1/2/8/32
    /// MB). The miss counter `hsh_err` stays 0 (no "Inaccurate solution"
    /// event fires on this fixture — the C++ member is uninitialized garbage
    /// before the first increment, so only the *increments* are engine
    /// facts; the port deterministically starts at 0). The repairs statistic
    /// is read from the engine's matching table through `hsh_rpr` (0.8.5
    /// instance counter `getHshRpr`, `JMatchTable.cpp:930-932`).
    #[test]
    fn stats_and_hash_repairs() {
        // org: 400 LCG bytes with a 100-byte zero block at [200..300);
        // new: the zero block moved 50 bytes to the right (50 bytes taken
        // from org[300..350] inserted before it).
        let org = zero_block_fixture();
        let new = zero_block_moved_fixture(&org);

        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(JFileMem::new(org)),
            Box::new(JFileMem::new(new)),
            Box::new(rec),
        );
        assert!(jd.jdiff().is_ok());
        assert_eq!(jd.hash().hash_hits(), 196);
        assert_eq!(jd.hsh_err(), 0);
        assert_eq!(jd.hsh_rpr(), 0);
    }

    /// 400 LCG bytes with a 100-byte zero block at [200..300).
    fn zero_block_fixture() -> Vec<u8> {
        let mut org = lcg_bytes(1, 400);
        for b in &mut org[200..300] {
            *b = 0;
        }
        org
    }

    /// The zero-block fixture with the zero block moved 50 bytes to the right
    /// (50 bytes taken from org[300..350] inserted before it).
    fn zero_block_moved_fixture(org: &[u8]) -> Vec<u8> {
        let mut new = Vec::with_capacity(400);
        new.extend_from_slice(&org[0..200]);
        new.extend_from_slice(&org[300..350]);
        new.extend_from_slice(&org[200..300]);
        new.extend_from_slice(&org[350..400]);
        new
    }
}
