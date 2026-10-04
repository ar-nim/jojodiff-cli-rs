//! Memory-guard integration tests (plan 2026-10-04-memguard-bugfix):
//! absurd option values must exit 10 (EXI_MEM) with the refusal message
//! instead of aborting (SIGABRT). All refusal vectors are petabyte-scale
//! (saturated c_atoi values), so they exceed MemAvailable + SwapFree on
//! any conceivable machine — the tests are machine-independent.

use assert_cmd::Command;
use predicates::str::contains;

fn jdiff() -> Command {
    Command::cargo_bin("jdiff").unwrap()
}

/// Fixture pair: 17 bytes differing in the tail (roundtrip.rs pair).
fn fixtures(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    std::fs::write(&a, b"hello world hello").unwrap();
    std::fs::write(&b, b"hello world byebye").unwrap();
    (a, b)
}

#[test]
fn absurd_buffer_size_exits_10_with_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .args(["-m", "99999999999999999999", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("available (RAM + swap)"))
        .stderr(contains("JDIFF_UNSAFE_NO_MEMGUARD=1"))
        .stderr(contains("Error allocating memory !"))
        .stderr(contains("buffers (-m)"));
}

#[test]
fn absurd_buffer_and_search_max_exits_10() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .args([
            "-m",
            "99999999999999999999",
            "-x",
            "99999999999999999999",
            "-j",
        ])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("match table (-x)"))
        .stderr(contains("Error allocating memory !"));
}

#[test]
fn sane_sizes_still_run_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    let patch = dir.path().join("ok.patch");
    // Exit 1 = differences found, the normal 0.8.5 swap.
    jdiff()
        .args(["-m", "64", "-i", "8", "-x", "256", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(&patch)
        .assert()
        .code(1);
    let out = dir.path().join("out.bin");
    jdiff()
        .args(["-u"])
        .arg(&a)
        .arg(&patch)
        .arg(&out)
        .assert()
        .code(0);
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&b).unwrap());
}
