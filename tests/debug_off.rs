#![cfg(not(feature = "debug"))]
//! Release-build contract for the `-d <name>` option (Task 20's 0.8.5 CLI,
//! spec §14/§18.G): the `#if debug` strcmp arms are compiled out in release
//! builds, so `-d <name>` is parsed and consumed like every other option but
//! sets nothing — the run stays pristine (the 0.8.1 behavior of `-dhsh`
//! falling through to a filename died with the getopt_long rewrite: it is
//! now `-d` with an attached argument).

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

/// `jdiff -dhsh a.bin b.bin p.bin` (release build): `-dhsh` is `-d` with the
/// attached argument "hsh" — consumed silently (the strcmp arms are compiled
/// out), and the diff runs normally on the three operands.
#[test]
fn dhsh_token_is_an_option_without_the_feature() {
    let dir = temp_dir("dhsh");
    fs::write(dir.join("a.bin"), b"hello world hello").unwrap();
    fs::write(dir.join("b.bin"), b"hello world byebye").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .arg("-dhsh")
        .arg("a.bin")
        .arg("b.bin")
        .arg("p.bin")
        .current_dir(&dir)
        .output()
        .expect("spawn jdiff");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stderr.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(dir.join("p.bin")).unwrap(), {
        // ESC EQL 11 "byeby" ESC INS "e" (0.8.5 implicit MOD).
        [
            0xA7u8, 0xA3, 0x0B, b'b', b'y', b'e', b'b', b'y', 0xA7, 0xA5, b'e',
        ]
    });
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
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}
