#![cfg(feature = "debug")]
//! Integration tests for the `debug` cargo feature (port plan Task 11, spec
//! §14): the 11 `-d*` CLI flags and the `#if debug` print sites, at parity with
//! the C++ `make debug` (`-D_DEBUG`) builds.
//!
//! Every expected string below was pinned against the vendored C++ oracle
//! (`reference/jojodiff-cpp`) built with `make debug` (forced 32-bit `hkey`
//! and the spec §15.1 2-line seek fix, like every other task's oracle). The
//! `-dprg` "Input" lines confirmed the debug-build `P8zd` width of 10
//! (`%10lld`, `JDefs.h` `#if debug` branch) on the very first line, which
//! prints position `-1` (the C++ prints `lzPosOrg - 1` before the first
//! increment — quirk preserved).
//!
//! The tests spawn the real binary with `-do`, which redirects the debug
//! stream to stdout (`JDebug::stddbg = stdout`, `main.cpp:283-284`), so
//! `Command::output()` captures it deterministically and no test leaks output
//! into the test log. The patch always goes to a file, leaving stdout purely
//! debug output. Pointer values (`%p`, buffer addresses) are inherently
//! non-reproducible; such lines are matched by prefix/suffix only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// Deterministic pseudo-random bytes (LCG, bits 8..=15), identical to the
/// fixtures the oracle expectations below were captured with.
fn lcg(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as u8
        })
        .collect()
}

/// Tiny pair: A = "hello world hello", B = "hello world byebye" (17 bytes).
const ORG_A: &[u8] = b"hello world hello";
const NEW_B: &[u8] = b"hello world byebye";

/// 1000-byte LCG pair differing only at byte 250 (`big_a`/`big_b` oracle
/// fixtures; NB the first byte is 89 = 0x59, not the LCG value — the seed's
/// first output byte IS 0x59, no patch needed).
fn big_pair() -> (Vec<u8>, Vec<u8>) {
    let org = lcg(1, 1000);
    let mut new = org.clone();
    new[250] ^= 0xFF;
    (org, new)
}

/// 100000-byte LCG pair with a large repeated block (moves 40000 bytes from
/// org[20000..60000] to offset 60000), forcing out-of-buffer reads and seeks
/// with a small `-m 16` (8 kB) buffer (`sek_a`/`sek_b` oracle fixtures).
fn sek_pair() -> (Vec<u8>, Vec<u8>) {
    let org = lcg(7, 100_000);
    let mut new = Vec::with_capacity(140_000);
    new.extend_from_slice(&org[0..60_000]);
    new.extend_from_slice(&org[20_000..60_000]);
    new.extend_from_slice(&org[60_000..100_000]);
    (org, new)
}

/// Unique temp directory per test (parallel-safe), cleaned up by the caller.
fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jdiff-t11-{}-{}-{}",
        tag,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_file(path: &Path, content: &[u8]) -> PathBuf {
    fs::write(path, content).expect("write fixture");
    path.to_path_buf()
}

/// Runs `jdiff -do <args...> a b out` with the given fixture pair and returns
/// (stdout-as-utf8, output). `-do` sends the debug stream to stdout.
fn run_dbg(dir: &Path, org: &[u8], new: &[u8], extra: &[&str], tag: &str) -> (String, Output) {
    let a = write_file(&dir.join(format!("{tag}a.bin")), org);
    let b = write_file(&dir.join(format!("{tag}b.bin")), new);
    let p = dir.join(format!("{tag}p.bin"));

    let mut args: Vec<std::ffi::OsString> = vec!["-do".into()];
    args.extend(extra.iter().map(std::ffi::OsString::from));
    args.extend([
        a.as_os_str().to_os_string(),
        b.as_os_str().to_os_string(),
        p.into_os_string(),
    ]);

    let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(&args)
        .output()
        .expect("spawn jdiff");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out)
}

/// The patch jdiff produces for (big pair), pinned byte-exact against the
/// oracle (EQL 249, MOD 0xc3, EQL 750-ish run — 11 bytes total). Debug flags
/// must not change it (the debug flush's `put(ESC, 0, …)` is a writer no-op).
const PATCH_BIG: &[u8] = &[
    0xA7, 0xA3, 0xF9, 0xA7, 0xA6, 0xC3, 0xA7, 0xA3, 0xFD, 0x02, 0xED,
];

/// DBGPRG (`-dprg`, `JDiff.cpp:145-148`): "Input " lines with the debug-width
/// `P8zd` (= 10) positions; the C++ prints `lzPosOrg - 1`, so the first line
/// reports position `-1` (oracle-pinned quirk).
#[test]
fn prg_input_lines_tiny() {
    let dir = temp_dir("prg-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dprg"], "prg");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("Input         -1->68         -1->68.\n"),
        "first Input line: {stdout:?}"
    );
    assert!(
        stdout.contains("Input         11->68         11->62.\n"),
        "mismatch line ('h'->'b' at file position 12): {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGBUF (`-dbuf`, `JFileAhead.cpp:50-54`): one `ufFabOpn` line per reader
/// with the buffer pointers (shape-only) and size.
#[test]
fn buf_open_lines() {
    let dir = temp_dir("buf-open");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dbuf"], "buf");
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "exactly two ufFabOpn lines: {stdout:?}");
    assert!(
        lines[0].starts_with("ufFabOpn(Org):(buf=0x") && lines[0].ends_with("sze=262144)"),
        "Org open line: {stdout:?}"
    );
    assert!(
        lines[1].starts_with("ufFabOpn(New):(buf=0x") && lines[1].ends_with("sze=262144)"),
        "New open line: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGRED (`-dred`): the four `ufFabGet` line shapes — store-fill, memory
/// fast path, memory EOF and the short-read EOF line (which prints the fid's
/// address with `%p`, a C++ formatting quirk preserved in shape).
#[test]
fn red_get_lines_tiny() {
    let dir = temp_dir("red-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dred"], "red");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("ufFabGet(Org,         0,0)->68 (sto 0x"),
        "sto line: {stdout:?}"
    );
    assert!(
        stdout.contains("ufFabGet(Org,         0,0)->68 (mem 0x"),
        "mem line: {stdout:?}"
    );
    assert!(
        stdout.lines().last() == Some("ufFabGet(New,        18,0)->EOF (mem)."),
        "final EOF line: {stdout:?}"
    );
    // Short read at buffer fill: `%p` of the fid string, then the position.
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("ufFabGet(0x") && l.ends_with(",         0,0)->EOF.")),
        "short-read EOF line (fid as %p): {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGPRG on the 1000-byte pair, including the byte-250 mismatch and the
/// "Current position" line after find-ahead (`JDiff.cpp:219-221`).
#[test]
fn prg_big_mismatch_and_current_position() {
    let dir = temp_dir("prg-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dprg"], "prgB");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.starts_with("Input         -1->59         -1->59.\n"),
        "first line: {stdout:?}"
    );
    assert!(
        stdout.contains("Input        249->3c        249->c3.\n"),
        "mismatch at 250: {stdout:?}"
    );
    assert!(
        stdout.contains("Current position in new file= 250\n"),
        "current-position line: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGAHD (`-dahd`, `JDiff.cpp:216-218`): the find-ahead summary. On this
/// fixture exactly one line is produced.
#[test]
fn ahd_findahead_line_big() {
    let dir = temp_dir("ahd-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dahd"], "ahdB");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout, "Findahead on 250 250 skip 0 0 ahead 1\n",
        "exact find-ahead line"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGAHH (`-dahh`) in the prescan loop (`JDiff.cpp:546-550`): the final
/// field is `%8d` of the literal 0. First and last line oracle-pinned.
#[test]
fn ahh_prescan_lines_big() {
    let dir = temp_dir("ahh-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dahh"], "ahhB");
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    // The prescan init loop consumes the first SMPSZE-1 = 31 bytes, so the
    // add loop stores samples at positions 31..=999: 969 lines.
    assert_eq!(lines.len(), 969, "one line per stored sample: {stdout:?}");
    assert_eq!(lines[0], "ufHshAdd(d5 -> c8d9b3a9,         31,        0)");
    assert_eq!(
        lines[lines.len() - 1],
        "ufHshAdd(9a -> 956ca3a4,        999,        0)"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGAHH in the find-ahead loop (`JDiff.cpp:388-392`, needs `-ff`): both
/// trailing fields are `P8zd` (width 10), unlike the prescan's `%8d` tail.
#[test]
fn ahh_findahead_lines_ff() {
    let dir = temp_dir("ahh-ff");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-ff", "-dahh"], "ahhFF");
    assert_eq!(out.status.code(), Some(0));
    let first = stdout.lines().next().unwrap();
    assert_eq!(
        first, "ufHshAdd(d6 -> d0d7708e,        257,          0)",
        "10-wide final field: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGHSK (`-dhsk`, `JHashPos.h:111-116`): "Hash Key" lines per hashed byte,
/// with the printable-ASCII `%c` filter (space otherwise).
#[test]
fn hsk_hash_key_lines_tiny() {
    let dir = temp_dir("hsk-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dhsk"], "hsk");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.starts_with("Hash Key 68 68 h\n"),
        "first line: {stdout:?}"
    );
    assert!(
        stdout.contains("Hash Key 195e 20  \n"),
        "space byte prints two trailing spaces: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGHSH (`-dhsh`): the constructor's "Hash Ini" line (pointers shape-only)
/// and the per-store "Hash Add" lines (`JHashPos.cpp:124-130`), where the
/// final `%c` is `.` for an empty bucket and `!` for an override.
#[test]
fn hsh_ini_and_add_lines_big() {
    let dir = temp_dir("hsh-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dhsh"], "hshB");
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 970, "Ini + 969 stores: {stdout:?}");
    assert!(
        lines[0]
            .starts_with("Hash Ini sizeof= 4+ 8=12, 8388593 samples, 100663116 bytes, address=0x")
            && lines[0].ends_with("."),
        "Ini line: {stdout:?}"
    );
    assert_eq!(
        lines[1], "Hash Add  5884712         31 c8d9b3a9 .",
        "first store line"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGMCH (`-dmch`): the "Mch Add" line (`JMatchTable.cpp:166-170`) and the
/// table dump line (`JMatchTable.cpp:302-310`). On this fixture exactly two
/// lines are produced; all fields are deterministic.
#[test]
fn mch_lines_big() {
    let dir = temp_dir("mch-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dmch"], "mchB");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout,
        concat!(
            "Mch Add (       282,       282) New (       282,       282) Bse (250)\n",
            "Mch 0*[C       999,       999,       282, 718]       251:0:1024\n",
        ),
        "exact Mch output"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGCMP (`-dcmp`): the check() prologue ("Fnd (…): ", no newline) and
/// result line (`JMatchTable.cpp:398-402,430-437`). Exactly one line here.
#[test]
fn cmp_check_line_big() {
    let dir = temp_dir("cmp-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dcmp"], "cmpB");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout,
        "Fnd (       250,       250,1024,1):        251        251 24 OK! (j)152 == (j)152\n",
        "exact check output"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGDST (`-ddst`, `JDiff.cpp:574-577`): the hashtable distribution over 128
/// buckets after the prescan. Header, sample buckets and the two summary
/// lines are oracle-pinned (positions are `liIdx * 100000/128`).
#[test]
fn dst_distribution_lines_big() {
    let dir = temp_dir("dst-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-ddst"], "dstB");
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2 + 128 + 2, "2 + 128 buckets + 2 summaries");
    // Bucket width = 1000/128 = 7 positions ("Pos=" bounds are idx*7).
    assert_eq!(lines[0], "Hash Dist Overload    = 1");
    assert_eq!(lines[1], "Hash Dist Reliability = 48");
    assert_eq!(
        lines[2],
        "Hash Dist        0 Pos=         0:         7 Cnt=       0 Rlb=-1"
    );
    assert_eq!(
        lines[126],
        "Hash Dist      124 Pos=       868:       875 Cnt=       7 Rlb=1"
    );
    assert_eq!(lines[130], "Hash Dist Avg/Min/Max/% = 6/0/7/100");
    assert_eq!(lines[131], "Hash Dist Load           = 865/8388593=0");
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGBUF seek lines (`JFileAhead.cpp:270-273`) on the 100000-byte pair with
/// an 8 kB buffer: the backtrack verification seeks to 85536 and 60000
/// (oracle-pinned values).
#[test]
fn buf_seek_lines_sek() {
    let dir = temp_dir("buf-sek");
    let (org, new) = sek_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-m", "16", "-dbuf"], "sekB");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("ufFabGet: Seek 0.\n"),
        "first reset: {stdout:?}"
    );
    assert!(
        stdout.contains("ufFabGet: Seek 85536.\n"),
        "verify seek: {stdout:?}"
    );
    assert!(
        stdout.contains("ufFabGet: Seek 60000.\n"),
        "copy seek: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGRED on the sek pair: the memory-EOF line at the file end (width-10
/// position `100000`) and the short-read line at 140000 (fid as `%p`).
#[test]
fn red_eof_lines_sek() {
    let dir = temp_dir("red-sek");
    let (org, new) = sek_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-m", "16", "-dred"], "sekR");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("ufFabGet(Org,    100000,0)->EOF (mem).\n"),
        "file-end line: {stdout:?}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("ufFabGet(0x") && l.ends_with(",    140000,0)->EOF.")),
        "short read at 140000: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGRED soft-ahead out-of-buffer (`-f -m 16 -dred`, `JFileAhead.cpp:165-170`):
/// the EOB line prints the fid's address with `%p` (C++ quirk preserved in
/// shape) and the read type 2.
#[test]
fn red_eob_lines_sek_fast() {
    let dir = temp_dir("red-eob");
    let (org, new) = sek_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-f", "-m", "16", "-dred"], "sekE");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("ufFabGet(0x") && l.ends_with(",     85536,2)->EOB.")),
        "EOB line: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Debug flags are output-neutral: the patch bytes with `-dmch` equal the
/// plain-run patch (the debug flush's `put(ESC, 0, …)` is a writer no-op).
#[test]
fn dmch_leaves_patch_bytes_unchanged() {
    let dir = temp_dir("mch-patch");
    let (org, new) = big_pair();
    let a = write_file(&dir.join("a.bin"), &org);
    let b = write_file(&dir.join("b.bin"), &new);
    let p0 = dir.join("p0.bin");
    let p1 = dir.join("p1.bin");

    for (p, extra) in [(&p0, vec![]), (&p1, vec!["-dmch"])] {
        let mut args: Vec<std::ffi::OsString> =
            extra.iter().map(std::ffi::OsString::from).collect();
        args.extend([
            a.clone().into_os_string(),
            b.clone().into_os_string(),
            p.clone().into_os_string(),
        ]);
        let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
            .args(&args)
            .output()
            .expect("spawn jdiff");
        assert_eq!(out.status.code(), Some(0));
    }
    assert_eq!(fs::read(&p0).unwrap(), PATCH_BIG);
    assert_eq!(
        fs::read(&p1).unwrap(),
        PATCH_BIG,
        "-dmch must not change output"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Every one of the 11 flags is accepted as an option (not a filename) in
/// debug builds, exercised on the 1000-byte pair. (`-ddst` on a file smaller
/// than the 128 buckets divides by zero in the C++ too — SIGFPE, verified on
/// the oracle — so the tiny fixture would crash both implementations.)
#[test]
fn all_eleven_flags_accepted() {
    let dir = temp_dir("flags");
    let (org, new) = big_pair();
    let a = write_file(&dir.join("a.bin"), &org);
    let b = write_file(&dir.join("b.bin"), &new);
    let p = dir.join("p.bin");
    for flag in [
        "-dhsh", "-dahd", "-dcmp", "-dprg", "-dbuf", "-dhsk", "-dahh", "-dbkt", "-dred", "-dmch",
        "-ddst",
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
            .args([
                flag,
                "-do",
                a.as_os_str().to_str().unwrap(),
                b.as_os_str().to_str().unwrap(),
                p.as_os_str().to_str().unwrap(),
            ])
            .output()
            .expect("spawn jdiff");
        assert_eq!(
            out.status.code(),
            Some(0),
            "{flag} must be an option, not a filename: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}
