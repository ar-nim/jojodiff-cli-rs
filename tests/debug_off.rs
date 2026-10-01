#![cfg(not(feature = "debug"))]
//! Release-build contract for the `-d*` tokens (Task 11, spec §14): without
//! the `debug` feature the `#if debug` strcmp branches are gone, so `-dhsh`…
//! `-ddst` fall through to the non-option branch and are treated as filenames,
//! exactly like the stock release C++ binary (`main.cpp:286-309`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Unique temp directory per test (parallel-safe), cleaned up by the caller.
fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jdiff-t11off-{}-{}-{}",
        tag,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// `jdiff -dhsh …`: the token is not an option here, ends the option loop and
/// becomes the first filename — the "Could not open first file" error proves
/// it was never consumed as a debug flag (`main.cpp` falls to the `else`).
#[test]
fn dhsh_token_is_a_filename_without_the_feature() {
    let dir = temp_dir("dhsh");
    let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .arg("-dhsh")
        .arg("a.bin")
        .arg("b.bin")
        .arg("p.bin")
        .current_dir(&dir)
        .output()
        .expect("spawn jdiff");
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Could not open first file -dhsh for reading.\n"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// The default run stays pristine: no debug output anywhere.
#[test]
fn default_run_is_pristine() {
    let dir = temp_dir("pristine");
    fs::write(dir.join("a.bin"), b"hello world hello").unwrap();
    fs::write(dir.join("b.bin"), b"hello world byebye").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .arg("-m")
        .arg("1")
        .arg("a.bin")
        .arg("b.bin")
        .arg("p.bin")
        .current_dir(&dir)
        .output()
        .expect("spawn jdiff");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}
