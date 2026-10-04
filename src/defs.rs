//! Global definitions, ported 1:1 from the 0.8.5 C++ `src/JDefs.h` plus
//! `is_prime`/`getLowerPrime` from `src/JDefs.cpp` and the `MINEQL` threshold
//! from `src/JOutBin.h`.
//!
//! Every constant is byte-exact with the original (including historical
//! spellings); C++ source references are kept for traceability.

use std::ffi::OsStr;

/// C `EOF`: end of input, never a valid byte value (`JDefs.h`).
pub const EOF: i32 = -1;

/// End-Of-Buffer constant, `EOF - 1` = -2 (`JDefs.h:145`).
pub const EOB: i32 = EOF - 1;

/// Read type for [`JFile::get`](crate::jfile::JFile::get):
/// `0=read, 1=hard ahead, 2=soft ahead` (`JFileIStream.cpp:46-48`). Only used
/// for dispatch, never serialized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadType {
    Read = 0,
    HardAhead = 1,
    SoftAhead = 2,
}

/// Sample size in bits, `sizeof(hkey) * 8` on the 32-bit oracle build
/// (`_LARGESAMPLE` is not set by the Makefile); literal in Rust.
pub const SMPSZE: i32 = 32;

// (The 0.8.1-era MCH_PME/MCH_MAX table constants are gone in 0.8.5: the
// matching table is dynamic — `getLowerPrime(aiMchSze * 2)`, spec §18.E/§19.
// The prime lives on as the `mch_pme` field of `JMatchTable`.)

/// EQL flush threshold (`MINEQL`, `JOutBin.h:28`): 0.8.5 lowered it from 4
/// to 2 ("start EQL-sequence on 3'rd byte"), a wire-format break (§18.C).
pub const MINEQL: i32 = 2;

/// Patch-format opcodes (`JDefs.h:163-168`): the wire values are the
/// discriminants. `Esc` doubles as the data-escape and the "no operator
/// yet" seed of `opr_cur`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive] // API-07; crate-internal matches stay exhaustive
pub enum Op {
    /// `ESC` 0xA7 — escape.
    Esc = 0xA7,
    /// `MOD` 0xA6 — modify.
    Mod = 0xA6,
    /// `INS` 0xA5 — insert.
    Ins = 0xA5,
    /// `DEL` 0xA4 — delete.
    Del = 0xA4,
    /// `EQL` 0xA3 — equal.
    Eql = 0xA3,
    /// `BKT` 0xA2 — backtrace.
    Bkt = 0xA2,
}

impl Op {
    /// The wire byte (the value of the C++ `i32` opcode constants).
    pub const fn byte(self) -> u8 {
        self as u8
    }

    /// Classifies a patch-stream byte as an opcode; `None` for every
    /// non-opcode byte, which stays data in the decoder exactly like the
    /// C++ `default` arms.
    pub const fn from_byte(b: u8) -> Option<Op> {
        match b {
            x if x == Self::Esc as u8 => Some(Self::Esc),
            x if x == Self::Mod as u8 => Some(Self::Mod),
            x if x == Self::Ins as u8 => Some(Self::Ins),
            x if x == Self::Del as u8 => Some(Self::Del),
            x if x == Self::Eql as u8 => Some(Self::Eql),
            x if x == Self::Bkt as u8 => Some(Self::Bkt),
            _ => None,
        }
    }
}

/// Exit codes, renumbered/negated at 0.8.5 with the new `EXI_OK`
/// (`JDefs.h:146-158`). The CLI exits with `-EXI_*` so the process exit codes
/// stay positive (2/3/4/5/6/7/8/9/10/20); the engine returns the raw negative
/// error codes (`JFileAhead.h:115` `SeekError = EXI_SEK`).
pub const EXI_OK: i32 = 0; // OK Exit code
pub const EXI_DIF: i32 = 1; // OK Exit code, differences found
pub const EXI_EQL: i32 = 2; // OK Exit code, no differences found
pub const EXI_ARG: i32 = -2; // Error: not enough arguments
pub const EXI_FRT: i32 = -3; // Error opening first file
pub const EXI_SCD: i32 = -4; // Error opening second file
pub const EXI_OUT: i32 = -5; // Error opening output file
pub const EXI_SEK: i32 = -6; // Error seeking file
pub const EXI_LRG: i32 = -7; // Error on 64-bit number
pub const EXI_RED: i32 = -8; // Error reading file
pub const EXI_WRI: i32 = -9; // Error writing file
pub const EXI_MEM: i32 = -10; // Error allocating memory
pub const EXI_ERR: i32 = -20; // Spurious error occured (sic, original spelling)

/// Version string, byte-exact with JojoDiff 0.8.5 (`JDefs.h:41`).
pub const JDIFF_VERSION: &str = "0.8.5 (beta) 2020";

/// Copyright string, byte-exact with JojoDiff 0.8.5 (`JDefs.h:42`).
pub const JDIFF_COPYRIGHT: &str = "Copyright (C) 2002-2020 Joris Heirbaut";

/// Largest positive offset, `off_t` maximum on the 64-bit build (`JDefs.h`).
pub const MAX_OFF_T: i64 = i64::MAX;

/// Check if number is a prime number (`isPrime`, `JDefs.cpp:37-45`).
pub fn is_prime(number: i32) -> bool {
    if number < 2 {
        return false;
    }
    if number == 2 {
        return true;
    }
    if number % 2 == 0 {
        return false;
    }
    let mut i = 3;
    while number / i >= i {
        if number % i == 0 {
            return false;
        }
        i += 2;
    }
    true
}

/// Get highest lower prime (`getLowerPrime`, `JDefs.cpp:53-67`), including
/// the exact switch cases; every other value is answered by a downward
/// [`is_prime`] search.
pub fn get_lower_prime(ai_num: i32) -> i32 {
    /* The switch operands of `JDefs.cpp:55-60` as named consts: Rust
     * patterns accept literals and const paths, not arithmetic
     * expressions like the C++ case labels. */
    const MB32: i32 = 32 * 1024 * 1024;
    const MB16: i32 = 16 * 1024 * 1024;
    const MB8: i32 = 8 * 1024 * 1024;
    const MB128: i32 = 128 * 1024 * 1024;
    const MB512: i32 = 512 * 1024 * 1024;
    match ai_num {
        1024 => return 1021,
        MB32 => return 33554393,
        MB16 => return 16777213,
        MB8 => return 8388593,
        MB128 => return 134217689,
        MB512 => return 536870909,
        _ => {}
    }
    let mut ai_num = ai_num;
    while ai_num > 0 {
        if is_prime(ai_num) {
            return ai_num;
        }
        ai_num -= 1;
    }
    ai_num
}

/// Width used by `P8zd` (`JDefs.h:118-126`): width 12 in release builds,
/// width 10 in `-D_DEBUG`/debug-feature builds. The feature switch is a
/// compile-time constant, so both branches fold in their respective builds.
pub fn p8(v: i64) -> String {
    if cfg!(feature = "debug") {
        format!("{:>10}", v)
    } else {
        format!("{:>12}", v)
    }
}

/// C `atoi` semantics on the lossy form of an [`OsStr`]: skip leading C
/// whitespace (space, `\t`, `\n`, `\v`, `\f`, `\r`), optional sign, then the
/// leading decimal digits; everything after the digits is ignored, and `0` is
/// returned when there are none (so `-m abc` parses as `-m 0`, which at 0.8.5
/// selects the default sizes — spec §21.12; 0.8.1's in-memory mode is void).
///
/// Unlike C, where overflow is undefined behavior, the accumulator saturates:
/// results clamp to `i32::MAX` / `i32::MIN`.
pub fn c_atoi(s: &OsStr) -> i32 {
    let lossy = s.to_string_lossy();
    let mut it = lossy.bytes().peekable();

    // Skip leading whitespace (C `isspace`).
    while matches!(
        it.peek(),
        Some(b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
    ) {
        it.next();
    }

    let negative = match it.peek() {
        Some(b'-') => {
            it.next();
            true
        }
        Some(b'+') => {
            it.next();
            false
        }
        _ => false,
    };

    let mut acc: i64 = 0;
    for byte in it {
        let Some(digit) = char::from(byte).to_digit(10) else {
            break;
        };
        acc = acc.saturating_mul(10).saturating_add(i64::from(digit));
    }

    let val = if negative { -acc } else { acc };
    val.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

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

/// Human-readable MiB label for memory-budget messages: one decimal place,
/// `Mb` suffix matching the options' own units (`-m`/`-i` are MB).
pub fn fmt_mb(bytes: i64) -> String {
    format!("{:.1}Mb", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    /// Memory-budget labels (`fmt_mb`): one decimal, `Mb` suffix matching
    /// the options' own units (-m/-i are MB).
    #[test]
    fn fmt_mb_labels() {
        assert_eq!(fmt_mb(0), "0.0Mb");
        assert_eq!(fmt_mb(1048576), "1.0Mb");
        assert_eq!(fmt_mb(33554172), "32.0Mb");
        assert_eq!(fmt_mb(2148531564), "2049.0Mb"); // 2048.9995 MiB rounds up
        assert_eq!(fmt_mb(1125899905794048), "1073741823.0Mb"); // 2^50 - 2^20
    }

    #[test]
    fn constants_match_cxx() {
        assert_eq!(EOB, -2);
        // Opcode wire values (`JDefs.h:163-168`) as the Op discriminants.
        assert_eq!(
            (
                Op::Esc.byte(),
                Op::Mod.byte(),
                Op::Ins.byte(),
                Op::Del.byte(),
                Op::Eql.byte(),
                Op::Bkt.byte()
            ),
            (0xA7, 0xA6, 0xA5, 0xA4, 0xA3, 0xA2)
        );
        // from_byte classifies every opcode and rejects its neighborhood.
        assert_eq!(Op::from_byte(0xA7), Some(Op::Esc));
        assert_eq!(Op::from_byte(0xA2), Some(Op::Bkt));
        assert_eq!(Op::from_byte(0xA1), None);
        assert_eq!(Op::from_byte(0xA8), None);
        assert_eq!(Op::from_byte(0x00), None);
        assert_eq!(Op::from_byte(0xFF), None);
    }

    /// Exit codes, byte-exact with 0.8.5 (`JDefs.h:146-158`): a positive
    /// `EXI_OK`/`EXI_DIF`/`EXI_EQL` and negative error codes; the process
    /// exit codes stay positive via `exit(-EXI_*)`.
    #[test]
    fn exit_codes_match_cxx_085() {
        assert_eq!(EXI_OK, 0);
        assert_eq!(EXI_DIF, 1);
        assert_eq!(EXI_EQL, 2);
        assert_eq!(EXI_ARG, -2);
        assert_eq!(EXI_FRT, -3);
        assert_eq!(EXI_SCD, -4);
        assert_eq!(EXI_OUT, -5);
        assert_eq!(EXI_SEK, -6);
        assert_eq!(EXI_LRG, -7);
        assert_eq!(EXI_RED, -8);
        assert_eq!(EXI_WRI, -9);
        assert_eq!(EXI_MEM, -10);
        assert_eq!(EXI_ERR, -20);
    }

    /// Version strings, byte-exact with 0.8.5 (`JDefs.h:41-42`).
    #[test]
    fn version_strings_match_cxx_085() {
        assert_eq!(JDIFF_VERSION, "0.8.5 (beta) 2020");
        assert_eq!(JDIFF_COPYRIGHT, "Copyright (C) 2002-2020 Joris Heirbaut");
    }

    /// EQL flush threshold (`MINEQL`, `JOutBin.h:28`): 0.8.5 lowered it
    /// from 4 to 2 ("start EQL-sequence on 3'rd byte").
    #[test]
    fn mineql_matches_cxx_085() {
        assert_eq!(MINEQL, 2);
    }

    /// `isPrime` edges (`JDefs.cpp:37-45`): <2 not prime, 2 prime, even
    /// composites rejected, the trial loop (`i` odd, `number/i >= i`)
    /// rejects odd composites and accepts odd primes.
    #[test]
    fn is_prime_edges() {
        assert!(!is_prime(0));
        assert!(!is_prime(1));
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(!is_prime(4)); // even composite
        assert!(is_prime(5));
        assert!(is_prime(7)); // odd prime
        assert!(!is_prime(9)); // odd composite (9/3 >= 3)
        assert!(!is_prime(25)); // odd composite (25/5 >= 5)
        assert!(is_prime(2097143));
    }

    /// `getLowerPrime` (`JDefs.cpp:53-67`): the exact switch cases and the
    /// default downward `isPrime` search.
    #[test]
    fn get_lower_prime_switch_and_search() {
        // Exact switch cases (`JDefs.cpp:55-60`).
        assert_eq!(get_lower_prime(1024), 1021);
        assert_eq!(get_lower_prime(32 * 1024 * 1024), 33554393);
        assert_eq!(get_lower_prime(16 * 1024 * 1024), 16777213);
        assert_eq!(get_lower_prime(8 * 1024 * 1024), 8388593);
        assert_eq!(get_lower_prime(128 * 1024 * 1024), 134217689);
        assert_eq!(get_lower_prime(512 * 1024 * 1024), 536870909);
        // Default branch: downward isPrime search.
        assert_eq!(get_lower_prime(2097152), 2097143);
        assert_eq!(get_lower_prime(1000), 997);
        // Exhausted search returns the (non-positive) counter like the C++.
        assert_eq!(get_lower_prime(0), 0);
        assert_eq!(get_lower_prime(-5), -5);
    }

    #[test]
    fn atoi_semantics() {
        assert_eq!(c_atoi(OsStr::new("128")), 128);
        assert_eq!(c_atoi(OsStr::new("  -42abc")), -42);
        assert_eq!(c_atoi(OsStr::new("abc")), 0);
        assert_eq!(c_atoi(OsStr::new("1")), 1); // -m 1 → 1/2*1024 = 0
        assert_eq!(c_atoi(OsStr::new("")), 0);
    }

    /// The C `%c` printable-ASCII filter shared by the DBGCMP trace, the
    /// ASCII listing and the patch verbose trace (`JMatchTable.cpp:857-864`).
    #[test]
    fn print_char_matches_c_percent_c_filter() {
        assert_eq!(print_char(0x68), 'h');
        assert_eq!(print_char(32), ' ');
        assert_eq!(print_char(65), 'A');
        assert_eq!(print_char(127), '\u{7f}');
        assert_eq!(print_char(31), ' ');
        assert_eq!(print_char(128), ' ');
        assert_eq!(print_char(-1), ' ');
    }

    #[test]
    fn width_formatting() {
        // `P8zd` is `%10` in debug-feature builds, `%12` in release
        // (JDefs.h:118-126); both pinned against the respective oracle builds.
        if cfg!(feature = "debug") {
            assert_eq!(p8(0), "         0");
            assert_eq!(p8(57751), "     57751");
            assert_eq!(p8(-1), "        -1");
        } else {
            assert_eq!(p8(0), "           0");
            assert_eq!(p8(57751), "       57751");
            assert_eq!(p8(-1), "          -1");
        }
    }
}
