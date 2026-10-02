//! `JOutBin`: binary patch-file writer, 1:1 port of C++ `src/JOutBin.cpp`
//! (spec §18.C/§18.F, wire format §18.C).
//!
//! The output has the following format (`JOutBin.cpp:36-52`):
//!
//! ```text
//! <esc> <opcode> [<length>|<data>]
//! ```
//!
//! where `<data>` is a series of data bytes ended by the next `<esc><opcode>`
//! sequence; an `<esc><opcode>` sequence occurring within the data is prefixed
//! with an additional `<esc>`.
//!
//! 0.8.5 changes vs 0.8.1 (spec §18.C): the constructor seeds the current
//! operand with MOD and `ufPutOpr` emits `ESC <opr>` only when
//! `opr != MOD || opr_cur == INS` — a MOD run at patch start or following
//! EQL/DEL/BKT is written **without** the `ESC MOD` prefix (implicit MOD);
//! `ESC MOD` is still emitted when switching INS→MOD. The pending-equal
//! threshold drops from 4 to `MINEQL` = 2, so runs of ≥3 equal bytes become
//! `ESC EQL <len>`.

use std::io::Write;

use super::{JOut, OutStats};
use crate::defs::{BKT, DEL, EQL, ESC, INS, MOD, MINEQL};

/// Binary patch writer (`JOutBin`, `JOutBin.cpp:27-232`), generic over any
/// `std::io::Write` sink (the C++ writes to a `FILE *`).
pub struct JOutBin<W: Write> {
    out: W,
    stats: OutStats,
    /// Current operand: INS, MOD, EQL or DEL. ESC means none
    /// (`miOprCur`, `JOutBin.h:52`). The constructor seeds MOD
    /// (`JOutBin.cpp:27`; 0.8.1 seeded ESC).
    opr_cur: i32,
    /// Number of pending equal bytes (`mzEqlCnt`, `JOutBin.h:53`).
    eql_cnt: i64,
    /// First `MINEQL` equal bytes (`miEqlBuf[MINEQL]`, `JOutBin.h:54`).
    eql_buf: [i32; MINEQL as usize],
    /// Pending escape character in data stream? (`mbOutEsc`, `JOutBin.h:55`).
    out_esc: bool,
}

impl<W: Write> JOutBin<W> {
    /// `JOutBin::JOutBin` (`JOutBin.cpp:27`): `miOprCur = MOD`, `mzEqlCnt = 0`,
    /// `mbOutEsc = false`, all statistics zeroed (`JOut` ctor, `JOut.h:61-65`).
    pub fn new(out: W) -> Self {
        JOutBin {
            out,
            stats: OutStats::default(),
            opr_cur: MOD,
            eql_cnt: 0,
            eql_buf: [0; MINEQL as usize],
            out_esc: false,
        }
    }

    /// Returns the underlying writer, discarding the statistics (the
    /// Rust-side counterpart of simply dropping the C++ object; needed to
    /// flush/drop buffered writers such as `BufWriter`).
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Outputs one byte to the sink. The C++ `putc(…, mpFilOut)` return value
    /// is never checked (`JOutBin.cpp`); the Rust port panics on a write
    /// error instead of silently producing a truncated patch file.
    fn raw(&mut self, byt: u8) {
        self.out.write_all(&[byt]).expect("JOutBin: write error");
    }

    /// `JOutBin::ufPutLen` (`JOutBin.cpp:65-104`): outputs a length as
    ///
    /// ```text
    /// byte1  following  formula               if number is
    /// -----  ---------  --------------------  --------------------
    /// 0-251             1-252                 between 1 and 252
    /// 252    x          253 + x               between 253 and 508
    /// 253    xx         253 + 256 + xx        a 16-bit number
    /// 254    xxxx       253 + 256 + xxxx      a 32-bit number
    /// 255    xxxxxxxx   253 + 256 + xxxxxxxx  a 64-bit number
    /// ```
    ///
    /// The 9-byte tier is always enabled: the oracle build defines
    /// `JDIFF_LARGEFILE` (`JOutBin.cpp:77,89-102`).
    fn put_len(&mut self, len: i64) {
        if len <= 252 {
            self.raw((len - 1) as u8);
            self.stats.ctl += 1;
        } else if len <= 508 {
            self.raw(252);
            self.raw((len - 253) as u8);
            self.stats.ctl += 2;
        } else if len <= 0xffff {
            self.raw(253);
            self.raw((len >> 8) as u8);
            self.raw(len as u8);
            self.stats.ctl += 3;
        } else if len <= 0xffff_ffff {
            self.raw(254);
            self.raw((len >> 24) as u8);
            self.raw((len >> 16) as u8);
            self.raw((len >> 8) as u8);
            self.raw(len as u8);
            self.stats.ctl += 5;
        } else {
            self.raw(255);
            self.raw((len >> 56) as u8);
            self.raw((len >> 48) as u8);
            self.raw((len >> 40) as u8);
            self.raw((len >> 32) as u8);
            self.raw((len >> 24) as u8);
            self.raw((len >> 16) as u8);
            self.raw((len >> 8) as u8);
            self.raw(len as u8);
            self.stats.ctl += 9;
        }
    }

    /// `JOutBin::ufPutOpr` (`JOutBin.cpp:110-130`): outputs a new opcode and
    /// closes the previous data stream. 0.8.5: the `ESC <opr>` pair is only
    /// emitted when `opr != MOD || opr_cur == INS` — no MOD is needed after
    /// an EQL, BKT or DEL (implicit MOD) — and `opr_cur` is assigned here for
    /// every call.
    fn put_opr(&mut self, opr: i32) {
        if self.out_esc {
            self.raw(ESC as u8);
            self.raw(ESC as u8);
            self.out_esc = false;
            self.stats.esc += 1;
            self.stats.dta += 1;
        }

        if opr != ESC {
            // No need to output a MOD after an EQL, BKT or DEL
            if opr != MOD || self.opr_cur == INS {
                self.raw(ESC as u8);
                self.raw(opr as u8);
                self.stats.ctl += 2;
            }
        }
        self.opr_cur = opr;
    }

    /// `JOutBin::ufPutByt` (`JOutBin.cpp:136-160`): outputs a byte, prefixing
    /// a data sequence `<esc><opcode>` with an additional `<esc>` byte.
    fn put_byte(&mut self, byt: i32) {
        if self.out_esc {
            self.out_esc = false;
            if (BKT..=ESC).contains(&byt) {
                /* output an additional <esc> byte */
                self.raw(ESC as u8);
                self.stats.esc += 1;
            }
            self.raw(ESC as u8);
            self.stats.dta += 1;
        }
        if byt == ESC {
            self.out_esc = true;
        } else {
            self.raw(byt as u8);
            self.stats.dta += 1;
        }
    }
}

impl<W: Write> JOut for JOutBin<W> {
    /// `JOutBin::put` (`JOutBin.cpp:165-232`): binary output function for
    /// generating patch files.
    fn put(
        &mut self,
        opr: i32,
        len: i64,
        org: i32,
        new: i32,
        _pos_org: i64,
        _pos_new: i64,
    ) -> bool {
        /* Output a pending EQL operand (if MINEQL or more equal bytes) */
        if opr != EQL && self.eql_cnt > 0 {
            if self.eql_cnt > i64::from(MINEQL) || (self.opr_cur != MOD && opr != MOD) {
                // as of 3 equal bytes => output as EQL (ESC EQL <cnt>)
                self.put_opr(EQL);
                self.put_len(self.eql_cnt);

                self.stats.eql += self.eql_cnt;
            } else {
                // less than 3 equal bytes => output as MOD
                if self.opr_cur != MOD {
                    self.put_opr(MOD);
                }
                for li_cnt in 0..self.eql_cnt {
                    self.put_byte(self.eql_buf[li_cnt as usize]);
                }
            }
            self.eql_cnt = 0;
        }

        /* Handle current operand */
        match opr {
            ESC => {
                /* before closing the output */
                self.put_opr(ESC);
            }

            MOD | INS => {
                if self.opr_cur != opr {
                    self.put_opr(opr);
                }
                self.put_byte(new);
            }

            DEL => {
                self.put_opr(DEL);
                self.put_len(len);

                self.stats.del += len;
            }

            BKT => {
                self.put_opr(BKT);
                self.put_len(len);

                self.stats.bkt += len;
            }

            EQL => {
                if self.eql_cnt < i64::from(MINEQL) {
                    self.eql_buf[self.eql_cnt as usize] = org;
                    self.eql_cnt += 1;
                    return self.eql_cnt >= i64::from(MINEQL);
                } else {
                    self.eql_cnt += len;
                    return true;
                }
            }

            /* The engine only ever sends ESC/MOD/INS/DEL/BKT/EQL; the C++
             * switch has no default and falls through to `return false`. */
            _ => {}
        }

        false
    }

    fn stats(&self) -> OutStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `put` with `(opr, len, org, new)` tuples over a `Vec<u8>` sink,
    /// followed by the engine's end-of-stream flush `put(ESC, 0, 0, 0, …)`
    /// (`JDiff.cpp:197/249`), and returns the patch bytes and the statistics.
    fn run_stats(ops: &[(i32, i64, i32, i32)]) -> (Vec<u8>, OutStats) {
        let mut jout = JOutBin::new(Vec::new());
        for &(opr, len, org, new) in ops {
            jout.put(opr, len, org, new, 0, 0);
        }
        jout.put(ESC, 0, 0, 0, 0, 0);
        let stats = jout.stats();
        (jout.into_inner(), stats)
    }

    #[test]
    fn length_codec_tiers() {
        // put_len via public DEL: put(DEL, len, …) = ESC DEL <put_len(len)>.
        // Golden vectors per spec §3 / JOutBin.cpp:64-103; these exact bytes
        // are re-asserted against the C++ oracle in Task 12.
        const CASES: &[(i64, &[u8], i64)] = &[
            // (len, put_len bytes, ctl = 2 opcode bytes + length bytes)
            (1, &[0x00], 3),
            (252, &[0xFB], 3),
            (253, &[0xFC, 0x00], 4),
            (508, &[0xFC, 0xFF], 4),
            (509, &[0xFD, 0x01, 0xFD], 5),
            (65_535, &[0xFD, 0xFF, 0xFF], 5),
            (65_536, &[0xFE, 0x00, 0x01, 0x00, 0x00], 7),
            // Brief sketch says [FF,00,00,00,00,01,00,00,00]; the C++ oracle
            // (JOutBin.cpp:89-102, MSB-first) and the jpatch ufGetInt decoder
            // both put bit 32 into the 4th length byte.
            (
                0x1_0000_0000,
                &[0xFF, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
                11,
            ),
        ];
        for &(len, bytes, ctl) in CASES {
            let mut jout = JOutBin::new(Vec::new());
            assert!(!jout.put(DEL, len, 0, 0, 0, 0), "put(DEL, {len}) return");
            let st = jout.stats();
            let mut expected = vec![ESC as u8, DEL as u8];
            expected.extend_from_slice(bytes);
            assert_eq!(jout.into_inner(), expected, "put_len({len}) bytes");
            assert_eq!(st.ctl, ctl, "ctl for len {len}");
            assert_eq!(st.del, len, "del for len {len}");
            assert_eq!(st.dta, 0, "dta for len {len}");
        }
    }

    #[test]
    fn esc_escaping_in_data() {
        // 0.8.5 implicit MOD (spec §18.C): the constructor seeds opr_cur =
        // MOD (JOutBin.cpp:27) and ufPutOpr suppresses the ESC MOD pair
        // unless the previous operand was INS (JOutBin.cpp:121-128), so MOD
        // data at patch start is written without any opcode.
        //
        // MOD data 0x00,0xA7,0x00: the 0xA7 (ESC) byte is held pending and
        // flushed as plain data when the next byte (0x00 < BKT) arrives — no
        // escape twin (JOutBin.cpp:139-159).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x00),
            (MOD, 1, 0x00, 0xA7),
            (MOD, 1, 0x00, 0x00),
        ]);
        assert_eq!(out, [0x00, 0xA7, 0x00]);
        assert_eq!((st.dta, st.esc, st.ctl), (3, 0, 0));

        // MOD data 0xA7,0xA6: ESC followed by an opcode-range byte
        // (BKT..=ESC) is doubled (JOutBin.cpp:141-149).
        let (out, st) = run_stats(&[(MOD, 1, 0x00, 0xA7), (MOD, 1, 0x00, 0xA6)]);
        assert_eq!(out, [0xA7, 0xA7, 0xA6]);
        assert_eq!((st.dta, st.esc, st.ctl), (2, 1, 0));

        // A pending data-ESC at an operand switch is flushed as data plus its
        // escape twin by put_opr (JOutBin.cpp:113-119); the INS switch itself
        // still emits ESC INS.
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x00),
            (MOD, 1, 0x00, 0xA7),
            (INS, 1, 0x00, 0x01),
        ]);
        assert_eq!(out, [0x00, 0xA7, 0xA7, 0xA7, 0xA5, 0x01]);
        assert_eq!((st.dta, st.esc, st.ctl), (3, 1, 2));

        // … and likewise by the engine's final put(ESC) at end of stream.
        let (out, st) = run_stats(&[(MOD, 1, 0x00, 0x00), (MOD, 1, 0x00, 0xA7)]);
        assert_eq!(out, [0x00, 0xA7, 0xA7]);
        assert_eq!((st.dta, st.esc, st.ctl), (2, 1, 0));
    }

    #[test]
    fn eql_threshold_three_is_eql_two_is_mod_data() {
        // ≥3 equal bytes (MINEQL=2) with MOD around them: flush condition
        // `eql_cnt > MINEQL` (JOutBin.cpp:175) emits ESC EQL <cnt>; the
        // following MOD is implicit (no ESC MOD after an EQL).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x11),
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (EQL, 1, 0x44, 0x44),
            (MOD, 1, 0x00, 0x55),
        ]);
        assert_eq!(out, [0x11, 0xA7, 0xA3, 0x02, 0x55]);
        assert_eq!((st.dta, st.ctl, st.eql), (2, 3, 3));

        // Boundary: exactly 2 pending equals with MOD on both sides stay MOD
        // data (`eql_cnt > MINEQL` is false, `opr_cur != MOD` is false).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x11),
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (MOD, 1, 0x00, 0x55),
        ]);
        assert_eq!(out, [0x11, 0x22, 0x33, 0x55]);
        assert!(!out.contains(&(EQL as u8)), "no EQL opcode in {out:?}");
        assert_eq!((st.dta, st.ctl, st.eql), (4, 0, 0));

        // 2 equals with INS on both sides: the second flush clause
        // (`opr_cur != MOD && opr != MOD`, JOutBin.cpp:175) forces a tiny
        // explicit EQL record.
        let (out, st) = run_stats(&[
            (INS, 1, 0x00, 0x11),
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (INS, 1, 0x00, 0x44),
        ]);
        assert_eq!(
            out,
            [0xA7, 0xA5, 0x11, 0xA7, 0xA3, 0x01, 0xA7, 0xA5, 0x44]
        );
        assert_eq!((st.dta, st.ctl, st.eql), (2, 7, 2));

        // Equals at patch start: opr_cur still holds the constructor's MOD
        // seed, so the buffered bytes go out as implicit MOD data.
        let (out, st) = run_stats(&[
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (MOD, 1, 0x00, 0x55),
        ]);
        assert_eq!(out, [0x22, 0x33, 0x55]);
        assert_eq!((st.dta, st.ctl, st.eql), (3, 0, 0));
    }

    #[test]
    fn eql_long_run_emits_opcode() {
        // Engine driving for a 10-byte equal run (JDiff.cpp:193-224):
        // put(EQL, 1, …) byte-wise until it returns true (after the 2nd byte,
        // `eql_cnt >= MINEQL`, JOutBin.cpp:223), the remaining 8 accumulate in
        // lzEql and are flushed as a single put(EQL, 8, 0, 0, …) by flushEql
        // (JDiff.cpp:507-516); the final put(ESC) emits ESC EQL <put_len(10)>.
        let (out, st) = run_stats(&[
            (EQL, 1, 0xF0, 0xF0),
            (EQL, 1, 0xF1, 0xF1),
            (EQL, 8, 0, 0),
        ]);
        assert_eq!(out, [0xA7, 0xA3, 0x09]);
        assert_eq!(st.eql, 10);
        assert_eq!((st.ctl, st.dta), (3, 0));
    }

    #[test]
    fn ins_to_mod_switch_emits_esc_mod() {
        // ESC MOD is still emitted when switching INS→MOD
        // (`aiOpr != MOD || miOprCur == INS`, JOutBin.cpp:123).
        let (out, st) = run_stats(&[
            (INS, 1, 0x00, 0x11),
            (INS, 1, 0x00, 0x12),
            (MOD, 1, 0x00, 0x22),
            (MOD, 1, 0x00, 0x23),
        ]);
        assert_eq!(out, [0xA7, 0xA5, 0x11, 0x12, 0xA7, 0xA6, 0x22, 0x23]);
        assert_eq!(
            out.iter().filter(|&&b| b == ESC as u8).count(),
            2,
            "INS opcode + the INS→MOD switch"
        );
        assert_eq!((st.dta, st.ctl), (4, 4));
    }

    #[test]
    fn mod_after_del_bkt_is_implicit() {
        // A MOD run following DEL/BKT is emitted without the ESC MOD prefix
        // ("No need to output a MOD after an EQL, BKT or DEL",
        // JOutBin.cpp:122-127): the data bytes ride the implicit MOD.
        let (out, st) = run_stats(&[
            (DEL, 5, 0, 0),
            (MOD, 1, 0x00, 0x41),
            (BKT, 3, 0, 0),
            (MOD, 1, 0x00, 0x42),
        ]);
        assert_eq!(out, [0xA7, 0xA4, 0x04, 0x41, 0xA7, 0xA2, 0x02, 0x42]);
        assert_eq!((st.dta, st.ctl, st.del, st.bkt), (2, 6, 5, 3));
    }

    #[test]
    fn ins_after_mod_switches_opcode_once() {
        // The opcode is emitted only when the operand changes; the leading
        // MOD is implicit (constructor seed).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x01),
            (MOD, 1, 0x00, 0x02),
            (INS, 1, 0x00, 0x03),
            (INS, 1, 0x00, 0x04),
        ]);
        assert_eq!(out, [0x01, 0x02, 0xA7, 0xA5, 0x03, 0x04]);
        assert_eq!(
            out.iter().filter(|&&b| b == ESC as u8).count(),
            1,
            "exactly one opcode switch"
        );
        assert_eq!((st.dta, st.ctl), (4, 2));
    }

    #[test]
    fn stats_counters_match_cxx_rules() {
        // Scripted sequence exercising every counter, driven exactly like
        // JDiff.cpp: byte-wise MOD/INS/EQL, flushEql length flushes, DEL/BKT
        // with lengths, final ESC flush. Implicit MOD: neither the leading
        // INS-preceded MOD runs nor the trailing MOD after the EQL record
        // emit an ESC MOD pair (the latter only switches opr_cur).
        let (out, st) = run_stats(&[
            (INS, 1, 0x00, 0x41), // ESC INS 41
            (EQL, 1, 0x42, 0x42), // buffered (cnt 1, false)
            (EQL, 1, 0x43, 0x43), // buffered (cnt 2, true: >= MINEQL)
            (EQL, 1, 0x44, 0x44), // length-mode accumulate (cnt 3)
            (INS, 1, 0x00, 0x45), // 3 equals + INS flush → ESC EQL 3
            (DEL, 300, 0, 0),     // ESC DEL 252,2F (300 = 253 + 47)
            (BKT, 5, 0, 0),       // ESC BKT 04
            (INS, 1, 0x00, 0x46), // ESC INS 46
            (EQL, 1, 0x47, 0x47), // buffered (cnt 1, false)
            (EQL, 1, 0x48, 0x48), // buffered (cnt 2, true)
            (EQL, 1, 0x49, 0x49), // accumulate (cnt 3)
            (EQL, 1, 0x4A, 0x4A), // accumulate (cnt 4)
            (MOD, 1, 0x00, 0x4B), // 4 equals + MOD → ESC EQL 4, then implicit MOD data
        ]);
        assert_eq!(
            out,
            [
                0xA7, 0xA5, 0x41, // ESC INS 41
                0xA7, 0xA3, 0x02, // ESC EQL 3
                0xA7, 0xA5, 0x45, // ESC INS 45
                0xA7, 0xA4, 0xFC, 0x2F, // ESC DEL 300
                0xA7, 0xA2, 0x04, // ESC BKT 5
                0xA7, 0xA5, 0x46, // ESC INS 46
                0xA7, 0xA3, 0x03, // ESC EQL 4
                0x4B,             // implicit MOD data (no ESC MOD)
            ]
        );
        assert_eq!(
            st,
            OutStats {
                dta: 4,  // 41 45 46 4B
                ctl: 19, // 7 opcode pairs (14) + lengths 1+2+1+1 (5)
                del: 300,
                bkt: 5,
                esc: 0,
                eql: 7, // 42 43 44 (3) + 47 48 49 4A (4) went out via EQL records
            }
        );
    }
}
