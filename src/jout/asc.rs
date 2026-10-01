//! `JOutAsc`: ASCII byte-by-byte listing writer (`-l`), 1:1 port of C++
//! `src/JOutAsc.cpp` (spec §11.2).
//!
//! Every `put` prints the two file positions right-aligned to width 12
//! (`P8zd` = `%12lld`, `JDefs.h:84`) followed by one operand line:
//!
//! ```text
//! MOD {org:>3o} {new:>3o} {c(org)}-{c(new)}
//! INS     {new:>3o}  -{c(new)}
//! DEL {len}
//! BKT {len}
//! EQL {org:>3o} {new:>3o} {c(org)}-{c(new)}
//! ```
//!
//! where `{b:>3o}` is the octal value space-padded to width 3 (C `printf`
//! `%3o`) and `c(b)` is the byte as a character when printable (`32..=127`),
//! a space otherwise. `ESC` calls are ignored; the return value is always
//! `false` so the engine keeps sending byte-wise details.

use std::io::Write;

use super::{JOut, OutStats};
use crate::defs::{BKT, DEL, EQL, ESC, INS, MOD};

/// ASCII listing writer (`JOutAsc`, `JOutAsc.cpp:26-126`), generic over any
/// `std::io::Write` sink (the C++ writes to a `FILE *`).
pub struct JOutAsc<W: Write> {
    out: W,
    stats: OutStats,
    /// Previous operand for the ctl-on-change accounting (`liOprCur`,
    /// `JOutAsc.cpp:43`). The C++ original is a function-local `static`
    /// shared by every instance in the process; the port keeps it
    /// per-instance, which is indistinguishable for the single-writer CLI.
    opr_cur: i32,
}

impl<W: Write> JOutAsc<W> {
    /// `JOutAsc::JOutAsc` (`JOutAsc.cpp:26`): `liOprCur = ESC`, all
    /// statistics zeroed (`JOut` ctor, `JOut.h:61-65`).
    pub fn new(out: W) -> Self {
        JOutAsc {
            out,
            stats: OutStats::default(),
            opr_cur: ESC,
        }
    }

    /// Returns the underlying writer, discarding the statistics (the
    /// Rust-side counterpart of simply dropping the C++ object; needed to
    /// flush/drop buffered writers such as `BufWriter`).
    pub fn into_inner(self) -> W {
        self.out
    }

    /// The C `printf` `%c` argument rendering (`JOutAsc.cpp:53-54`): the
    /// byte itself when `32 <= b <= 127`, a space otherwise.
    fn chr(b: i32) -> char {
        if (32..=127).contains(&b) {
            char::from_u32(b as u32).expect("32..=127 is a valid char scalar")
        } else {
            ' '
        }
    }

    /// Length encoding size in the binary format (`JOutAsc::ufPutSze`,
    /// `JOutAsc.cpp:107-126`): 1/2/3/5/9 bytes. The 9-byte tier is always
    /// enabled: the oracle build defines `JDIFF_LARGEFILE`
    /// (`JDefs.h:64-67` via `-D_FILE_OFFSET_BITS=64`).
    fn put_sze(len: i64) -> i64 {
        if len <= 252 {
            1
        } else if len <= 508 {
            2
        } else if len <= 0xffff {
            3
        } else if len <= 0xffff_ffff {
            5
        } else {
            9
        }
    }

    /// Writes a formatted fragment, panicking on I/O errors (the C++ never
    /// checks `fprintf`'s return value; the port refuses to silently
    /// truncate a listing).
    fn out(&mut self, args: std::fmt::Arguments<'_>) {
        self.out.write_fmt(args).expect("JOutAsc: write error");
    }
}

impl<W: Write> JOut for JOutAsc<W> {
    /// `JOutAsc::put` (`JOutAsc.cpp:35-105`): ASCII output function for
    /// visualisation; always returns `false` ("we always want details").
    fn put(&mut self, opr: i32, len: i64, org: i32, new: i32, pos_org: i64, pos_new: i64) -> bool {
        if opr == ESC {
            return false;
        }

        /* P8zd" " P8zd" " (JOutAsc.cpp:47-48) */
        self.out(format_args!("{pos_org:>12} {pos_new:>12} "));

        match opr {
            MOD => {
                /* "MOD %3o %3o %c-%c\n" */
                self.out(format_args!(
                    "MOD {org:>3o} {new:>3o} {}-{}\n",
                    Self::chr(org),
                    Self::chr(new)
                ));

                if self.opr_cur != opr {
                    self.opr_cur = opr;
                    self.stats.ctl += 2;
                }
                if new == ESC {
                    self.stats.esc += 1;
                }
                self.stats.dta += 1;
            }

            INS => {
                /* "INS     %3o  -%c\n" */
                self.out(format_args!("INS     {new:>3o}  -{}\n", Self::chr(new)));

                if self.opr_cur != opr {
                    self.opr_cur = opr;
                    self.stats.ctl += 2;
                }
                if new == ESC {
                    self.stats.esc += 1;
                }
                self.stats.dta += 1;
            }

            DEL => {
                /* "DEL %"PRIzd"\n" */
                self.out(format_args!("DEL {len}\n"));

                self.opr_cur = DEL;
                self.stats.ctl += 2 + Self::put_sze(len);
                self.stats.del += len;
            }

            BKT => {
                /* "BKT %"PRIzd"\n" */
                self.out(format_args!("BKT {len}\n"));

                self.opr_cur = BKT;
                self.stats.ctl += 2 + Self::put_sze(len);
                self.stats.bkt += len;
            }

            EQL => {
                /* "EQL %3o %3o %c-%c\n" */
                self.out(format_args!(
                    "EQL {org:>3o} {new:>3o} {}-{}\n",
                    Self::chr(org),
                    Self::chr(new)
                ));

                if self.opr_cur != opr {
                    self.opr_cur = opr;
                    self.stats.ctl += 2 + 4; // 4=approx length uf ufPutLen
                }
                self.stats.eql += 1;
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

    /// Drives `put` with `(opr, len, org, new, pos_org, pos_new)` tuples over
    /// a `Vec<u8>` sink and returns the listing text and the statistics.
    /// `JOutAsc::put` always returns `false`; the driver asserts that.
    fn run(ops: &[(i32, i64, i32, i32, i64, i64)]) -> (String, OutStats) {
        let mut jout = JOutAsc::new(Vec::new());
        for &(opr, len, org, new, pos_org, pos_new) in ops {
            assert!(
                !jout.put(opr, len, org, new, pos_org, pos_new),
                "put({opr}) must return false"
            );
        }
        let stats = jout.stats();
        (
            String::from_utf8(jout.into_inner()).expect("ASCII output"),
            stats,
        )
    }

    #[test]
    fn asc_lines() {
        // Brief Step 1 script; literals verified byte-for-byte against a
        // compiled C++ oracle harness over the vendored JOutAsc.cpp
        // (P8zd = "%12lld", JDefs.h:84). The brief's INS sketch
        // "INS     250" is wrong: 0xA7 = 167 decimal = 0o247. Byte 0xA7 is
        // ESC, hence esc=1; the ESC put itself is ignored entirely.
        let (out, st) = run(&[
            (MOD, 1, 0x65, 0x41, 7, 9),
            (INS, 1, -1, 0xA7, 0, 1),
            (DEL, 57751, 0, 0, 0, 0),
            (EQL, 1, 0x20, 0x20, 3, 3),
            (ESC, 0, 0, 0, 4, 4),
        ]);
        assert_eq!(
            out,
            concat!(
                "           7            9 MOD 145 101 e-A\n",
                "           0            1 INS     247  - \n",
                "           0            0 DEL 57751\n",
                "           3            3 EQL  40  40  - \n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 2,
                ctl: 15,
                del: 57751,
                bkt: 0,
                esc: 1,
                eql: 1
            }
        );
    }

    #[test]
    fn asc_octal_padding_and_char_boundaries() {
        // Octal is space-padded (`printf` "%3o"), the printable range is
        // 32..=127 inclusive (0x7F prints as itself), and MOD/INS bytes equal
        // to ESC (0xA7) bump `esc`. Oracle-verified literals.
        let (out, st) = run(&[
            (MOD, 1, 0x08, 0x1F, 1, 1),
            (MOD, 1, 0x20, 0x7F, 2, 2),
            (MOD, 1, 0x80, 0xFF, 3, 3),
            (INS, 1, -1, 0x00, 4, 4),
            (EQL, 1, 0x41, 0x41, 5, 5),
            (MOD, 1, 0x30, 0xA7, 6, 6),
            (INS, 1, -1, 0xA7, 7, 7),
        ]);
        assert_eq!(
            out,
            concat!(
                "           1            1 MOD  10  37  - \n",
                "           2            2 MOD  40 177  -\u{7f}\n",
                "           3            3 MOD 200 377  - \n",
                "           4            4 INS       0  - \n",
                "           5            5 EQL 101 101 A-A\n",
                "           6            6 MOD  60 247 0- \n",
                "           7            7 INS     247  - \n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 6,
                ctl: 14,
                del: 0,
                bkt: 0,
                esc: 2,
                eql: 1
            }
        );
    }

    #[test]
    fn stats_counters_match_cxx_rules() {
        // DEL/BKT ctl = 2 + ufPutSze(len) with the §3 tiers 1/2/3/5/9
        // (JOutAsc.cpp:107-126; the 9-byte tier is compiled in because the
        // oracle build defines JDIFF_LARGEFILE); del/bkt accumulate len,
        // EQL ctl = 2+4 on operand change with eql++ per byte.
        // Oracle: dta=6 ctl=64 del=131576 bkt=8589935610 esc=2 eql=1.
        let (out, st) = run(&[
            (MOD, 1, 0x08, 0x1F, 1, 1),
            (MOD, 1, 0x20, 0x7F, 2, 2),
            (MOD, 1, 0x80, 0xFF, 3, 3),
            (INS, 1, -1, 0x00, 4, 4),
            (EQL, 1, 0x41, 0x41, 5, 5),
            (MOD, 1, 0x30, 0xA7, 6, 6),
            (INS, 1, -1, 0xA7, 7, 7),
            (DEL, 252, 0, 0, 8, 8),           // ctl += 2+1
            (DEL, 253, 0, 0, 8, 8),           // ctl += 2+2
            (BKT, 508, 0, 0, 8, 8),           // ctl += 2+2
            (BKT, 509, 0, 0, 8, 8),           // ctl += 2+3
            (DEL, 65_535, 0, 0, 8, 8),        // ctl += 2+3
            (DEL, 65_536, 0, 0, 8, 8),        // ctl += 2+5
            (BKT, 4_294_967_296, 0, 0, 8, 8), // ctl += 2+9
            (BKT, 4_294_967_297, 0, 0, 8, 8), // ctl += 2+9
        ]);
        assert_eq!(
            out,
            concat!(
                "           1            1 MOD  10  37  - \n",
                "           2            2 MOD  40 177  -\u{7f}\n",
                "           3            3 MOD 200 377  - \n",
                "           4            4 INS       0  - \n",
                "           5            5 EQL 101 101 A-A\n",
                "           6            6 MOD  60 247 0- \n",
                "           7            7 INS     247  - \n",
                "           8            8 DEL 252\n",
                "           8            8 DEL 253\n",
                "           8            8 BKT 508\n",
                "           8            8 BKT 509\n",
                "           8            8 DEL 65535\n",
                "           8            8 DEL 65536\n",
                "           8            8 BKT 4294967296\n",
                "           8            8 BKT 4294967297\n",
            )
        );
        assert_eq!(
            st,
            OutStats {
                dta: 6,
                ctl: 64,
                del: 131_576,
                bkt: 8_589_935_610,
                esc: 2,
                eql: 1,
            }
        );
    }
}
