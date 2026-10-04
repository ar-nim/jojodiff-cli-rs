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
pub fn jdiff(args: &[&OsStr]) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::new(env!("CARGO_BIN_EXE_jdiff"));
    c.args(args);
    c
}
