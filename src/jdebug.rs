//! Debug output plumbing, ported from the C++ `JDebug` class
//! (`headers/JDebug.h` / `src/JDebug.cpp`).
//!
//! In C++, `JDebug::stddbg` is a `FILE*` pointing at stderr by default and at
//! stdout when the `-c`/`--console` option is given (0.8.1's `-do` is gone at
//! 0.8.5, spec §18.G). This port models it with a process-wide
//! [`DBG_TO_STDOUT`] flag and two accessors.
//!
//! # The `debug` cargo feature (spec §14, parity with `make debug`)
//!
//! The `debug` cargo feature corresponds to the C++ `-D_DEBUG` build
//! (`#define debug 1`): it enables the `gbDbg` flag array (`JDebug::gbDbg`,
//! `JDebug.h:37-47`), the 11 `-dhsh`…`-ddst` CLI flags and every
//! `#if debug` print site. Without the feature this module provides only the
//! release-build surface (`-c`/`stddbg`, verbose greetings/statistics).
//!
//! Debug builds format positions with `P8zd = %10lld` (width 10) instead of
//! the release `%12lld` — [`crate::defs::p8`] switches on the feature.
//!
//! # Pointer values (`%p`)
//!
//! The C++ sites print buffer/string addresses with `%p`. The Rust ports print
//! the equivalent object addresses (allocation start/end, fid string pointer);
//! only the *format and shape* of those lines is guaranteed to match the
//! oracle — the values themselves are inherently non-reproducible. Two sites
//! (`JFileAhead.cpp:165-170` EOB and `:283-288` short-read EOF) pass the
//! `msFid` *string* to `%p` (a C++ formatting quirk); the port preserves the
//! shape by printing the fid string's address.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// Debug/verbose output target: `false` (default) = stderr, `true` = stdout
/// (set by the `-c`/`--console` option).
pub static DBG_TO_STDOUT: AtomicBool = AtomicBool::new(false);

/// Debug-flag indices into `gbDbg` (`JDebug.h:37-47`). Only meaningful with
/// the `debug` feature; like the C++, the flags `-dhsk`/`-dbkt` have no print
/// sites and are accepted for parity alone.
pub const DBGHSH: usize = 0; // Debug Hash                     -dhsh
pub const DBGAHD: usize = 1; // Debug Ahead                    -dahd
pub const DBGCMP: usize = 2; // Debug Compare                  -dcmp
pub const DBGPRG: usize = 3; // Debug Progress                 -dprg
pub const DBGBUF: usize = 4; // Debug Ahead Buffer             -dbuf
pub const DBGAHH: usize = 5; // Debug Ahead Hash               -dahh
pub const DBGHSK: usize = 6; // Debug ufHshNxt                 -dhsk
pub const DBGBKT: usize = 7; // Debug ufFabSek                 -dbkt
pub const DBGRED: usize = 8; // Debug ufFabGet                 -dred
pub const DBGMCH: usize = 9; // Debug ufMch...                 -dmch
pub const DBGDST: usize = 10; // Debug Hashtable distribution   -ddst

/// Port of the C++ `JDebug` class; the `gbDbg` flags live in [`GB_DBG`].
pub struct JDebug;

/// Returns the debug/verbose stream (`JDebug::stddbg`): stderr, or stdout
/// with `-c`. Callers write and flush per line, like the C++
/// `fprintf(stddbg, ...)`.
pub fn stddbg() -> Box<dyn Write> {
    if DBG_TO_STDOUT.load(Ordering::Relaxed) {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::io::stderr())
    }
}

/// Writes one formatted chunk to the debug stream (stderr, or stdout with
/// `-c`) and flushes. Stream errors are ignored, like a C `FILE*`.
pub fn dbg_print(args: std::fmt::Arguments<'_>) {
    if DBG_TO_STDOUT.load(Ordering::Relaxed) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_fmt(args);
        let _ = out.flush();
    } else {
        let mut out = std::io::stderr().lock();
        let _ = out.write_fmt(args);
        let _ = out.flush();
    }
}

/// The C++ `JDebug::gbDbg[16]` flag array (`JDebug.cpp:30`), process-global
/// like the static original. A plain `Mutex` over a fixed array keeps it
/// `no unsafe`, `no static mut`.
#[cfg(feature = "debug")]
pub static GB_DBG: std::sync::Mutex<[bool; 16]> = std::sync::Mutex::new([false; 16]);

/// Reads debug flag `idx` (`JDebug::gbDbg[idx]`). Debug builds only.
#[cfg(feature = "debug")]
pub fn dbg(idx: usize) -> bool {
    GB_DBG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[idx]
}

/// Sets debug flag `idx` to `val` (`JDebug::gbDbg[idx] = val`); called by the
/// `-d*` CLI options. Debug builds only.
#[cfg(feature = "debug")]
pub fn dbg_set(idx: usize, val: bool) {
    GB_DBG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[idx] = val;
}

/// Every test here exercises feature-only surface, so the whole module is
/// gated: in default builds it would compile to an empty shell whose imports
/// trip `-D warnings`.
#[cfg(all(test, feature = "debug"))]
mod tests {
    use super::*;

    /// Flag indices are the `JDebug.h:37-47` constants.
    #[cfg(feature = "debug")]
    #[test]
    fn flag_indices_match_jdebug_h() {
        assert_eq!(
            (DBGHSH, DBGAHD, DBGCMP, DBGPRG, DBGBUF, DBGAHH),
            (0, 1, 2, 3, 4, 5)
        );
        assert_eq!((DBGHSK, DBGBKT, DBGRED, DBGMCH, DBGDST), (6, 7, 8, 9, 10));
    }

    /// `gbDbg` starts all-zero and is mutated through `dbg_set`
    /// (`JDebug.cpp:30`); the test restores every flag it touches and holds
    /// the crate-wide test lock while the flags are set, so parallel engine
    /// tests never observe a stray flag.
    #[cfg(feature = "debug")]
    #[test]
    fn flags_default_false_and_set_roundtrip() {
        let _guard = crate::test_util::gb_dbg_guard();
        for idx in 0..16 {
            assert!(!dbg(idx), "gbDbg[{idx}] must default to false");
        }
        dbg_set(DBGMCH, true);
        assert!(dbg(DBGMCH));
        dbg_set(DBGMCH, false);
        assert!(!dbg(DBGMCH));
        dbg_set(DBGDST, true);
        dbg_set(DBGDST, false);
    }
}
