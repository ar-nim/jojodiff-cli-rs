//! `JOutBin`: binary patch-file writer, 1:1 port of C++ `src/JOutBin.cpp`
//! (spec §11.1, wire format §3).
//!
//! The output has the following format (`JOutBin.cpp:32-52`):
//!
//! ```text
//! <esc> <opcode> [<length>|<data>]
//! ```
//!
//! where `<data>` is a series of data bytes ended by the next `<esc><opcode>`
//! sequence; an `<esc><opcode>` sequence occurring within the data is prefixed
//! with an additional `<esc>`.

use std::io::Write;

use super::{JOut, OutStats};
use crate::defs::{BKT, DEL, EQL, ESC, INS, MOD};

/// Binary patch writer (`JOutBin`, `JOutBin.cpp:26-225`), generic over any
/// `std::io::Write` sink (the C++ writes to a `FILE *`).
pub struct JOutBin<W: Write> {
    out: W,
    stats: OutStats,
    /// Current operand: INS, MOD, EQL or DEL. ESC means none
    /// (`miOprCur`, `JOutBin.h:51`).
    opr_cur: i32,
    /// Number of pending equal bytes (`mzEqlCnt`, `JOutBin.h:52`).
    eql_cnt: i64,
    /// First four equal bytes (`miEqlBuf`, `JOutBin.h:53`).
    eql_buf: [i32; 4],
    /// Pending escape character in data stream? (`mbOutEsc`, `JOutBin.h:54`).
    out_esc: bool,
}

impl<W: Write> JOutBin<W> {
    /// `JOutBin::JOutBin` (`JOutBin.cpp:26`): `miOprCur = ESC`, `mzEqlCnt = 0`,
    /// `mbOutEsc = false`, all statistics zeroed (`JOut` ctor, `JOut.h:61-65`).
    pub fn new(out: W) -> Self {
        JOutBin {
            out,
            stats: OutStats::default(),
            opr_cur: ESC,
            eql_cnt: 0,
            eql_buf: [0; 4],
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

    /// `JOutBin::ufPutLen` (`JOutBin.cpp:64-103`): outputs a length as
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

    /// `JOutBin::ufPutOpr` (`JOutBin.cpp:109-123`): outputs a new opcode and
    /// closes the previous data stream.
    fn put_opr(&mut self, opr: i32) {
        if self.out_esc {
            self.raw(ESC as u8);
            self.raw(ESC as u8);
            self.out_esc = false;
            self.stats.esc += 1;
            self.stats.dta += 1;
        }

        if opr != ESC {
            self.raw(ESC as u8);
            self.raw(opr as u8);
            self.stats.ctl += 2;
        }
    }

    /// `JOutBin::ufPutByt` (`JOutBin.cpp:129-147`): outputs a byte, prefixing
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
    /// `JOutBin::put` (`JOutBin.cpp:152-225`): binary output function for
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
        /* Output a pending EQL operand (if more than 4 equal bytes) */
        if opr != EQL && self.eql_cnt > 0 {
            if self.eql_cnt > 4 || (self.opr_cur != MOD && opr != MOD) {
                // more than 4 equal bytes => output as EQL
                self.opr_cur = EQL;
                self.put_opr(EQL);
                self.put_len(self.eql_cnt);

                self.stats.eql += self.eql_cnt;
            } else {
                // less than 4 equal bytes => output as MOD
                if self.opr_cur != MOD {
                    self.opr_cur = MOD;
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
                self.opr_cur = ESC;
            }

            MOD | INS => {
                if self.opr_cur != opr {
                    self.opr_cur = opr;
                    self.put_opr(opr);
                }
                self.put_byte(new);
            }

            DEL => {
                self.put_opr(DEL);
                self.put_len(len);

                self.opr_cur = DEL;
                self.stats.del += len;
            }

            BKT => {
                self.put_opr(BKT);
                self.put_len(len);

                self.opr_cur = BKT;
                self.stats.bkt += len;
            }

            EQL => {
                if self.eql_cnt < 4 {
                    self.eql_buf[self.eql_cnt as usize] = org;
                    self.eql_cnt += 1;
                    return self.eql_cnt >= 4;
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
        // MOD data 0x00,0xA7,0x00: the 0xA7 (ESC) byte is held pending and
        // flushed as plain data when the next byte (0x00 < BKT) arrives — no
        // escape twin (JOutBin.cpp:129-147). Note: the brief's sketch
        // "00 A7 A7 00" contradicts the C++, which emits "00 A7 00".
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x00),
            (MOD, 1, 0x00, 0xA7),
            (MOD, 1, 0x00, 0x00),
        ]);
        assert_eq!(out, [0xA7, 0xA6, 0x00, 0xA7, 0x00]);
        assert_eq!((st.dta, st.esc, st.ctl), (3, 0, 2));

        // MOD data 0xA7,0xA6: ESC followed by an opcode-range byte
        // (BKT..=ESC) is doubled (JOutBin.cpp:133-137).
        let (out, st) = run_stats(&[(MOD, 1, 0x00, 0xA7), (MOD, 1, 0x00, 0xA6)]);
        assert_eq!(out, [0xA7, 0xA6, 0xA7, 0xA7, 0xA6]);
        assert_eq!((st.dta, st.esc, st.ctl), (2, 1, 2));

        // A pending data-ESC at an operand switch is flushed as data plus its
        // escape twin by put_opr (JOutBin.cpp:110-116).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x00),
            (MOD, 1, 0x00, 0xA7),
            (INS, 1, 0x00, 0x01),
        ]);
        assert_eq!(out, [0xA7, 0xA6, 0x00, 0xA7, 0xA7, 0xA7, 0xA5, 0x01]);
        assert_eq!((st.dta, st.esc, st.ctl), (3, 1, 4));

        // … and likewise by the engine's final put(ESC) at end of stream.
        let (out, st) = run_stats(&[(MOD, 1, 0x00, 0x00), (MOD, 1, 0x00, 0xA7)]);
        assert_eq!(out, [0xA7, 0xA6, 0x00, 0xA7, 0xA7]);
        assert_eq!((st.dta, st.esc, st.ctl), (2, 1, 2));
    }

    #[test]
    fn eql_short_run_becomes_mod() {
        // Prev MOD, 3 equal bytes, next MOD: the buffered equals are emitted
        // as MOD data — no EQL opcode (JOutBin.cpp:169-177).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x11),
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (EQL, 1, 0x44, 0x44),
            (MOD, 1, 0x00, 0x55),
        ]);
        assert_eq!(out, [0xA7, 0xA6, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert!(!out.contains(&(EQL as u8)), "no EQL opcode in {out:?}");
        assert_eq!((st.dta, st.ctl, st.eql), (5, 2, 0));

        // Boundary: exactly 4 pending equals with MOD on both sides still
        // become MOD data (flush condition is `eql_cnt > 4`).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x11),
            (EQL, 1, 0x22, 0x22),
            (EQL, 1, 0x33, 0x33),
            (EQL, 1, 0x44, 0x44),
            (EQL, 1, 0x55, 0x55),
            (MOD, 1, 0x00, 0x66),
        ]);
        assert_eq!(out, [0xA7, 0xA6, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        assert!(!out.contains(&(EQL as u8)), "no EQL opcode in {out:?}");
        assert_eq!((st.dta, st.eql), (6, 0));
    }

    #[test]
    fn eql_long_run_emits_opcode() {
        // Engine driving for a 10-byte equal run (JDiff.cpp:150-171):
        // put(EQL, 1, …) byte-wise until it returns true (after the 4th byte),
        // the remaining 6 accumulate in lzEql and are flushed as a single
        // put(EQL, 6, 0, 0, …) by ufPutEql (JDiff.cpp:258-267); the final
        // put(ESC) emits ESC EQL <put_len(10)>.
        let (out, st) = run_stats(&[
            (EQL, 1, 0xF0, 0xF0),
            (EQL, 1, 0xF1, 0xF1),
            (EQL, 1, 0xF2, 0xF2),
            (EQL, 1, 0xF3, 0xF3),
            (EQL, 6, 0, 0),
        ]);
        assert_eq!(out, [0xA7, 0xA3, 0x09]);
        assert_eq!(st.eql, 10);
        assert_eq!((st.ctl, st.dta), (3, 0));
    }

    #[test]
    fn ins_after_mod_switches_opcode_once() {
        // The opcode is emitted only when the operand changes
        // (JOutBin.cpp:188-195).
        let (out, st) = run_stats(&[
            (MOD, 1, 0x00, 0x01),
            (MOD, 1, 0x00, 0x02),
            (INS, 1, 0x00, 0x03),
            (INS, 1, 0x00, 0x04),
        ]);
        assert_eq!(out, [0xA7, 0xA6, 0x01, 0x02, 0xA7, 0xA5, 0x03, 0x04]);
        assert_eq!(
            out.iter().filter(|&&b| b == ESC as u8).count(),
            2,
            "exactly one opcode switch"
        );
        assert_eq!((st.dta, st.ctl), (4, 4));
    }

    #[test]
    fn stats_counters_match_cxx_rules() {
        // Scripted sequence exercising every counter, driven exactly like
        // JDiff.cpp: byte-wise MOD/INS/EQL, ufPutEql length flushes, DEL/BKT
        // with lengths, final ESC flush.
        let (out, st) = run_stats(&[
            (INS, 1, 0x00, 0x41), // ESC INS 41
            (EQL, 1, 0x42, 0x42), // buffered
            (EQL, 1, 0x43, 0x43), // buffered
            (EQL, 1, 0x44, 0x44), // buffered
            (INS, 1, 0x00, 0x45), // 3 equals + INS flush (opr_cur=INS≠MOD, opr=INS≠MOD) → EQL opcode
            (DEL, 300, 0, 0),     // ESC DEL 252,2F (300 = 253 + 47)
            (BKT, 5, 0, 0),       // ESC BKT 04
            (INS, 1, 0x00, 0x46), // ESC INS 46
            (EQL, 1, 0x47, 0x47), // buffered (4th returns true)
            (EQL, 1, 0x48, 0x48),
            (EQL, 1, 0x49, 0x49),
            (EQL, 1, 0x4A, 0x4A),
            (MOD, 1, 0x00, 0x4B), // 4 equals + MOD after INS (opr == MOD) → MOD data branch
        ]);
        assert_eq!(
            out,
            [
                0xA7, 0xA5, 0x41, // ESC INS 41
                0xA7, 0xA3, 0x02, // ESC EQL 2
                0xA7, 0xA5, 0x45, // ESC INS 45
                0xA7, 0xA4, 0xFC, 0x2F, // ESC DEL 300
                0xA7, 0xA2, 0x04, // ESC BKT 5
                0xA7, 0xA5, 0x46, // ESC INS 46
                0xA7, 0xA6, 0x47, 0x48, 0x49, 0x4A, 0x4B, // ESC MOD 47..4B
            ]
        );
        assert_eq!(
            st,
            OutStats {
                dta: 8,  // 41 45 46 47 48 49 4A 4B
                ctl: 18, // 7 opcodes (14) + lengths 1+2+1 (4)
                del: 300,
                bkt: 5,
                esc: 0,
                eql: 3, // 42 43 44 went out via the EQL record
            }
        );
    }
}
