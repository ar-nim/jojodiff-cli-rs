//! Debug output plumbing, ported from the C++ `JDebug` class
//! (`headers/JDebug.h` / `src/JDebug.cpp`).
//!
//! In C++, `JDebug::stddbg` is a `FILE*` pointing at stderr by default and at
//! stdout when the `-do` option is given. This port models it with a
//! process-wide [`DBG_TO_STDOUT`] flag and two accessors. The `gbDbg` debug
//! flags are added here in Task 11.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// Debug/verbose output target: `false` (default) = stderr, `true` = stdout
/// (set by the `-do` option).
pub static DBG_TO_STDOUT: AtomicBool = AtomicBool::new(false);

/// Port of the C++ `JDebug` class; grows the `gbDbg` debug flags in Task 11.
pub struct JDebug;

/// Returns the debug/verbose stream (`JDebug::stddbg`): stderr, or stdout with
/// `-do`. Callers write and flush per line, like the C++ `fprintf(stddbg, ...)`.
pub fn stddbg() -> Box<dyn Write> {
    if DBG_TO_STDOUT.load(Ordering::Relaxed) {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::io::stderr())
    }
}

/// Writes one formatted chunk to the debug stream (stderr, or stdout with
/// `-do`) and flushes. Stream errors are ignored, like a C `FILE*`.
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
