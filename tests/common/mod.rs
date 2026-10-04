//! Shared test harness (spec §8.4): tempfile-backed scratch dirs with
//! RAII cleanup, and an assert_cmd factory for spawning the binary.

#![allow(dead_code)] // each integration test crate compiles this module; not all use every helper

use std::ffi::OsStr;
use tempfile::TempDir;

/// Scratch dir under the system temp root, auto-removed on drop.
/// Bounded by construction: tests write fixtures here, never unbounded
/// streams (spec §10 bounded-loops rule).
pub fn scratch(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("jdiff-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("create scratch dir")
}

/// Spawns the jdiff binary with args.
///
/// `JDIFF_UNSAFE_NO_MEMGUARD=1` opts the harness out of the Layer-A
/// pre-flight gate (spec §21.19): the option matrices include `-m 2048`,
/// whose footprint fits a big machine but exceeds
/// `MemAvailable + SwapFree − headroom` on small/loaded ones where the
/// sparse zero pages previously ran fine. These suites pin the byte
/// contract, not the host's RAM; the guard itself is exercised
/// (petabyte-scale, machine-independent) in tests/memguard.rs.
pub fn jdiff(args: &[&OsStr]) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::new(env!("CARGO_BIN_EXE_jdiff"));
    c.env("JDIFF_UNSAFE_NO_MEMGUARD", "1");
    c.args(args);
    c
}
