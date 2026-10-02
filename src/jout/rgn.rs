//! `JOutRgn`: region listing writer (`-r`), 1:1 port of C++
//! `src/JOutRgn.cpp` (spec §18.F; 0.8.1 output this under `-lr`).
//!
//! Consecutive puts of the same operand are accumulated into a count; when
//! the operand changes — or the engine's final `put(ESC, …)` arrives — one
//! line is printed for the *previous* operand:
//!
//! ```text
//! MOD {pos_org-cnt:>12} {pos_new-cnt:>12} MOD {cnt}
//! INS {pos_org:>12}     {pos_new-cnt:>12} INS {cnt}
//! DEL {pos_org-cnt:>12} {pos_new:>12}     DEL {cnt}
//! BKT {pos_org+cnt:>12} {pos_new:>12}     BKT {cnt}
//! EQL {pos_org-cnt:>12} {pos_new-cnt:>12} EQL {cnt}
//! ```
//!
//! 0.8.5 statistics quirks, ported as written (spec §21.14): the MOD flush's
//! `ctl += 2` sits behind a dead `if (siOprCur == INS)` (`JOutRgn.cpp:50-57`);
//! an EQL region of `cnt <= MINEQL` counts as dta instead of ctl+eql
//! (`:79-85`); and DEL/BKT add `2+ufPutLen` where ufPutLen returns
//! 1/2/3/**4**/**8** (`:120-139`, inconsistent with the real 5/9 encoding).
//!
//! Always returns `true` (length mode: the engine sends lengths instead of
//! byte-wise details).

use std::io::Write;

use super::{JOut, OutStats};
use crate::defs::{BKT, DEL, EQL, ESC, INS, MINEQL, MOD};

/// Region listing writer (`JOutRgn`, `JOutRgn.cpp:30-141`), generic over any
/// `std::io::Write` sink (the C++ writes to a `FILE *`).
pub struct JOutRgn<W: Write> {
    out: W,
    stats: OutStats,
    /// Operand currently being accumulated (`siOprCur`, `JOutRgn.cpp:40`).
    /// The C++ original is a function-local `static` shared by every
    /// instance in the process; the port keeps it per-instance, which is
    /// indistinguishable for the single-writer CLI.
    opr_cur: i32,
    /// Accumulated length of the current operand (`szOprCnt`,
    /// `JOutRgn.cpp:41`).
    opr_cnt: i64,
}

impl<W: Write> JOutRgn<W> {
    /// `JOutRgn::JOutRgn` (`JOutRgn.cpp:26`): `siOprCur = ESC`,
    /// `szOprCnt = 0`, all statistics zeroed (`JOut` ctor, `JOut.h:61-65`).
    pub fn new(out: W) -> Self {
        JOutRgn {
            out,
            stats: OutStats::default(),
            opr_cur: ESC,
            opr_cnt: 0,
        }
    }

    /// Returns the underlying writer, discarding the statistics (the
    /// Rust-side counterpart of simply dropping the C++ object; needed to
    /// flush/drop buffered writers such as `BufWriter`).
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Writes a formatted fragment, panicking on I/O errors (the C++ never
    /// checks `fprintf`'s return value; the port refuses to silently
    /// truncate a listing).
    fn out(&mut self, args: std::fmt::Arguments<'_>) {
        self.out.write_fmt(args).expect("JOutRgn: write error");
    }

    /// `JOutRgn::ufPutLen` (`JOutRgn.cpp:120-139`): length-encoding size.
    /// Unlike the real encoding tiers (1/2/3/5/9, cf. `JOutAsc::ufPutSze`)
    /// the 32/64-bit tiers here return **4** and **8** — an upstream
    /// inconsistency ported as written (spec §21.14).
    fn put_len(len: i64) -> i64 {
        if len <= 252 {
            1
        } else if len <= 508 {
            2
        } else if len <= 0xffff {
            3
        } else if len <= 0xffff_ffff {
            4
        } else {
            8
        }
    }
}

impl<W: Write> JOut for JOutRgn<W> {
    /// `JOutRgn::put` (`JOutRgn.cpp:32-95`): region output; always returns
    /// `true` ("we never need details").
    fn put(&mut self, opr: i32, len: i64, _org: i32, new: i32, pos_org: i64, pos_new: i64) -> bool {
        /* write output when operation code changes */
        if opr != self.opr_cur {
            let cnt = self.opr_cnt;
            match self.opr_cur {
                MOD => {
                    /* P8zd " " P8zd " MOD %"PRIzd"\n" */
                    // a MOD sequence is only needed after an INS sequence —
                    // dead here (siOprCur is MOD in this arm), ported as
                    // written (JOutRgn.cpp:50-57; spec §21.14)
                    if self.opr_cur == INS {
                        self.stats.ctl += 2;
                    }
                    self.stats.dta += cnt;
                    let (col_org, col_new) = (pos_org - cnt, pos_new - cnt);
                    self.out(format_args!("{col_org:>12} {col_new:>12} MOD {cnt}\n"));
                }

                INS => {
                    /* P8zd " " P8zd " INS %"PRIzd"\n" */
                    self.stats.ctl += 2;
                    self.stats.dta += cnt;
                    let col_new = pos_new - cnt;
                    self.out(format_args!("{pos_org:>12} {col_new:>12} INS {cnt}\n"));
                }

                DEL => {
                    /* P8zd " " P8zd " DEL %"PRIzd"\n" */
                    self.stats.ctl += 2 + Self::put_len(cnt);
                    self.stats.del += cnt;
                    let col_org = pos_org - cnt;
                    self.out(format_args!("{col_org:>12} {pos_new:>12} DEL {cnt}\n"));
                }

                BKT => {
                    /* P8zd " " P8zd " BKT %"PRIzd"\n" */
                    self.stats.ctl += 2 + Self::put_len(cnt);
                    self.stats.bkt += cnt;
                    let col_org = pos_org + cnt;
                    self.out(format_args!("{col_org:>12} {pos_new:>12} BKT {cnt}\n"));
                }

                EQL => {
                    /* P8zd " " P8zd " EQL %"PRIzd"\n" */
                    if cnt <= i64::from(MINEQL) {
                        self.stats.dta += cnt;
                    } else {
                        self.stats.ctl += 2 + Self::put_len(cnt);
                        self.stats.eql += cnt;
                    }
                    let (col_org, col_new) = (pos_org - cnt, pos_new - cnt);
                    self.out(format_args!("{col_org:>12} {col_new:>12} EQL {cnt}\n"));
                }

                /* ESC: nothing pending, nothing to flush */
                _ => {}
            }

            self.opr_cur = opr;
            self.opr_cnt = 0;
        }

        /* accumulate operation codes; the C++ INS/MOD cases fall through
         * into the DEL/BKT/EQL case (`JOutRgn.cpp:82-93`) */
        match opr {
            INS | MOD => {
                if new == ESC {
                    self.stats.esc += 1;
                }
                self.opr_cnt += len;
            }
            DEL | BKT | EQL => {
                self.opr_cnt += len;
            }
            /* ESC is not accumulated: the pending region was already
             * flushed above and the next region starts from zero */
            _ => {}
        }

        true
    }

    fn stats(&self) -> OutStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `put` with `(opr, len, org, new, pos_org, pos_new)` tuples over
    /// a `Vec<u8>` sink and returns the listing text and the statistics.
    /// `JOutRgn::put` always returns `true`; the driver asserts that.
    fn run(ops: &[(i32, i64, i32, i32, i64, i64)]) -> (String, OutStats) {
        let mut jout = JOutRgn::new(Vec::new());
        for &(opr, len, org, new, pos_org, pos_new) in ops {
            assert!(
                jout.put(opr, len, org, new, pos_org, pos_new),
                "put({opr}) must return true"
            );
        }
        let stats = jout.stats();
        (
            String::from_utf8(jout.into_inner()).expect("ASCII output"),
            stats,
        )
    }

    #[test]
    fn rgn_regions() {
        // Engine-shaped driving (JDiff.cpp): EQL arrives as length-mode
        // ufPutEql flushes positioned at the run start, MOD/INS byte-wise
        // with advancing positions, DEL/BKT as single length puts at the
        // pre-skip position, and the final put(ESC) carries the EOF
        // positions and flushes the last region. Literals verified
        // byte-for-byte against a compiled C++ oracle harness over the
        // vendored JOutRgn.cpp. (The brief's §16 sketch sequence/positions
        // are inconsistent with its own operand list; the C++ wins.)
        let (out, st) = run(&[
            (EQL, 5, 0, 0, 0, 0),       // first put: nothing pending to flush
            (MOD, 1, 0x65, 0x41, 5, 5), // flushes EQL 5 at (5-5, 5-5)
            (MOD, 1, 0x66, 0x42, 6, 6),
            (MOD, 1, 0x67, 0x43, 7, 7),
            (MOD, 1, 0x68, 0x44, 8, 8),
            (MOD, 1, 0x69, 0x45, 9, 9),
            (MOD, 1, 0x6A, 0x46, 10, 10),
            (MOD, 1, 0x6B, 0x47, 11, 11),
            (MOD, 1, 0x6C, 0x48, 12, 12),
            (INS, 1, -1, 0x61, 13, 13), // flushes MOD 8 at (13-8, 13-8)
            (INS, 1, -1, 0x62, 13, 14),
            (INS, 1, -1, 0x63, 13, 15),
            (EQL, 10, 0, 0, 13, 16),       // flushes INS 3 at (13, 16-3)
            (DEL, 100, 0, 0, 23, 26),      // flushes EQL 10 at (23-10, 26-10)
            (BKT, 7, 0, 0, 123, 26),       // flushes DEL 100 at (123-100, 26)
            (EQL, 4, 0, 0, 116, 26),       // flushes BKT 7 at (116+7, 26)
            (MOD, 1, 0x20, 0x20, 120, 30), // flushes EQL 4 at (120-4, 30-4)
            (ESC, 0, 0, 0, 121, 31),       // flushes MOD 1 at (121-1, 31-1)
        ]);
        assert_eq!(
            out,
            concat!(
                "           0            0 EQL 5\n",
                "           5            5 MOD 8\n",
                "          13           13 INS 3\n",
                "          13           16 EQL 10\n",
                "          23           26 DEL 100\n",
                "         123           26 BKT 7\n",
                "         116           26 EQL 4\n",
                "         120           30 MOD 1\n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 12,
                ctl: 17,
                del: 100,
                bkt: 7,
                esc: 0,
                eql: 19
            }
        );
    }

    #[test]
    fn rgn_esc_flush_and_esc_byte_stats() {
        // The initial ESC state emits nothing on the first put; a MOD byte
        // equal to ESC (0xA7) bumps esc while accumulating; the final
        // put(ESC) flushes the pending region without accumulating.
        //
        // 0.8.5 accounting (JOutRgn.cpp:50-57,79-85): the MOD flush's ctl += 2
        // sits behind a dead `if (siOprCur == INS)` (siOprCur is MOD in that
        // arm — ported as written, so MOD flushes never add ctl), and an EQL
        // region of cnt <= MINEQL counts as dta, not eql.
        // Oracle: dta=3 ctl=0 del=0 bkt=0 esc=1 eql=0.
        let (out, st) = run(&[
            (EQL, 1, 0x41, 0x41, 0, 0),
            (MOD, 1, 0x30, 0xA7, 1, 1),
            (MOD, 1, 0x31, 0x32, 2, 2),
            (ESC, 0, 0, 0, 3, 3),
        ]);
        assert_eq!(
            out,
            concat!(
                "           0            0 EQL 1\n",
                "           1            1 MOD 2\n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 3,
                ctl: 0,
                del: 0,
                bkt: 0,
                esc: 1,
                eql: 0
            }
        );
    }

    #[test]
    fn rgn_mineql_boundary_and_put_len_quirk() {
        // EQL split on MINEQL (JOutRgn.cpp:79-85): cnt <= 2 → dta (no eql,
        // no ctl), cnt > 2 → ctl += 2+put_len and eql += cnt. The MOD flush's
        // ctl += 2 is dead (`siOprCur == INS` inside `case (MOD)`, :50-57 —
        // ported as written). DEL/BKT add 2+ufPutLen with ufPutLen returning
        // 1/2/3/4/8 (JOutRgn.cpp:120-139) — the 4/8 for the 32/64-bit tiers
        // is inconsistent with the real 5/9 encoded bytes and is ported as
        // written (spec §21.14).
        let (out, st) = run(&[
            (EQL, 2, 0, 0, 2, 2),                  // pending (first put: nothing to flush)
            (MOD, 1, 0x41, 0x41, 2, 2),            // flush EQL 2: cnt <= MINEQL → dta += 2
            (EQL, 3, 0, 0, 3, 3),                  // flush MOD 1: dead-if → no ctl; dta += 1
            (INS, 1, -1, 0x61, 6, 6),              // flush EQL 3: ctl += 2+1, eql += 3
            (DEL, 509, 0, 0, 6, 7),                // flush INS 1: ctl += 2, dta += 1
            (BKT, 65_536, 0, 0, 515, 7),           // flush DEL 509: ctl += 2+3
            (INS, 1, -1, 0x62, 65_545, 8),         // flush BKT 65536: ctl += 2+4 (quirk)
            (BKT, 4_294_967_296, 0, 0, 65_545, 8), // flush INS 1: ctl += 2, dta += 1
            (ESC, 0, 0, 0, 65_545, 8),             // flush BKT 2^32: ctl += 2+8 (quirk)
        ]);
        assert_eq!(
            out,
            concat!(
                "           0            0 EQL 2\n",
                "           2            2 MOD 1\n",
                "           3            3 EQL 3\n",
                "           6            6 INS 1\n",
                "           6            7 DEL 509\n",
                "      131081            8 BKT 65536\n",
                "       65545            7 INS 1\n",
                "  4295032841            8 BKT 4294967296\n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 5,  // EQL 2 + MOD 1 + INS 1 + INS 1
                ctl: 28, // 3 + 2 + 5 + 6 + 2 + 10
                del: 509,
                bkt: 4_295_032_832, // 65536 + 2^32
                esc: 0,
                eql: 3, // only the >MINEQL region
            }
        );
    }
}
