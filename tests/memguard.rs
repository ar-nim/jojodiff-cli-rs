//! Memory-guard integration tests (plan 2026-10-04-memguard-bugfix):
//! absurd option values must exit 10 (EXI_MEM) with the refusal message
//! instead of aborting (SIGABRT). All refusal vectors are petabyte-scale
//! (saturated c_atoi values), so they exceed MemAvailable + SwapFree on
//! any conceivable machine — the tests are machine-independent.

use assert_cmd::Command;
use predicates::boolean::PredicateBooleanExt;
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

/// Layer B / escape hatch: with the guard disabled, a petabyte request is
/// refused by the OS (`try_reserve`) — exit 10 with the refusal note, NOT
/// the Layer-A "(RAM + swap)" text and NOT a SIGABRT. Petabyte scale keeps
/// this machine-independent (no OS grants 2.25 PB to one process).
#[test]
fn escape_hatch_lets_the_os_decide() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    jdiff()
        .env("JDIFF_UNSAFE_NO_MEMGUARD", "1")
        .args(["-m", "99999999999999999999", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("out.patch"))
        .assert()
        .code(10)
        .stderr(contains("refused the"))
        .stderr(contains("Error allocating memory !"))
        .stderr(contains("(RAM + swap)").not());
}

/// "Don't break valid big patches": patch/destination files are STREAMED
/// through the fixed buffers — their SIZE is never bounded by the memory
/// guard, only the option VALUES are. A 32 MiB pair with edits produces a
/// real patch; applying it with defaults and with -m 256 must both be
/// byte-exact. (32 MiB keeps the run in the seconds range.)
fn lcg_bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as u8
        })
        .collect()
}

#[test]
fn big_patch_files_stream_unbounded() {
    let dir = tempfile::tempdir().unwrap();
    let org = lcg_bytes(1, 32 * 1024 * 1024);
    let mut new = org.clone();
    new[..16].copy_from_slice(&org[16 * 1024..16 * 1024 + 16]); // edit near head
    new[10_000_000..10_000_128].fill(0); // edit mid-file
    let a = dir.path().join("big.org");
    let b = dir.path().join("big.new");
    std::fs::write(&a, &org).unwrap();
    std::fs::write(&b, &new).unwrap();
    let patch = dir.path().join("big.patch");

    for m in ["64", "256"] {
        jdiff()
            .args(["-m", m, "-j"])
            .arg(&a)
            .arg(&b)
            .arg(&patch)
            .assert()
            .code(1);
        let out = dir.path().join("big.out");
        jdiff()
            .args(["-u", "-m", m])
            .arg(&a)
            .arg(&patch)
            .arg(&out)
            .assert()
            .code(0);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            new,
            "roundtrip byte-exact with -m {m}"
        );
    }
}

/// Flag-chain regression (plan task 9): `-ffffffff` (8x -f halves
/// mch_max to 0) and `-bbbbbbbbbbbb -m 8 -i 1` (12x -b wraps it to
/// i32::MIN with small buffers) both panicked in JMatchTable's ctor
/// assert (rc=101). Both now run the normal diff path (exit 1).
#[test]
fn flag_chains_floor_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = fixtures(dir.path());
    // -f x8: buffers 16 MB, table 1 MB (hsh_mbt halves to 0 -> clamped
    // to 1 by JHashPos) — deterministic on any machine.
    jdiff()
        .args(["-ffffffff", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("f8.patch"))
        .assert()
        .code(1);
    // -b x12 + small -m/-i: without the floor this reached the ctor with
    // mch_max = i32::MIN (observed rc=101 pre-fix).
    jdiff()
        .args(["-bbbbbbbbbbbb", "-m", "8", "-i", "1", "-j"])
        .arg(&a)
        .arg(&b)
        .arg(dir.path().join("b12.patch"))
        .assert()
        .code(1);
}
