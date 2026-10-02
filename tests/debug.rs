#![cfg(feature = "debug")]
//! Integration tests for the `debug` cargo feature (spec §18.G): the
//! `-d <name>` CLI syntax, the 11 flag indices and every `#if debug` print
//! site of the 0.8.5 site census, at parity with the C++ `-D_DEBUG` build.
//!
//! Every expected string below is pinned against the 0.8.5 debug-variant
//! oracle (`target/oracle-dbg/jdiff`, the `g++ -g -D_DEBUG` build of
//! `reference/jojodiff-0.8.5`; re-verified byte-for-byte with a full
//! flag × fixture matrix in Task 21). The debug-build `P8zd` width of 10
//! (`%10lld`, `JDefs.h` `#if debug` branch) shows on the first "Input" line,
//! which prints position `-1` (the C++ prints `lzPosOrg - 1` before the
//! first increment — quirk preserved).
//!
//! The tests spawn the real binary with `-c`, which sends the debug stream
//! to stdout (`JDebug::stddbg = stdout`, `main.cpp:360-362`; `-c` replaces
//! the 0.8.1 `-do`), so `Command::output()` captures it deterministically
//! and no test leaks output into the test log. The patch always goes to a
//! file, leaving stdout purely debug output. 0.8.5 CLI syntax: `-d <name>`
//! (the name may also be attached, `-dmch` — one getopt option either way),
//! and unknown names are silently ignored (`main.cpp:446-472`). Pointer
//! values (`%p`, buffer addresses) are inherently non-reproducible; such
//! lines are matched by prefix/suffix only. The dead flags `hsk`, `bkt`,
//! `dst` are accepted with **zero** print sites (§18.G/§21.13); the
//! always-on `getbuf` invariant asserts are pinned from the outside in
//! tests/roundtrip.rs (`t_option_debug_matches_release` — reachable-behavior
//! parity) and stay documented as unreachable on the violation paths (spec
//! §21.3/§21.17).

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

/// Runs `jdiff -c <args...> a b out` with the given fixture pair and returns
/// (stdout-as-utf8, output). The 0.8.5 CLI sends the debug stream to stdout
/// with `-c` (`main.cpp:360-362`; the 0.8.1 `-do` option is gone), and the
/// 0.8.5 `-d <name>` syntax splits the 0.8.1 attached `-dprg` form — the
/// mechanical flag-syntax adaptation of Task 20; Task 21 owns the full
/// debug-surface test rewrite (spec §18.G).
fn run_dbg(dir: &Path, org: &[u8], new: &[u8], extra: &[&str], tag: &str) -> (String, Output) {
    let a = write_file(&dir.join(format!("{tag}a.bin")), org);
    let b = write_file(&dir.join(format!("{tag}b.bin")), new);
    let p = dir.join(format!("{tag}p.bin"));

    let mut args: Vec<std::ffi::OsString> = vec!["-c".into()];
    for tok in extra {
        if let Some(name) = tok.strip_prefix("-d").filter(|n| !n.is_empty()) {
            args.push("-d".into());
            args.push(name.into());
        } else {
            args.push((*tok).into());
        }
    }
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
/// oracle (EQL 250, implicit-MOD 0xc3, EQL 749-run — 10 bytes total). 0.8.5
/// wire format (spec §18.C): the MOD byte follows the EQL record without the
/// 0.8.1 `ESC MOD` pair. Debug flags must not change it (the debug flush's
/// `put(ESC, 0, …)` is a writer no-op).
const PATCH_BIG: &[u8] = &[0xA7, 0xA3, 0xF9, 0xC3, 0xA7, 0xA3, 0xFD, 0x02, 0xED];

/// The tiny-pair patch (ESC EQL 11 "byeby" ESC INS "e"), as pinned in
/// tests/roundtrip.rs (`PATCH_AB`) — used here to show `-d` handling leaves
/// the diff itself untouched.
const PATCH_AB_TINY: &[u8] = &[
    0xA7, 0xA3, 0x0B, b'b', b'y', b'e', b'b', b'y', 0xA7, 0xA5, b'e',
];

/// DBGPRG (`-dprg`, `JDiff.cpp:145-148`): "Input " lines with the debug-width
/// `P8zd` (= 10) positions; the C++ prints `lzPosOrg - 1`, so the first line
/// reports position `-1` (oracle-pinned quirk).
#[test]
fn prg_input_lines_tiny() {
    let dir = temp_dir("prg-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dprg"], "prg");
    assert_eq!(out.status.code(), Some(1));
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
/// with the buffer pointers (shape-only) and size. The 0.8.5 CLI defaults
/// both buffers to 1 MB (`main.cpp:618-620`), so `sze=1048576` (debug-oracle
/// verified); 0.8.1 defaulted to 256 kB.
#[test]
fn buf_open_lines() {
    let dir = temp_dir("buf-open");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dbuf"], "buf");
    assert_eq!(out.status.code(), Some(1));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "exactly two ufFabOpn lines: {stdout:?}");
    assert!(
        lines[0].starts_with("ufFabOpn(Org):(buf=0x") && lines[0].ends_with("sze=1048576)"),
        "Org open line: {stdout:?}"
    );
    assert!(
        lines[1].starts_with("ufFabOpn(New):(buf=0x") && lines[1].ends_with("sze=1048576)"),
        "New open line: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGRED (`-dred`) at 0.8.5 (spec §18.G): the 0.8.1 `ufFabGet` trace sites
/// are gone; the only DBGRED code left in `JFileAhead` is the double-verify
/// block (`JFileAhead.cpp:149-186`), which prints `pos-error !` /
/// `len-error !` / `buf-error !` lines **only** when the buffer logic or its
/// contents are wrong. On the tiny pair the debug stream must therefore stay
/// completely empty — a negative invariant over every buffer read.
#[test]
fn red_double_verify_silent_tiny() {
    let dir = temp_dir("red-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dred"], "red");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout.is_empty(),
        "no DBGRED output expected (sites removed at 0.8.5, double-verify\n\
         silent on correct buffer contents): {stdout:?}"
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
    assert_eq!(out.status.code(), Some(1));
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

/// DBGAHD (`-dahd`, `JDiff.cpp:281-283`): the find-ahead summary. On this
/// single-edit fixture exactly one line is produced, with the same values as
/// the 0.8.1 pin (pinned against the 0.8.5 C++ engine's debug build).
#[test]
fn ahd_findahead_line_big() {
    let dir = temp_dir("ahd-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dahd"], "ahdB");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout, "Findahead on 250 250 skip 0 0 ahead 1\n",
        "exact find-ahead line"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGAHH (`-dahh`) in the 0.8.5 `buildFullIndex` (`JDiff.cpp:758-762`): the
/// site lives in the **verbose>1** slow loop only (0.8.1 printed it at any
/// verbosity), so the run needs `-vv`; both trailing fields are `%8d` /
/// `P8zd` as before. First and last line pinned against the 0.8.5 C++
/// engine's debug build (oracle harness, `-D_DEBUG` + 32-bit `hkey`): the
/// eql-aware hash leaves the first window's key unchanged (no equal-adjacent
/// bytes → eql stays 0) but shifts the last one (`956ca3a4` → `957ca3a4`,
/// the 0.8.1 pin).
#[test]
fn ahh_prescan_lines_big() {
    let dir = temp_dir("ahh-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-vv", "-dahh"], "ahhB");
    assert_eq!(out.status.code(), Some(1));
    // Filter out the verbose>1 engine/CLI text mixed into the stream; the
    // "Indexing  : ...           " marker shares the first line with the
    // first trace (no newline in between, 1:1 with the C++), so each line
    // is trimmed to its ufHshAdd trace.
    let lines: Vec<&str> = stdout
        .lines()
        .filter_map(|l| {
            let i = l.find("ufHshAdd")?;
            Some(&l[i..])
        })
        .collect();
    // The buildFullIndex init loop consumes the first SMPSZE-1 = 31 bytes,
    // so the add loop stores samples at positions 31..=999: 969 lines.
    assert_eq!(lines.len(), 969, "one line per stored sample");
    assert_eq!(lines[0], "ufHshAdd(d5 -> c8d9b3a9,         31,        0)");
    assert_eq!(
        lines[lines.len() - 1],
        "ufHshAdd(9a -> 957ca3a4,        999,        0)"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGAHH in the search loop: 0.8.5 has **no DBGAHH site left there** (the
/// 0.8.1 find-ahead trace `JDiff.cpp:388-392` is gone with the search()
/// rewrite; spec §18.G lists only `JDiff.cpp:759`). `-ff -dahh` therefore
/// stays silent.
#[test]
fn ahh_findahead_lines_ff() {
    let dir = temp_dir("ahh-ff");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-ff", "-dahh"], "ahhFF");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout.is_empty(),
        "no DBGAHH site in the 0.8.5 search loop: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGHSK (`-dhsk`) and DBGBKT (`-dbkt`) at 0.8.5: the hash moved from
/// `JHashPos` into `JDiff` (`JDiff.cpp:361-371`) and the "Hash Key" trace
/// site was dropped; the bucket/seek trace never existed at 0.8.5 — both
/// flags are accepted with **zero print sites** (spec §18.G/§21.13).
#[test]
fn dead_flags_hsk_bkt_are_silent() {
    let dir = temp_dir("hsk-tiny");
    for (tag, flag) in [("hsk", "-dhsk"), ("bkt", "-dbkt")] {
        let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &[flag], tag);
        assert_eq!(out.status.code(), Some(1));
        assert!(
            stdout.is_empty(),
            "{flag} has zero print sites at 0.8.5: {stdout:?}"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// Unknown `-d <name>` names are **silently ignored** (`main.cpp:446-472`:
/// the strcmp chain simply matches nothing — there is no else arm), and the
/// diff runs normally. Oracle-verified (`-d bogus` matches the C++ byte
/// stream, empty debug output, exit 1).
#[test]
fn unknown_name_is_silently_ignored() {
    let dir = temp_dir("d-bogus");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-d", "bogus"], "bog");
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.is_empty(), "no output for an unknown name: {stdout:?}");
    // The patch is the normal tiny-pair patch (the run is a plain diff).
    assert_eq!(fs::read(dir.join("bogp.bin")).unwrap(), PATCH_AB_TINY);
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGHSH (`-dhsh`): the constructor's "Hash Ini" line (pointers shape-only)
/// and the per-store "Hash Add" lines (`JHashPos.cpp:127-132`), where the
/// final `%c` is `.` for an empty bucket and `!` for an override.
///
/// 0.8.5 MB sizing (spec §18.E): the run pins the CLI default-equivalent
/// element count via `-i 8` (8*1024*1024/16 = 524288 elements → prime
/// 524287, size in bytes prime*12 = 6291444) — element-count-equivalent to
/// the 32-bit-hkey debug oracle's `-i 6` (6*1024*1024/12), re-verified
/// byte-for-byte today ("Hash Ini sizeof= 4+ 8=12, 524287 samples,
/// 6291444 bytes" and first store "Hash Add   117956         31 c8d9b3a9 .").
#[test]
fn hsh_ini_and_add_lines_big() {
    let dir = temp_dir("hsh-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-i", "8", "-dhsh"], "hshB");
    assert_eq!(out.status.code(), Some(1));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 970, "Ini + 969 stores: {stdout:?}");
    assert!(
        lines[0]
            .starts_with("Hash Ini sizeof= 4+ 8=12, 524287 samples, 6291444 bytes, address=0x")
            && lines[0].ends_with("."),
        "Ini line: {stdout:?}"
    );
    assert_eq!(
        lines[1], "Hash Add   117956         31 c8d9b3a9 .",
        "first store line"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGMCH (`-dmch`), 0.8.5 formats (Task 16 port): the isBest election line
/// (`JMatchTable.cpp:633-642`: "Val/Old/Inv", compare, '*' when elected, the
/// node dump), the "Add" line (`:349-354`, `ret=` is the eMatchReturn
/// discriminant — 6 = Valid), the "Mch Old Max Distance" aging line and the
/// getbest verdict (`:153-168`). Pinned against the 0.8.5 C++ engine's
/// debug build: the 0.8.5 search's look-back re-init also adds the match at
/// 171 (found during the backward scan), which the table elects first; the
/// getbest verdict still selects the verified 251 match. (The 0.8.1 pins
/// "Mch Add (...) New (...) Bse (...)" and the per-candidate "Mch 0*[C ...]"
/// dump are gone at 0.8.5.)
#[test]
fn mch_lines_big() {
    let dir = temp_dir("mch-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dmch"], "mchB");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout,
        concat!(
            "Val   256 * [ 0:       171>         0<       171~       171#   1:       251+ 256] bse=250 fnd=251=251(1)\n",
            "Mch Old Max Distance = 79\n",
            "Add         [  :       171>         0<       171] bse=250 ret=6\n",
            "Suboptimal Match at 250: from 251(1), length 256\n",
        ),
        "exact Mch output"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGMCH on the tiny pair (`JMatchTable.cpp:156`): the table is empty at the
/// single mismatch (12), so getbest elects nothing and the debug stream is
/// exactly the "Match Failure at" verdict line — the brief's new-format pin,
/// oracle-verified (`target/oracle-dbg/jdiff -c -dmch` prints the same single
/// line, exit 1).
#[test]
fn mch_match_failure_tiny() {
    let dir = temp_dir("mch-tiny");
    let (stdout, out) = run_dbg(&dir, ORG_A, NEW_B, &["-dmch"], "mchT");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout, "Match Failure at 12\n", "exact getbest failure line");
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGCMP (`-dcmp`), 0.8.5 formats (Task 16 port): the check() prologue
/// ("Cmp Col|Gld (…): " — `JMatchTable.cpp:825-830`, was "Fnd (…): ") and
/// the result line (`:857-864`, run capped at EQLMAX 256 — was 24 — with
/// the bytes printed `%02x` hex, was `%3o` octal). Exactly one line here,
/// identical to the 0.8.1-engine pin (pinned against the 0.8.5 C++
/// engine's debug build).
#[test]
fn cmp_check_line_big() {
    let dir = temp_dir("cmp-big");
    let (org, new) = big_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-dcmp"], "cmpB");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout,
        "Cmp Col (       250,       250,1024,1):        251        251 256 OK! ( )b9 == ( )b9\n",
        "exact check output"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGDST (`-ddst`) at 0.8.5: the flag is accepted but has **zero print
/// sites** (spec §18.G/§21.13). The 0.8.1 site (distribution after the
/// prescan, 128 buckets) is gone with the 0.8.5 prescan; `dist()` now serves
/// only the release-visible verbose>2 call sites (`JDiff.cpp:324-327,784-787`,
/// 10 buckets), which need `-vvv`, not `-ddst` — that block is pinned
/// oracle-exact in tests/roundtrip.rs
/// (`inaccurate_solution_lines_at_verbose_3`). The debug stream must stay
/// completely empty.
#[test]
fn dst_flag_is_silent() {
    let dir = temp_dir("dst-big");
    let org = lcg(1, 100_000);
    let mut new = org.clone();
    new[250] ^= 0xFF;
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-ddst"], "dstB");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout.is_empty(),
        "-ddst has zero print sites at 0.8.5: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGBUF at 0.8.5 on the 100000-byte pair with an 8 MB buffer (`-m 16`):
/// the 0.8.1 `ufFabGet: Seek` trace site is gone — the only DBGBUF output is
/// the two `ufFabOpn` lines, even across the heavy reset/scrollback traffic
/// of this pair. The sek pair is a pure block move, so the patch holds no
/// data bytes (`dta == 0`) and the 0.8.5 swapped mapping exits 0
/// ("all data found within source", main.cpp:921-924) despite the non-empty
/// 29-byte EQL/DEL/BKT patch.
#[test]
fn buf_open_only_no_seek_lines_sek() {
    let dir = temp_dir("buf-sek");
    let (org, new) = sek_pair();
    let (stdout, out) = run_dbg(&dir, &org, &new, &["-m", "16", "-dbuf"], "sekB");
    assert_eq!(out.status.code(), Some(0), "EQL-only patch: dta == 0");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "only the two ufFabOpn lines (Seek trace removed at 0.8.5): {stdout:?}"
    );
    assert!(
        lines[0].starts_with("ufFabOpn(Org):(buf=0x"),
        "Org open line: {stdout:?}"
    );
    assert!(
        lines[1].starts_with("ufFabOpn(New):(buf=0x"),
        "New open line: {stdout:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// DBGRED on the sek pair (plain and with `-f`, exercising reset, scrollback,
/// EOF latching and soft-ahead bounds): the double-verify block stays silent
/// — the buffer contents match fresh reads everywhere, and EOF/EOB are no
/// longer traced at 0.8.5. (Exit 0: the EQL-only block-move patch carries no
/// data bytes, `dta == 0` → EXI_EQL under the swapped mapping.)
#[test]
fn red_double_verify_silent_sek() {
    let dir = temp_dir("red-sek");
    let (org, new) = sek_pair();
    for (tag, extra) in [("sekR", vec!["-m", "16"]), ("sekE", vec!["-f", "-m", "16"])] {
        let mut opts = extra;
        opts.push("-dred");
        let (stdout, out) = run_dbg(&dir, &org, &new, &opts, tag);
        assert_eq!(out.status.code(), Some(0), "{tag}: EQL-only patch");
        assert!(
            stdout.is_empty(),
            "{tag}: no DBGRED output expected (sites removed at 0.8.5,\n\
             double-verify silent on correct buffer contents): {stdout:?}"
        );
    }
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
        assert_eq!(out.status.code(), Some(1));
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
/// debug builds, exercised on the 1000-byte pair — except `-ddst`, which
/// needs the 100000-byte pair: the 0.8.5 `dist()` Avg/Min/Max guard does not
/// cover `liMax < 100` (inner `liMax/100` = 0 → C++ SIGFPE, port panics on
/// the same division), and the 1000-byte pair tops out at `liMax` 7 over 128
/// buckets. (`-ddst` on a file smaller than the 128 buckets divides by zero
/// in the fill loop in the C++ too — SIGFPE, verified on the 0.8.1 oracle.)
/// 0.8.5 syntax: `-d <name>` consumes the name; `-c` redirects the stream.
#[test]
fn all_eleven_flags_accepted() {
    let dir = temp_dir("flags");
    let (org, new) = big_pair();
    let a = write_file(&dir.join("a.bin"), &org);
    let b = write_file(&dir.join("b.bin"), &new);
    let p = dir.join("p.bin");
    for flag in [
        "hsh", "ahd", "cmp", "prg", "buf", "hsk", "ahh", "bkt", "red", "mch",
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
            .args([
                "-d",
                flag,
                "-c",
                a.as_os_str().to_str().unwrap(),
                b.as_os_str().to_str().unwrap(),
                p.as_os_str().to_str().unwrap(),
            ])
            .output()
            .expect("spawn jdiff");
        assert_eq!(
            out.status.code(),
            Some(1),
            "-d {flag} must be an option, not a filename: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // -d dst on the 100000-byte pair (liMax 781 >= 100: no SIGFPE path).
    let org = lcg(1, 100_000);
    let mut new = org.clone();
    new[250] ^= 0xFF;
    let a = write_file(&dir.join("a-big.bin"), &org);
    let b = write_file(&dir.join("b-big.bin"), &new);
    let out = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args([
            "-d",
            "dst",
            "-c",
            a.as_os_str().to_str().unwrap(),
            b.as_os_str().to_str().unwrap(),
            p.as_os_str().to_str().unwrap(),
        ])
        .output()
        .expect("spawn jdiff");
    assert_eq!(
        out.status.code(),
        Some(1),
        "-d dst must be an option, not a filename: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::remove_dir_all(&dir).unwrap();
}
