//! JDiff engine: the main diffing loop, ported 1:1 from C++ `src/JDiff.cpp` +
//! `headers/JDiff.h` (spec §6).
//!
//! [`JDiff::jdiff`] compares both files byte by byte and, on a mismatch, calls
//! [`JDiff::uf_fnd_ahd`] to find the nearest equal region ahead: the prescan
//! ([`JDiff::uf_fnd_ahd_scn`]) fills a [`JHashPos`] hashtable with 32-byte
//! samples of the original file, the find-ahead probes it with samples of the
//! new file and hands the hits to a [`JMatchTable`], whose best verified match
//! is turned into DEL/BKT/INS instructions plus an ahead counter. The engine
//! drives a [`JOut`] sink; the `EQL` return-value contract (byte mode until the
//! sink grants length mode) is respected exactly as in the C++.
//!
//! Two original quirks are replicated 1:1 (both verified against the C++
//! oracle):
//!
//! * `lbFnd` is a `bool` in the C++ (`JDiff.cpp:132`), so the negative error
//!   return of `ufFndAhd` converts to `true` and the `if (lbFnd < 0)` check at
//!   `JDiff.cpp:213` is dead code. Read errors therefore do not abort the loop;
//!   they surface through the final `lcNew < EOB || lcOrg < EOB` check
//!   (`JDiff.cpp:252-255`), after the new file has been drained as INS bytes.
//! * Deleting the whole original file (`"abc" → ""`) emits nothing but the
//!   final ESC operand: the main loop never runs because `lcNew` starts at
//!   `EOF`, and no trailing DEL is generated.
//!
//! Debug output (`#if debug` blocks, spec §14) is Task 11 and intentionally
//! absent; `giHshErr` therefore always stays 0 in this build, like the C++
//! release build.

use crate::defs::{BKT, DEL, EOB, EOF, EQL, ESC, INS, MOD, ReadType, SMPSZE};
use crate::jdebug::dbg_print;
use crate::jfile::JFile;
use crate::jhashpos::JHashPos;
use crate::jmatchtable::JMatchTable;
use crate::jout::JOut;

/// JDiff engine (`JDiff.h:153`): owns the two file readers, the output sink,
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
    /// Min number of matches to find (`miMchMin`).
    mch_min: i32,
    /// Max number of bytes to look ahead (`miAhdMax`; C++ stores an int, the
    /// CLI-facing constructor parameter is `i64` per the port interface).
    ahd_max: i64,
    /// Compare all matches, even if data not in buffer? (`mbCmpAll`).
    cmp_all: bool,
    /// Prescan original file: 0=no, 1=yes, 2=done (`miSrcScn`).
    src_scn: i32,

    /// Current ahead position on the original file (`mzAhdOrg`).
    az_org: i64,
    /// Current ahead position on the new file (`mzAhdNew`).
    az_new: i64,
    /// Current hash value for the original file (`mlHshOrg`).
    hsh_org: u32,
    /// Current hash value for the new file (`mlHshNew`).
    hsh_new: u32,
    /// Current file value, original (`miValOrg`).
    val_org: i32,
    /// Current file value, new (`miValNew`).
    val_new: i32,
    /// Equal-byte counter in the current sample, original (`miEqlOrg`).
    eql_org: i32,
    /// Equal-byte counter in the current sample, new (`miEqlNew`).
    eql_new: i32,
    /// Number of false hash hits (`giHshErr`; only incremented in the C++
    /// debug build, therefore always 0 here).
    hsh_err: i32,
}

impl<'a> JDiff<'a> {
    /// Create JDiff for working on the specified files (`JDiff.cpp:83-99`).
    ///
    /// `ahd_max` is raised to at least 1024 (`miAhdMax(aiAhdMax<1024?1024:
    /// aiAhdMax)`); `src_scn` becomes the C++ `miSrcScn` int (false = 0,
    /// true = 1) and is set to 2 by [`JDiff::uf_fnd_ahd`] after the prescan.
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
        JDiff {
            org,
            r#new,
            out,
            hsh: JHashPos::new(hsh_sze),
            mch: JMatchTable::new(),
            verbose,
            src_bkt,
            mch_max,
            mch_min,
            ahd_max: ahd_max.max(1024),
            cmp_all,
            src_scn: i32::from(src_scn),
            az_org: 0,
            az_new: 0,
            hsh_org: 0,
            hsh_new: 0,
            val_org: 0,
            val_new: 0,
            eql_org: 0,
            eql_new: 0,
            hsh_err: 0,
        }
    }

    /// Hashtable accessor for the post-run statistics (`getHsh`,
    /// `JDiff.h:201`).
    pub fn hash(&self) -> &JHashPos {
        &self.hsh
    }

    /// Number of false hash hits (`getHshErr`, `JDiff.h:202`); always 0 in the
    /// release build, like the C++.
    pub fn hsh_err(&self) -> i32 {
        self.hsh_err
    }

    /// Difference function (`JDiff::jdiff`, `JDiff.cpp:122-256`): compares both
    /// files byte by byte and writes the differences to the output handler.
    ///
    /// Returns 0 on success or a negative `EXI_*` read-error code (which, as in
    /// the C++, reaches the return statement only via the final error check —
    /// see the module docs on the `bool lbFnd` quirk).
    pub fn jdiff(&mut self) -> i32 {
        let mut lc_org: i32; /* byte from original file */
        let mut lc_new: i32; /* byte from new file */
        let mut lz_pos_org: i64 = 0;
        let mut lz_pos_new: i64 = 0;

        let mut lb_eql = false; /* accumulate equal bytes? */
        let mut lz_eql: i64 = 0; /* accumulated equal bytes */

        let mut lb_fnd = false; /* offsets are pointing to a valid solution? */
        let mut lz_ahd: i64 = 0;
        let mut lz_skp_org: i64 = 0;
        let mut lz_skp_new: i64 = 0;

        /* Take one byte from each file ... (JDiff.cpp:141-143) */
        lc_org = self.org.get(lz_pos_org, ReadType::Read);
        lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
        while lc_new >= 0 {
            if lc_org == lc_new {
                /* Output or count equals (JDiff.cpp:151-157) */
                if lb_eql {
                    lz_eql += 1;
                } else {
                    lb_eql = self.out.put(EQL, 1, lc_org, lc_new, lz_pos_org, lz_pos_new);
                }

                /* Take next byte from each file ... */
                lz_pos_org += 1;
                lc_org = self.org.get(lz_pos_org, ReadType::Read);
                lz_pos_new += 1;
                lc_new = self.r#new.get(lz_pos_new, ReadType::Read);

                /* decrease ahead counter */
                lz_ahd -= 1;
            } else if lz_ahd > 0 {
                /* Output accumulated equals (JDiff.cpp:165-184) */
                self.uf_put_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);

                /* Output difference */
                if lc_org < 0 {
                    self.out.put(INS, 1, lc_org, lc_new, lz_pos_org, lz_pos_new);

                    /* Take next byte from each file ... */
                    lz_pos_new += 1;
                    lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                } else {
                    self.out.put(MOD, 1, lc_org, lc_new, lz_pos_org, lz_pos_new);

                    /* Take next byte from each file ... */
                    lz_pos_org += 1;
                    lc_org = self.org.get(lz_pos_org, ReadType::Read);
                    lz_pos_new += 1;
                    lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                }

                /* decrease ahead counter */
                lz_ahd -= 1;
            } else if lb_fnd && lz_ahd == 0 {
                /* to avoid infinite loop (when ufFabFnd persists with
                 * lzSkpOrg, lzSkpNew, lzAhd all zero, JDiff.cpp:186-191) */
                lz_ahd = i64::from(SMPSZE);
                lb_fnd = false;
            } else {
                /* Find a new equals-region (JDiff.cpp:211-214).
                 *
                 * The C++ assigns the int return to the bool lbFnd, turning
                 * negative error codes into `true` and making the following
                 * `if (lbFnd < 0) return lbFnd` dead code; read errors are
                 * therefore not fatal here and surface via the final EOB
                 * check below. Replicated 1:1 with `!= 0`. */
                lb_fnd = self.uf_fnd_ahd(
                    lz_pos_org,
                    lz_pos_new,
                    &mut lz_skp_org,
                    &mut lz_skp_new,
                    &mut lz_ahd,
                ) != 0;

                /* Output accumulated equals (JDiff.cpp:224-225) */
                self.uf_put_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);

                /* Execute offsets (JDiff.cpp:227-243) */
                if lz_skp_org > 0 {
                    self.out.put(DEL, lz_skp_org, 0, 0, lz_pos_org, lz_pos_new);
                    lz_pos_org += lz_skp_org;
                    lc_org = self.org.get(lz_pos_org, ReadType::Read);
                } else if lz_skp_org < 0 {
                    self.out.put(BKT, -lz_skp_org, 0, 0, lz_pos_org, lz_pos_new);
                    lz_pos_org += lz_skp_org;
                    lc_org = self.org.get(lz_pos_org, ReadType::Read);
                }
                if lz_skp_new > 0 {
                    while lz_skp_new > 0 && lc_new > EOF {
                        self.out.put(INS, 1, 0, lc_new, lz_pos_org, lz_pos_new);
                        lz_skp_new -= 1;
                        lz_pos_new += 1;
                        lc_new = self.r#new.get(lz_pos_new, ReadType::Read);
                    }
                }
            } /* if lcOrg == lcNew */
        } /* while lcNew >= 0 */

        /* Flush output buffer (JDiff.cpp:247-249) */
        self.uf_put_eql(lz_pos_org, lz_pos_new, &mut lz_eql, &mut lb_eql);
        self.out.put(ESC, 0, 0, 0, lz_pos_org, lz_pos_new);

        /* Return code (JDiff.cpp:251-255) */
        if lc_new < EOB || lc_org < EOB {
            return if lc_new < lc_org { lc_new } else { lc_org };
        }
        0
    } /* jdiff */

    /// Flush pending output (`JDiff::ufPutEql`, `JDiff.cpp:261-268`).
    ///
    /// `lz_eql`/`lb_eql` are the accumulation state of [`JDiff::jdiff`], which
    /// the C++ passes by reference (`off_t &lzEql, bool &lbEql`).
    fn uf_put_eql(&mut self, pos_org: i64, pos_new: i64, lz_eql: &mut i64, lb_eql: &mut bool) {
        /* Output accumulated equals (JDiff.cpp:263-266) */
        if *lz_eql > 0 {
            self.out
                .put(EQL, *lz_eql, 0, 0, pos_org - *lz_eql, pos_new - *lz_eql);
            *lz_eql = 0;
        }
        *lb_eql = false;
    }

    /// Find Ahead function (`JDiff::ufFndAhd`, `JDiff.cpp:285-488`): reads
    /// ahead on both files looking for an equal series of 32-byte samples and
    /// calculates the displacement vector between the files.
    ///
    /// Returns 0 = no solution found, 1 = solution found, < 0 = EXI error code.
    fn uf_fnd_ahd(
        &mut self,
        red_org: i64,
        red_new: i64,
        skp_org: &mut i64,
        skp_new: &mut i64,
        ahd: &mut i64,
    ) -> i32 {
        /* Prescan the original file? (JDiff.cpp:303-308) */
        if self.src_scn == 1 {
            let li_ret = self.uf_fnd_ahd_scn();
            if li_ret < 0 {
                return li_ret;
            }
            self.src_scn = 2;
        }

        /* Field-disjoint borrows make the C++ pointer aliasing legal in Rust
         * (the C++ reaches into gpHsh, gpMch, mpFilOrg, mpFilNew and the
         * state members simultaneously). */
        let Self {
            org,
            r#new,
            hsh,
            mch,
            src_bkt,
            mch_max,
            mch_min,
            ahd_max,
            cmp_all,
            src_scn,
            az_org: mz_ahd_org,
            az_new: mz_ahd_new,
            hsh_org: ml_hsh_org,
            hsh_new: ml_hsh_new,
            val_org: mi_val_org,
            val_new: mi_val_new,
            eql_org: mi_eql_org,
            eql_new: mi_eql_new,
            ..
        } = self;

        let mut lz_fnd_org: i64 = 0; /* Found position within original file;
         * the C++ also declares lzFndNew (out
         * parameter of gpMch->get), which the
         * Option return replaces here. */

        let mut li_fnd: i32 = 0; /* Number of matches found */
        /* Start with hard lookahead, till we've found at least one match
         * (JDiff.cpp:300-301); liSft is never changed in the C++. */
        let li_sft = ReadType::HardAhead;

        /*
         * How many bytes to look ahead ? (JDiff.cpp:311-324)
         */
        /* The C++ stores miAhdMax as int; the CLI cannot produce values
         * beyond the i32 range (atoi clamps), so narrowing saturates only
         * for values the C++ would already have truncated at its int
         * constructor parameter. */
        let mi_ahd_max = i32::try_from(*ahd_max).unwrap_or(i32::MAX);
        let mut li_max: i32 = if *src_scn == 2 {
            /* The C++ spells the first two branches out separately
             * (JDiff.cpp:315-318); both assign miAhdMax, so they are
             * merged here. */
            if *mz_ahd_new == 0
                || *mz_ahd_new < red_new
                || *mz_ahd_new > red_new + i64::from(mi_ahd_max)
            {
                mi_ahd_max
            } else {
                /* C++: liMax = miAhdMax - (mzAhdNew - azRedNew), an off_t
                 * difference truncated on the int assignment; within this
                 * branch the difference fits an i32. */
                mi_ahd_max - ((*mz_ahd_new - red_new) as i32)
            }
        } else {
            i32::MAX / 2
        };

        /*
         * How many bytes to look back on reset ? (JDiff.cpp:326-333)
         */
        let li_bck: i32 = if hsh.reliability() < mi_ahd_max {
            hsh.reliability() / 2
        } else {
            mi_ahd_max / 2
        };

        /*
         * Re-Initialize hash function (read 31 bytes) if
         * - ahead position has been reset, or
         * - read position has passed the ahead position
         * (JDiff.cpp:335-352)
         */
        if *src_scn == 0 && (*mz_ahd_org == 0 || *mz_ahd_org + i64::from(li_bck) < red_org) {
            *mz_ahd_org = red_org - i64::from(li_bck);
            if *mz_ahd_org < 0 {
                *mz_ahd_org = 0;
            }
            *mi_eql_org = 0;
            *ml_hsh_org = 0;

            *mi_eql_org = 0;
            *mi_val_org = org.get(*mz_ahd_org, li_sft);
            let mut li_idx = 0;
            while li_idx < SMPSZE - 1 && *mi_val_org > EOF {
                hsh.hash(*mi_val_org, ml_hsh_org);
                *mz_ahd_org += 1;
                uf_fnd_ahd_get(&mut **org, *mz_ahd_org, mi_val_org, mi_eql_org, li_sft);
                li_idx += 1;
            }
        }
        /* (JDiff.cpp:353-368) */
        if *mz_ahd_new == 0 || *mz_ahd_new + i64::from(li_bck) < red_new {
            *mz_ahd_new = red_new - i64::from(li_bck);
            if *mz_ahd_new < 0 {
                *mz_ahd_new = 0;
            }
            *mi_eql_new = 0;
            *ml_hsh_new = 0;
            li_max += li_bck;

            *mi_eql_new = 0;
            *mi_val_new = r#new.get(*mz_ahd_new, li_sft);
            li_max -= 1;
            let mut li_idx = 0;
            while li_idx < SMPSZE - 1 && *mi_val_new > EOF {
                hsh.hash(*mi_val_new, ml_hsh_new);
                *mz_ahd_new += 1;
                uf_fnd_ahd_get(&mut **r#new, *mz_ahd_new, mi_val_new, mi_eql_new, li_sft);
                li_max -= 1;
                li_idx += 1;
            }
        }

        /*
         * Build the table of matches (JDiff.cpp:372-437)
         */
        if mch.cleanup(red_new - i64::from(hsh.reliability())) {
            /* Do not backtrace before lzBseOrg (JDiff.cpp:374-375) */
            let lz_bse_org: i64 = if *src_bkt { 0 } else { red_org };

            /* Do not read from original file if it has been prescanned
             * (JDiff.cpp:377-378) */
            if *src_scn > 0 {
                *mi_val_org = EOB;
            }

            /* Scroll through both files until an equal hash value has been
             * found (JDiff.cpp:381-436) */
            while li_max > 0 && (*mi_val_new > EOF || *mi_val_org > EOF) {
                /* insert original file's value into hashtable (if no
                 * prescanning has been done) */
                if *mi_val_org > EOF {
                    /* hash the new value and add to hashtable */
                    hsh.hash(*mi_val_org, ml_hsh_org);
                    hsh.add(*ml_hsh_org, *mz_ahd_org, *mi_eql_org);

                    /* get next value from file */
                    *mz_ahd_org += 1;
                    uf_fnd_ahd_get(&mut **org, *mz_ahd_org, mi_val_org, mi_eql_org, li_sft);
                }

                /* check new file against original file */
                if *mi_val_new > EOF {
                    /* hash the new value and lookup in hashtable */
                    hsh.hash(*mi_val_new, ml_hsh_new);
                    if hsh.get(*ml_hsh_new, &mut lz_fnd_org) {
                        /* add found position into table of matches */
                        if lz_fnd_org > lz_bse_org {
                            /* add solution to the table of matches; the C++
                             * switch falls through from case 0 into case 1
                             * when the cleanup made room (JDiff.cpp:406-428)
                             */
                            let fallthrough_to_1 =
                                match mch.add(lz_fnd_org, *mz_ahd_new, red_new, *mi_eql_new) {
                                    /* table is full but cleanup made room */
                                    0 if li_bck > 0 && mch.cleanup(red_new) => true,
                                    /* table is full: stop lookahead */
                                    0 => {
                                        li_max = 0;
                                        continue;
                                    }
                                    /* alternative added */
                                    1 => true,
                                    /* 2: alternative collided; -1: compare failed */
                                    _ => false,
                                };
                            if fallthrough_to_1 && *mz_ahd_new > red_new {
                                li_fnd += 1;

                                if li_fnd == *mch_max {
                                    li_max = 0; // stop lookahead
                                    continue;
                                } else if li_fnd == *mch_min && li_max > hsh.reliability() {
                                    li_max = hsh.reliability(); // reduce lookahead
                                }
                            }
                        }
                    }

                    /* get next value from file */
                    *mz_ahd_new += 1;
                    uf_fnd_ahd_get(&mut **r#new, *mz_ahd_new, mi_val_new, mi_eql_new, li_sft);
                    li_max -= 1;
                } /* if siValNew > EOF */
            } /* while */
        } /* if ufMchFre(..) */

        /*
         * Check for errors (JDiff.cpp:439-444)
         */
        if *mi_val_new < EOB || *mi_val_org < EOB {
            return if *mi_val_new < *mi_val_org {
                *mi_val_new
            } else {
                *mi_val_org
            };
        }

        /*
         * Get the best match and calculate the offsets (JDiff.cpp:448-487)
         */
        match mch.get(red_org, red_new, hsh, &mut **org, &mut **r#new, *cmp_all) {
            None => {
                *skp_org = 0;
                *skp_new = 0;
                *ahd = (*mz_ahd_new - red_new) - i64::from(hsh.reliability());
                if *ahd < i64::from(SMPSZE) {
                    *ahd = i64::from(SMPSZE);
                }
                0
            }
            Some((lz_fnd_org, lz_fnd_new)) => {
                if lz_fnd_org >= red_org {
                    if lz_fnd_org - red_org >= lz_fnd_new - red_new {
                        /* go forward on original file (JDiff.cpp:457-462) */
                        *skp_org = lz_fnd_org - red_org + red_new - lz_fnd_new;
                        *skp_new = 0;
                        *ahd = lz_fnd_new - red_new;
                    } else {
                        /* go forward on new file (JDiff.cpp:462-467) */
                        *skp_org = 0;
                        *skp_new = lz_fnd_new - red_new + red_org - lz_fnd_org;
                        *ahd = lz_fnd_org - red_org;
                    }
                } else {
                    /* backtrack on original file (JDiff.cpp:468-484) */
                    *skp_org = red_org - lz_fnd_org + lz_fnd_new - red_new;
                    if *skp_org < red_org {
                        *skp_new = 0;
                        *skp_org = -*skp_org;
                        *ahd = lz_fnd_new - red_new;
                    } else {
                        /* do not backtrack before beginning of file */
                        *skp_new = *skp_org - red_org;
                        *skp_org = -red_org;
                        *ahd = (lz_fnd_new - red_new) - *skp_new;
                    }

                    /* reset ahead position when backtracking */
                    *mz_ahd_org = 0; // TODO reset matching table too?
                }

                1
            }
        }
    }

    /// Prescan the original file (`JDiff::ufFndAhdScn`, `JDiff.cpp:519-583`):
    /// calculates a hash-key for every 32-byte sample and stores it with its
    /// position in the hashtable. Serial port of the OpenMP block (spec §15.2).
    fn uf_fnd_ahd_scn(&mut self) -> i32 {
        let Self {
            org, hsh, verbose, ..
        } = self;

        let mut lk_hsh_org: u32 = 0; // Current hash value for original file
        let mut li_eql_org: i32 = 0; // Number of times current value occurs
        let mut lc_val_org: i32; // Current file value
        let mut lz_pos_org: i64 = 0; // Position within original file

        if *verbose > 0 {
            dbg_print(format_args!("Prescanning:\n"));
        }

        /* Initialize hash function (JDiff.cpp:532-537) */
        lc_val_org = org.get(lz_pos_org, ReadType::HardAhead);
        let mut li_idx = 0;
        while li_idx < SMPSZE - 1 && lc_val_org > EOF {
            hsh.hash(lc_val_org, &mut lk_hsh_org);
            lz_pos_org += 1;
            uf_fnd_ahd_get(
                &mut **org,
                lz_pos_org,
                &mut lc_val_org,
                &mut li_eql_org,
                ReadType::HardAhead,
            );
            li_idx += 1;
        }

        /* Build hashtable (JDiff.cpp:539-568): serial port of the OpenMP
         * parallel block (the pragma is only active in the never-used `make
         * parallel` target and is a data race there; the oracle is the
         * serial default build — spec §15.2). */
        li_idx = 0;
        while lc_val_org > EOF {
            hsh.hash(lc_val_org, &mut lk_hsh_org);
            hsh.add(lk_hsh_org, lz_pos_org, li_eql_org);

            lz_pos_org += 1;
            uf_fnd_ahd_get(
                &mut **org,
                lz_pos_org,
                &mut lc_val_org,
                &mut li_eql_org,
                ReadType::HardAhead,
            );

            if *verbose > 0 {
                /* output a dot every 16MB (JDiff.cpp:554-565) */
                li_idx += 1;
                if (li_idx & 0xff_ffff) == 0 {
                    if li_idx == 0x4000_0000 {
                        li_idx = 0;
                        dbg_print(format_args!(".\n")); /* every 1024MB */
                    } else {
                        dbg_print(format_args!("."));
                    }
                }
            }
        }

        if *verbose > 0 {
            dbg_print(format_args!(".\n"));
        }

        /* (JDiff.cpp:579-582) */
        if lc_val_org < EOB { lc_val_org } else { 0 }
    } /* ufFndAhdScn */
}

/// Get next character from file (lookahead) and count the number of equal
/// chars in the current sample (`JDiff::ufFndAhdGet`, `JDiff.cpp:504-513`).
///
/// `pos` is the position to read (the C++ callers pass `++azPos`), `val` holds
/// the previous byte on entry and the new byte on exit, `eql` is the sample
/// equal-run counter, `sft` the look-ahead read type (always hard-ahead here:
/// `liSft` is 1 throughout the C++).
fn uf_fnd_ahd_get(file: &mut dyn JFile, pos: i64, val: &mut i32, eql: &mut i32, sft: ReadType) {
    let lc_prv = *val;
    *val = file.get(pos, sft);
    if *val != lc_prv {
        if *eql > 0 {
            *eql -= 2;
        }
    } else {
        if *eql < SMPSZE {
            *eql += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::EXI_RED;
    use crate::jfile::JFileMem;
    use crate::jmatchtable::HSH_RPR;
    use crate::jout::{JOutBin, OutStats};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

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
    type OpLog = Vec<(i32, i64, i32, i32)>;

    /// Shared op log so the test can retrieve the (opr, len, org, new) tuples
    /// after the recorder has been moved into the engine.
    #[derive(Clone, Default)]
    struct Ops(Rc<RefCell<OpLog>>);

    /// Recording [`JOut`] that logs `(opr, len, org, new)` tuples and mimics
    /// the `JOutBin` return contract exactly (`JOutBin.cpp:152-225`): EQL
    /// buffers up to 4 bytes (returning true once 4 are pending, thereafter
    /// always true); every non-EQL operand flushes (and thereby resets) the
    /// pending EQL bytes and returns false.
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
            opr: i32,
            len: i64,
            org: i32,
            new: i32,
            _pos_org: i64,
            _pos_new: i64,
        ) -> bool {
            self.ops.0.borrow_mut().push((opr, len, org, new));
            if opr == EQL {
                if self.eql_cnt < 4 {
                    self.eql_cnt += 1;
                    self.eql_cnt >= 4
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

    /// Always-failing original file: every `get` returns `-EXI_RED`, like a
    /// hard read error (`JFile` implementations may return negative EXI codes;
    /// spec §6.2 step 8).
    struct FailingJFile;

    impl JFile for FailingJFile {
        fn get(&mut self, _pos: i64, _typ: ReadType) -> i32 {
            -EXI_RED
        }

        fn seekcount(&self) -> i64 {
            0
        }
    }

    /// Engine with the CLI default settings (spec §4), hashtable 65536 — for
    /// the fixture sizes used here every sample is stored regardless of the
    /// table prime, so behavior is identical to the 8388608 default (verified
    /// against the C++ oracle with both sizes).
    fn engine<'a>(
        org: Box<dyn JFile + 'a>,
        r#new: Box<dyn JFile + 'a>,
        out: Box<dyn JOut + 'a>,
    ) -> JDiff<'a> {
        JDiff::new(
            org,
            r#new,
            out,
            65536,
            0,
            true,
            true,
            8,
            4,
            256 * 1024,
            true,
        )
    }

    /// Serializes the HSH_RPR-sensitive test: [`HSH_RPR`] is a process global
    /// and cargo runs tests on parallel threads by default (same discipline as
    /// the `jmatchtable` tests' `hsh_rpr_guard`). Poison-immune so a panicking
    /// sibling cannot turn into a misleading secondary failure.
    fn hsh_rpr_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Drives the engine over the given file pair and returns (ret, ops).
    fn run(org: Vec<u8>, new: Vec<u8>) -> (i32, OpLog) {
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

    const E: i32 = EQL; // 163, brevity in the pinned sequences below
    const M: i32 = MOD; // 166
    const I: i32 = INS; // 165
    const X: i32 = ESC; // 167

    fn op(opr: i32, len: i64, org: i32, new: i32) -> (i32, i64, i32, i32) {
        (opr, len, org, new)
    }

    /// Brief step-1 test: identical files emit no difference operands — only
    /// the 4 byte-mode EQL calls, the accumulated EQL length flush and the
    /// final ESC (pinned against the C++ oracle), ret 0.
    #[test]
    fn identical_files_emit_nothing() {
        let data = b"hello world\n".to_vec();
        let (ret, ops) = run(data.clone(), data);
        assert_eq!(ret, 0);
        assert_eq!(
            ops,
            vec![
                op(E, 1, 104, 104), // 'h'
                op(E, 1, 101, 101), // 'e'
                op(E, 1, 108, 108), // 'l'
                op(E, 1, 108, 108), // 'l' -> length mode granted
                op(E, 8, 0, 0),     // uf_put_eql flush of the remaining 8
                op(X, 0, 0, 0),     // final ESC
            ]
        );
        // The brief's core intent: no difference operand was emitted.
        assert!(ops.iter().all(|&(opr, ..)| matches!(opr, EQL | ESC)));
    }

    /// Brief step-1 test: "" → "abc" emits three byte-wise INS operands (org =
    /// EOF as in the C++ `put(INS, 1, lcOrg, lcNew, …)`) plus the final ESC;
    /// ret 0.
    #[test]
    fn pure_insert() {
        let (ret, ops) = run(Vec::new(), b"abc".to_vec());
        assert_eq!(ret, 0);
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
        assert_eq!(ret, 0);
        assert_eq!(ops, vec![op(X, 0, 0, 0)]);
    }

    /// Brief step-1 test: "hello world" ×3 vs the XYZ modification. The
    /// engine ops must reproduce the reference patch bytes `a7 a3 11 a7 a6 58
    /// 59 5a 6c 6f 20 77 6f 72 6c 64 20 68 65 6c 6c 6f 0a` produced by the
    /// C++ `jdiff` + `JOutBin` oracle for this fixture (org = new = 36 bytes,
    /// `hello world ` ×3 with the final space turned into a newline and the
    /// second word's `wor` replaced by `XYZ`).
    #[test]
    fn modify_run() {
        let org = b"hello world hello world hello world\n".to_vec();
        let new = b"hello world hello XYZlo world hello\n".to_vec();
        let (ret, ops) = run(org, new);
        assert_eq!(ret, 0);

        // Exact C++-pinned operand sequence (oracle2, fixture 3, scn=1).
        assert_eq!(
            ops,
            vec![
                op(E, 1, 104, 104), // 'h' ×4 byte-mode EQL
                op(E, 1, 101, 101), // 'e'
                op(E, 1, 108, 108), // 'l'
                op(E, 1, 108, 108), // 'l' -> length mode granted
                op(E, 14, 0, 0),    // uf_put_eql flush (positions 4..18)
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
                0xa7, 0xa3, 0x11, 0xa7, 0xa6, 0x58, 0x59, 0x5a, 0x6c, 0x6f, 0x20, 0x77, 0x6f, 0x72,
                0x6c, 0x64, 0x20, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x0a,
            ]
        );
    }

    /// Brief step-1 test: a duplicated block produces a BKT (backtrack)
    /// operand. org = 1000 LCG bytes; new = org[0..800] + org[300..800] +
    /// org[800..1000], so the block org[300..800] appears twice. The whole
    /// operand sequence is pinned against the C++ oracle (oracle3, fixture 1).
    #[test]
    fn repeated_block_produces_bkt() {
        let org = lcg_bytes(1, 1000);
        let mut new = Vec::with_capacity(1500);
        new.extend_from_slice(&org[0..800]);
        new.extend_from_slice(&org[300..800]);
        new.extend_from_slice(&org[800..1000]);
        let (ret, ops) = run(org, new);
        assert_eq!(ret, 0);
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133),
                op(E, 1, 1, 1),
                op(E, 1, 58, 58),
                op(E, 796, 0, 0),
                op(BKT, 500, 0, 0), // backtrack 500 bytes on the original file
                op(E, 1, 190, 190),
                op(E, 1, 145, 145),
                op(E, 1, 102, 102),
                op(E, 1, 126, 126),
                op(E, 696, 0, 0),
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Brief step-1 test: removing 100 bytes at offset 500 produces a DEL of
    /// exactly 100 right after the first 500 equal bytes. org = 1000 LCG
    /// bytes; new = org minus [500..600). Whole sequence pinned against the
    /// C++ oracle (oracle3, fixture 0).
    #[test]
    fn small_shift_produces_del_or_ins() {
        let org = lcg_bytes(1, 1000);
        let mut new = Vec::with_capacity(900);
        new.extend_from_slice(&org[0..500]);
        new.extend_from_slice(&org[600..1000]);
        let (ret, ops) = run(org, new);
        assert_eq!(ret, 0);
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133),
                op(E, 1, 1, 1),
                op(E, 1, 58, 58),
                op(E, 496, 0, 0),   // 500 equal bytes total
                op(DEL, 100, 0, 0), // delete the 100 shifted-out bytes
                op(E, 1, 103, 103),
                op(E, 1, 136, 136),
                op(E, 1, 47, 47),
                op(E, 1, 102, 102),
                op(E, 396, 0, 0), // remaining 400 equal bytes
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Brief step-1 test: a read error on the original file propagates as
    /// -EXI_RED. As in the C++ (where `bool lbFnd` swallows the negative
    /// ufFndAhd return, see the module docs), the engine keeps running and
    /// drains the whole new file as INS operands carrying the failing org
    /// value (-8) before returning min(EOF, -8) = -8 (pinned against the C++
    /// oracle, oracle2 fixture 0 with the failing file).
    #[test]
    fn engine_error_propagates() {
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(FailingJFile),
            Box::new(JFileMem::new(b"hello world\n".to_vec())),
            Box::new(rec),
        );
        assert_eq!(jd.jdiff(), -EXI_RED);
        let new = b"hello world\n";
        let expected: Vec<(i32, i64, i32, i32)> = new
            .iter()
            .copied()
            .map(|b| op(I, 1, -EXI_RED, i32::from(b)))
            .chain([op(X, 0, 0, 0)])
            .collect();
        assert_eq!(ops.0.borrow().clone(), expected);
    }

    /// Two separated single-byte edits exercise two full find-ahead rounds,
    /// the prescan-once dispatch (src_scn 1 → 2) and the src_scn == 2
    /// lookahead budget branch. Sequence pinned against the C++ oracle
    /// (oracle5); the LCG bytes themselves double as a fixture cross-check.
    #[test]
    fn two_edits_two_find_ahead_rounds() {
        let mut org = lcg_bytes(1, 1000);
        let mut new = org.clone();
        new[250] = org[250] ^ 0xff;
        new[750] = org[750] ^ 0xff;
        org[0] = 89; // guard the LCG against accidental drift (see oracle)
        let (ret, ops) = run(org, new);
        assert_eq!(ret, 0);
        assert_eq!(
            ops,
            vec![
                op(E, 1, 89, 89),
                op(E, 1, 133, 133),
                op(E, 1, 1, 1),
                op(E, 1, 58, 58),
                op(E, 246, 0, 0),
                op(M, 1, 60, 195), // edit 1: 60 ^ 0xff = 195
                op(E, 1, 196, 196),
                op(E, 1, 187, 187),
                op(E, 1, 203, 203),
                op(E, 1, 53, 53),
                op(E, 495, 0, 0),
                op(M, 1, 124, 131), // edit 2: 124 ^ 0xff = 131
                op(E, 1, 38, 38),
                op(E, 1, 237, 237),
                op(E, 1, 37, 37),
                op(E, 1, 192, 192),
                op(E, 245, 0, 0),
                op(X, 0, 0, 0),
            ]
        );
    }

    /// Statistics and false-hit repairs: for the zero-block fixture the
    /// hashtable answers 115 lookups with a key hit, 2 of which are false
    /// (all-zero 32-byte windows share hash key 0) and get compare-repaired —
    /// exactly the C++ oracle's `HITS 115 RPR 2` (built with the 32-bit
    /// `hkey` of the oracle build, spec §2). `hsh_err` stays 0 in the
    /// release build, like the C++.
    #[test]
    fn stats_and_hash_repairs() {
        let _rpr = hsh_rpr_guard();

        // org: 400 LCG bytes with a 100-byte zero block at [200..300);
        // new: the zero block moved 50 bytes to the right (50 bytes taken
        // from org[300..350] inserted before it).
        let org = zero_block_fixture();
        let new = zero_block_moved_fixture(&org);

        let before = HSH_RPR.load(Ordering::Relaxed);
        let ops = Ops::default();
        let rec = RecordingOut::new(ops.clone());
        let mut jd = engine(
            Box::new(JFileMem::new(org)),
            Box::new(JFileMem::new(new)),
            Box::new(rec),
        );
        assert_eq!(jd.jdiff(), 0);
        assert_eq!(jd.hash().hash_hits(), 115);
        assert_eq!(jd.hsh_err(), 0);
        assert_eq!(HSH_RPR.load(Ordering::Relaxed) - before, 2);
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
