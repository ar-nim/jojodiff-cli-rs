//! Global definitions, ported 1:1 from the C++ `headers/JDefs.h` plus the
//! `giPme` prime table from `src/JHashPos.cpp`.
//!
//! Every constant is byte-exact with the original (including historical
//! spellings); C++ source references are kept for traceability.

use std::ffi::OsStr;

/// C `EOF`: end of input, never a valid byte value (`JDefs.h`).
pub const EOF: i32 = -1;

/// End-Of-Buffer constant, `EOF - 1` = -2 (`JDefs.h:110`).
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

/// Matching hashtable prime (`JMatchTable.h:29`).
pub const MCH_PME: i64 = 127;

/// Maximum size of matching table (`JMatchTable.h:30`).
pub const MCH_MAX: i32 = 256;

/** Output routine constants (`JDefs.h:127-132`). */
pub const ESC: i32 = 0xA7; // Escape
pub const MOD: i32 = 0xA6; // Modify
pub const INS: i32 = 0xA5; // Insert
pub const DEL: i32 = 0xA4; // Delete
pub const EQL: i32 = 0xA3; // Equal
pub const BKT: i32 = 0xA2; // Backtrace

/// Exit codes (`JDefs.h:111-122`).
pub const EXI_DIF: i32 = 0; // OK, differences found
pub const EXI_EQL: i32 = 1; // OK, no differences found
pub const EXI_ARG: i32 = 2; // Error: not enough arguments
pub const EXI_FRT: i32 = 3; // Error opening first file
pub const EXI_SCD: i32 = 4; // Error opening second file
pub const EXI_OUT: i32 = 5; // Error opening output file
pub const EXI_SEK: i32 = 6; // Error seeking file
pub const EXI_LRG: i32 = 7; // Error on 64-bit number
pub const EXI_RED: i32 = 8; // Error reading file
pub const EXI_WRI: i32 = 9; // Error writing file
pub const EXI_MEM: i32 = 10; // Error allocating memory
pub const EXI_ERR: i32 = 20; // Spurious error occured (sic, original spelling)

/// Version string, byte-exact with JojoDiff 0.8.1 (`JDefs.h:93`).
pub const JDIFF_VERSION: &str = "0.8.1 (beta) December 2011";

/// Copyright string, byte-exact with JojoDiff 0.8.1 (`JDefs.h:94`).
pub const JDIFF_COPYRIGHT: &str = "Copyright (C) 2002-2005,2009,2011 Joris Heirbaut";

/// Largest positive offset, `off_t` maximum on the 64-bit build (`JDefs.h`).
pub const MAX_OFF_T: i64 = i64::MAX;

/// Primes we select from when size is specified on the commandline
/// (`giPme`, `JHashPos.cpp:38-43`).
pub const GIPME: [i32; 20] = [
    134217689, 67108859, 33554393, 16777213, 8388593, 4194301, 2097143, 1048573, 524287, 262139,
    131071, 65521, 32749, 16381, 8191, 4093, 2039, 1021, 509, 251,
];

/// Width used by `P8zd` (`JDefs.h:80-88`): `%12lld` in release builds,
/// `%10lld` in `-D_DEBUG`/debug-feature builds. The feature switch is a
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
/// returned when there are none (so `-m abc` ⇒ 0 ⇒ in-memory mode).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn constants_match_cxx() {
        assert_eq!(EOB, -2);
        assert_eq!(
            (ESC, MOD, INS, DEL, EQL, BKT),
            (0xA7, 0xA6, 0xA5, 0xA4, 0xA3, 0xA2)
        );
        assert_eq!(EXI_ERR, 20);
        assert_eq!(GIPME.len(), 20);
        assert_eq!(GIPME[19], 251);
    }

    #[test]
    fn atoi_semantics() {
        assert_eq!(c_atoi(OsStr::new("128")), 128);
        assert_eq!(c_atoi(OsStr::new("  -42abc")), -42);
        assert_eq!(c_atoi(OsStr::new("abc")), 0);
        assert_eq!(c_atoi(OsStr::new("1")), 1); // -m 1 → 1/2*1024 = 0
        assert_eq!(c_atoi(OsStr::new("")), 0);
    }

    #[test]
    fn width_formatting() {
        // `P8zd` is `%10lld` in debug-feature builds, `%12lld` in release
        // (JDefs.h:80-88); both pinned against the respective oracle builds.
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
