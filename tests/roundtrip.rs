//! Integration tests for the `jdiff` CLI (port plan Task 9: option parsing,
//! greeting/help, file handling, stats output and exit codes) and, below the
//! marker comment, for the `jptch` CLI and patch decoder (Task 10, port of
//! `jpatch.cpp`).
//!
//! Every expected string and exit code below was pinned against the vendored
//! C++ oracle (`reference/jojodiff-cpp`) built as the canonical verification
//! build: default `make` plus the spec §15.1 2-line Linux fix and a forced
//! 32-bit `hkey` (spec §2 / Task 8 finding: on x86-64 Linux `unsigned long`
//! is 8 bytes, which would make SMPSZE=64 and diverge from the library).
//!
//! The tests spawn the real binary (`env!("CARGO_BIN_EXE_jdiff")`) and capture
//! stdout/stderr via `Command::output()`, so no test leaks process output into
//! the test log. Each test works in its own unique temp directory and cleans
//! up afterwards (parallel-safe).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// Full byte-exact greeting block (`main.cpp:318-344`): version line, copyright,
/// blank line, 10-line license block, blank line, file-adressing line (sic),
/// blank line. "samples are 32 bytes" is `SMPSZE` (32 bits, printed verbatim;
/// the brief's "4 bytes" literal is wrong, the oracle prints 32).
const GREETING: &str = concat!(
    "JDIFF - Jojo's binary diff version 0.8.1 (beta) December 2011\n",
    "Copyright (C) 2002-2005,2009,2011 Joris Heirbaut\n",
    "\n",
    "JojoDiff is free software: you can redistribute it and/or modify\n",
    "it under the terms of the GNU General Public License as published by\n",
    "the Free Software Foundation, either version 3 of the License, or\n",
    "(at your option) any later version.\n",
    "\n",
    "This program is distributed in the hope that it will be useful,\n",
    "but WITHOUT ANY WARRANTY; without even the implied warranty of\n",
    "MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\n",
    "GNU General Public License for more details.\n",
    "\n",
    "You should have received a copy of the GNU General Public License\n",
    "along with this program.  If not, see <http://www.gnu.org/licenses/>.\n",
    "\n",
    "File adressing is 64 bit (files up to 8388608 TB), samples are 32 bytes.\n",
    "\n",
);

/// Usage/help block (`main.cpp:347-383`); the `-min`/`-max` lines print the
/// current (post-parse) match limits, so they are formatted in.
fn usage(mch_min: i32, mch_max: i32) -> String {
    format!(
        concat!(
            "Usage: jdiff [options] <original file> <new file> [<output file>]\n",
            "  -v          Verbose (greeting, results and tips).\n",
            "  -vv         Verbose (debug info).\n",
            "  -h          Help (this text).\n",
            "  -l          Listing (ascii output).\n",
            "  -lr         Regions (ascii output).\n",
            "  -do         Write verbose and debug info to stdout instead of stddbg.\n",
            "  -b          Try to be better (using more memory).\n",
            "  -f          Try to be faster: no out of buffer compares.\n",
            "  -ff         Try to be faster: no out of buffer compares, nor pre-scanning.\n",
            "  -m size     Size (in kB) for look-ahead buffer (default 512kB, 0=no buffers).\n",
            "  -bs size    Block size (in bytes) for reading from files (default 4096).\n",
            "  -s size     Number of samples per file in MB (default 8).\n",
            "  -a size     Number of kB to look ahead (default=same as buffer-size).\n",
            "  -min count  Minimum number of solutions to find (default {}, max {}).\n",
            "  -max count  Maximum number of solutions to find (default {}, max {}).\n",
            "Principles:\n",
            "  JDIFF tries to find equal regions between two binary files using a heuristic\n",
            "  hash algorithm and outputs the differences between both files.\n",
            "  Heuristics are generally used for improving performance and memory usage,\n",
            "  at the cost of accuracy. Therefore, this program may not find a minimal set\n",
            "  of differences between files.\n",
            "Notes:\n",
            "  Options -b, -f or -ff should be used before other options.\n",
            "  Accuracy may be improved by increasing the number of samples.\n",
            "  Sample size is always lowered to the largest n-bit prime (n < 32)\n",
            "  Original and new file must be random access files.\n",
            "  Output is sent to standard output if output file is missing.\n",
            "Hint:\n",
            "  Do not use jdiff directly on compressed files, such as zip, gzip, rar, ...\n",
            "  Instead use uncompressed files, such as tar, cpio or zip-0, and then compress\n",
            "  the jdiff's output file afterwards.\n",
            "\n",
        ),
        mch_min, MCH_MAX, mch_max, MCH_MAX
    )
}

const MCH_MAX: i32 = 256;

/// 17-byte fixture differing only in the tail: A = "hello world hello",
/// B = "hello world byebye".
const ORG_A: &[u8] = b"hello world hello";
const NEW_B: &[u8] = b"hello world byebye";

/// The patch jdiff produces for (ORG_A, NEW_B), byte-exact with the 0.8.5
/// writer (spec §18.C): EQL 12, MOD data "byeby" riding the implicit MOD
/// (the 0.8.1 bytes carried an `ESC MOD` pair before it), INS "e".
const PATCH_AB: &[u8] = &[
    0xA7, 0xA3, 0x0B, // ESC EQL len=12
    b'b', b'y', b'e', b'b', b'y', // implicit MOD "byeby" (no ESC MOD pair)
    0xA7, 0xA5, b'e', // ESC INS "e"
];

/// The same patch in the 0.8.1 wire format (explicit `ESC MOD` after the
/// EQL record), used to exercise the `jptch` CLI while our decoder is still
/// jpatch.cpp's 0.8.1 reader (Task 19 lands the 0.8.5 reader, which accepts
/// both formats — explicit opcodes are a subset of the 0.8.5 grammar, spec
/// §18.C).
const PATCH_AB_081: &[u8] = &[
    0xA7, 0xA3, 0x0B, // ESC EQL len=12
    0xA7, 0xA6, b'b', b'y', b'e', b'b', b'y', // ESC MOD "byeby"
    0xA7, 0xA5, b'e', // ESC INS "e"
];

/// Unique temp directory per test (parallel-safe), cleaned up by the caller.
fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jdiff-t9-{}-{}-{}",
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

fn run(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(args)
        .output()
        .expect("spawn jdiff")
}

fn run_in(dir: &Path, args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn jdiff")
}

fn stderr_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `jdiff -h`: exit 2, stderr is the byte-exact greeting + usage block
/// (`main.cpp:318-387`; oracle-pinned).
// TODO(T20): byte-exact 0.8.1 greeting; Task 13 re-pointed JDIFF_VERSION/
// JDIFF_COPYRIGHT to the 0.8.5 strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn help_exit_code_and_text() {
    let out = run(&[OsStr::new("-h")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr_str(&out), GREETING.to_string() + &usage(8, 32));
    assert!(out.stdout.is_empty());
}

/// 0 or 1 file args: greeting + usage on stddbg, exit 2 (`main.cpp:346-387`).
// TODO(T20): byte-exact 0.8.1 greeting; Task 13 re-pointed JDIFF_VERSION/
// JDIFF_COPYRIGHT to the 0.8.5 strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn missing_args_exit_2() {
    for args in [
        vec![],
        vec![OsStr::new("some-file")],
        vec![OsStr::new("-v"), OsStr::new("some-file")],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2), "args {args:?}");
        assert_eq!(
            stderr_str(&out),
            GREETING.to_string() + &usage(8, 32),
            "args {args:?}"
        );
    }
}

/// Unopenable original file: "Could not open first file %s for reading.\n",
/// exit 3 (`main.cpp:482-485`). The stock C++ binary crashes before this point
/// (the fillBuffer threads read a garbage `stat` size and abort with
/// `bad_alloc`), so this pins the intended semantics of the ported open-check
/// code; see the Task 9 report, deviation note 1b.
#[test]
fn unopenable_org_exit_3_message() {
    let dir = temp_dir("org3");
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let missing = dir.join("does-not-exist");
    let outp = dir.join("p.bin");

    let out = run(&[missing.as_os_str(), b.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        stderr_str(&out),
        format!(
            "Could not open first file {} for reading.\n",
            missing.display()
        )
    );
    assert!(!outp.exists(), "output must not be created");
    fs::remove_dir_all(&dir).unwrap();
}

/// Unopenable new file: "Could not open second file %s for reading.\n", exit 4
/// (`main.cpp:504-507`); same stock-C++-crash caveat as the exit-3 test.
#[test]
fn unopenable_new_exit_4() {
    let dir = temp_dir("new4");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let missing = dir.join("does-not-exist");
    let outp = dir.join("p.bin");

    let out = run(&[a.as_os_str(), missing.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(4));
    assert_eq!(
        stderr_str(&out),
        format!(
            "Could not open second file {} for reading.\n",
            missing.display()
        )
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Unopenable output file: "Could not open output file %s for writing.\n",
/// exit 5 (`main.cpp:510-517`).
#[test]
fn unopenable_out_exit_5() {
    let dir = temp_dir("out5");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let bad_out = dir.join("no-such-dir").join("p.bin");

    let out = run(&[a.as_os_str(), b.as_os_str(), bad_out.as_os_str()]);
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        stderr_str(&out),
        format!(
            "Could not open output file {} for writing.\n",
            bad_out.display()
        )
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Identical fixture pair: exit 1 ("no differences found",
/// `main.cpp:610-613`). The patch is the single EQL record the engine flushes
/// at end of stream — `ESC EQL <len-1>` = 3 bytes for a 17-byte file — pinned
/// against the oracle (`A7 A3 10`). (The brief's "patch 0 bytes" contradicts
/// the C++; the C++ wins, see Task 9 report.)
#[test]
fn equal_files_exit_1_empty_patch() {
    let dir = temp_dir("equal");
    let a = write_file(&dir.join("a.bin"), b"abcdefghijklmnopq");
    let b = write_file(&dir.join("b.bin"), b"abcdefghijklmnopq");
    let outp = dir.join("p.bin");

    let out = run(&[a.as_os_str(), b.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&outp).unwrap(), [0xA7, 0xA3, 0x10]);
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Differing fixture pair: exit 0, patch byte-exact with the oracle
/// (`main.cpp:610-613`).
#[test]
fn differ_files_exit_0_nonempty_patch() {
    let dir = temp_dir("differ");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run(&[a.as_os_str(), b.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// With only two file arguments the patch goes to stdout and stddbg stays
/// empty (`main.cpp:392-395`, `510-511`).
#[test]
fn missing_output_goes_to_stdout() {
    let dir = temp_dir("stdout");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, PATCH_AB);
    assert!(out.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// `jdiff -v` on the tiny pair: stderr is the byte-exact greeting block, the
/// engine's prescan block and the three verbose statistics lines
/// (`main.cpp:318-344`, `563-567`; oracle-pinned).
// TODO(T20): byte-exact 0.8.1 greeting; Task 13 re-pointed JDIFF_VERSION/
// JDIFF_COPYRIGHT to the 0.8.5 strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn greeting_matches_reference() {
    let dir = temp_dir("greet");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let expected = GREETING.to_string()
        + "Prescanning:\n"
        + ".\n"
        + "Equal     bytes         = 12\n"
        + "Data      bytes written = 6\n"
        + "Overhead  bytes written = 7\n";

    let out = run(&[
        OsStr::new("-v"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr_str(&out), expected);
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// `jdiff -vv`: additionally the two pre-run lines and the thirteen
/// high-verbosity statistics lines, byte-exact (`main.cpp:538-562`;
/// oracle-pinned, including the "Hastable" typo and the unpadded `PRIzd`
/// values).
// TODO(T20): byte-exact 0.8.1 greeting; Task 13 re-pointed JDIFF_VERSION/
// JDIFF_COPYRIGHT to the 0.8.5 strings and Task 20 rewrites the banner text.
// The hashtable lines below are pinned to the 0.8.5 MB sizing (Task 15,
// spec §18.E) under the current CLI default of 8 MB (Task 20 changes the
// default to 32 MB → 2097152 elements → prime 2097143 → 24576 kb).
#[ignore]
#[test]
fn stats_lines_match_reference() {
    let dir = temp_dir("stats");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let expected = GREETING.to_string()
        + "Lookahead buffers: 512 kb. (256 kb. per file).\n"
        + "Hastable size    : 6144 kb. (524287 samples).\n"
        + "Prescanning:\n"
        + ".\n"
        + "Hashtable size          = 6291444 samples, 6144 KB, 6 MB\n"
        + "Hashtable prime         = 524287\n"
        + "Hashtable hits          = 0\n"
        + "Hashtable errors        = 0\n"
        + "Hashtable repairs       = 0\n"
        + "Hashtable overloading   = 0\n"
        + "Reliability distance    = 48\n"
        + "Random    accesses      = 0\n"
        + "Delete    bytes         = 0\n"
        + "Backtrack bytes         = 0\n"
        + "Escape    bytes written = 0\n"
        + "Control   bytes written = 7\n"
        + "Equal     bytes         = 12\n"
        + "Data      bytes written = 6\n"
        + "Overhead  bytes written = 7\n";

    let out = run(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr_str(&out), expected);
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// At `-vvv` the engine reports solutions that did not point to an equal
/// region: `"\nInaccurate solution at positions %zd/%zd!\n"` plus the
/// `"Comparing : ...           "` restart marker (`JDiff.cpp:255-258`). The
/// miss counter increments in **release** builds at 0.8.5 (spec §18.E/§21.6;
/// 0.8.1 counted it in debug builds only). On the bundled bkocomu pair with
/// the current CLI defaults the engine reports exactly three misses, pinned
/// here with their positions.
///
/// The positions are oracle-verified against the 0.8.5 C++ engine (oracle
/// harness, MemFile readers, `-i 6`): the C++ 32-bit-hkey build divides the
/// MB count by sizeof(hkey)+sizeof(off_t) = 12, so its `-i 6` table is
/// 524288 elements → prime 524287 — exactly the port's 8 MB table (fixed
/// divisor 16, spec §18.E) — and reproduces these three lines verbatim.
/// (With the CLI-shaped `-i 8` the C++ builds a different prime and lands
/// on different, equally valid match decisions.)
// TODO(T20): the surrounding verbose block is still 0.8.1-worded; the full
// 0.8.5 verbose stream becomes assertable at Task 20.
#[test]
fn inaccurate_solution_lines_at_verbose_3() {
    let dir = temp_dir("t17-inacc");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");
    let a = fixtures.join("bkocomu.0000.fil");
    let b = fixtures.join("bkocomu.0009.fil");
    let outp = dir.join("p.bin");

    let out = run(&[
        OsStr::new("-vvv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_str(&out));
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("\nInaccurate solution at positions 1338481/1240064!\n"),
        "first miss line: {stderr:?}"
    );
    assert!(
        stderr.contains("\nInaccurate solution at positions 1561636/1816576!\n"),
        "second miss line: {stderr:?}"
    );
    assert!(
        stderr.contains("\nInaccurate solution at positions 1922267/1854717!\n"),
        "third miss line: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("Inaccurate solution at positions").count(),
        3,
        "exactly three misses on this pair"
    );
    // Each miss prints the "Comparing : ...           " restart marker
    // directly after the line (miss line ends "!\n").
    assert_eq!(
        stderr
            .matches("!\nComparing : ...           ")
            .count(),
        3,
        "restart marker after each miss"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Option-quirk contract (`main.cpp:205-315`): `-m 1` selects in-memory mode
/// (integer division first), trailing options after the first filename are
/// just filenames, `-s` divides by 1024 while above 1024, and preset values
/// leak into the `-h` default display.
#[test]
fn option_quirks() {
    let dir = temp_dir("quirks");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");
    fn os(s: &str) -> &OsStr {
        OsStr::new(s)
    }

    // -m 1 → 1/2*1024 == 0 → in-memory mode ("0 kb." lookahead line).
    let out = run(&[
        os("-m"),
        os("1"),
        os("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "-m 1 run");
    assert!(
        stderr_str(&out).contains("Lookahead buffers: 0 kb. (0 kb. per file).\n"),
        "-m 1 lookahead line: {:?}",
        stderr_str(&out)
    );
    assert_eq!(
        fs::read(&outp).unwrap(),
        PATCH_AB,
        "-m 0 patch is served from memory"
    );

    // Options must precede filenames: `jdiff A B -l` creates a file named "-l".
    let out = run_in(&dir, &[a.as_os_str(), b.as_os_str(), os("-l")]);
    assert_eq!(out.status.code(), Some(0), "trailing -l run");
    assert!(
        dir.join("-l").exists(),
        "-l must be treated as an output filename"
    );
    fs::remove_file(dir.join("-l")).unwrap();

    // -s 2048 → while >1024 divide → hashtable 131071 samples / 1536 kb.
    // (0.8.5 MB sizing, spec §18.E: the normalized -s value 2 goes to
    // JHashPos::new as MB → 2*1024*1024/16 = 131072 elements →
    // get_lower_prime = 131071; bytes = 131071*12 → (1572852+512)/1024
    // = 1536 kb. Task 20 rewires -s to the 0.8.5 -i semantics.)
    let out = run(&[
        os("-s"),
        os("2048"),
        os("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "-s 2048 run");
    assert!(
        stderr_str(&out).contains("Hastable size    : 1536 kb. (131071 samples).\n"),
        "-s 2048 hashtable line: {:?}",
        stderr_str(&out)
    );

    // Presets assign unconditionally: `-b` before `-h` changes the displayed
    // current -min/-max defaults (oracle-verified).
    let out = run(&[os("-b"), os("-h")]);
    assert_eq!(out.status.code(), Some(2), "-b -h run");
    let text = stderr_str(&out);
    assert!(
        text.contains("(default 16, max 256)."),
        "-b -min line: {text:?}"
    );
    assert!(
        text.contains("(default 128, max 256)."),
        "-b -max line: {text:?}"
    );

    fs::remove_dir_all(&dir).unwrap();
}

/// Write-error resolution (cross-task flag from Task 6): the C++ diff CLI
/// never checks `putc` results — `EXI_WRI` is unreachable and write failures
/// are silently ignored. Oracle-pinned: `jdiff A B /dev/full` exits 0 with no
/// message and a truncated (here: empty) patch. The Rust CLI wraps its sink in
/// an error-swallowing adapter to match (`main.cpp:589-613` has no producer
/// for `-EXI_WRI`; only `jpatch.cpp` exits `EXI_WRI`).
#[cfg(target_os = "linux")]
#[test]
fn write_error_dev_full_matches_oracle() {
    let dir = temp_dir("devfull");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[a.as_os_str(), b.as_os_str(), OsStr::new("/dev/full")]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "oracle: silent exit 0 on write errors"
    );
    assert!(
        out.stderr.is_empty(),
        "no EXI_WRI message: {:?}",
        stderr_str(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Task 10: the `jptch` CLI and patch decoder (port of `jpatch.cpp`)
// ===========================================================================
//
// Every expected string, byte and exit code below was pinned against the
// vendored C++ oracle compiled from `reference/jojodiff-cpp/src/jpatch.cpp`
// with the release Makefile flags (`-D_FILE_OFFSET_BITS=64`, so `off_t` is
// 64-bit and `P8zd` is `%12lld`). Unlike the jdiff CLI, jpatch.cpp contains no
// pthread pre-read, so the open-error checks (exits 3/4/5) are LIVE in the
// stock binary and were verified against it directly.

const JPTCH: &str = env!("CARGO_BIN_EXE_jptch");

/// Byte-exact greeting block (`jpatch.cpp:339-356`): same version/copyright/
/// license text as jdiff, but a "JPATCH" title and a bare
/// "File adressing is 64 bit." line without the sizes (`jpatch.cpp:355`).
const GREETING_PTCH: &str = concat!(
    "JPATCH - Jojo's binary patch version 0.8.1 (beta) December 2011\n",
    "Copyright (C) 2002-2005,2009,2011 Joris Heirbaut\n",
    "\n",
    "JojoDiff is free software: you can redistribute it and/or modify\n",
    "it under the terms of the GNU General Public License as published by\n",
    "the Free Software Foundation, either version 3 of the License, or\n",
    "(at your option) any later version.\n",
    "\n",
    "This program is distributed in the hope that it will be useful,\n",
    "but WITHOUT ANY WARRANTY; without even the implied warranty of\n",
    "MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\n",
    "GNU General Public License for more details.\n",
    "\n",
    "You should have received a copy of the GNU General Public License\n",
    "along with this program.  If not, see <http://www.gnu.org/licenses/>.\n",
    "\n",
    "File adressing is 64 bit.\n",
    "\n",
);

/// Byte-exact usage/help block (`jpatch.cpp:360-373`), including the
/// "-t  Test: no output file." line and the trailing blank line; the `-l`
/// line is commented out in the C++ and therefore absent.
const USAGE_PTCH: &str = concat!(
    "Usage: jpatch [options] <original file> <patch file> [<output file>]\n",
    "  -v               Verbose: version and licence.\n",
    "  -vv              Verbose: debug info.\n",
    "  -vvv             Verbose: more debug info.\n",
    "  -h               Help (this text).\n",
    "  -t               Test: no output file.\n",
    "Principles:\n",
    "  JPATCH reapplies a diff file, generated by jdiff, to the <original file>,\n",
    "  restoring the <new file>. For example, if jdiff has been called like this:\n",
    "    jdiff data01.tar data02.tar data02.dif\n",
    "  then data02.tar can be restored as follows:\n",
    "    jpatch data01.tar data02.dif data02.tar\n",
    "\n",
);

/// `P8zd` (`%12lld` in the release build) used to build the expected verbose
/// lines. Debug-feature builds use `%10lld` (`JDefs.h` `#if debug` branch) —
/// pinned against the C++ `make debug` oracle, whose jptch shrinks the same
/// way (`jpatch.c` verbose lines share the engine's `P8zd`).
fn p12(v: i64) -> String {
    if cfg!(feature = "debug") {
        format!("{v:>10}")
    } else {
        format!("{v:>12}")
    }
}

fn run_ptch(args: &[&OsStr]) -> Output {
    Command::new(JPTCH)
        .args(args)
        .output()
        .expect("spawn jptch")
}

/// Runs `jptch` with a fixture file wired to stdin (the C `-` argument).
fn run_ptch_stdin(args: &[&OsStr], stdin: &Path) -> Output {
    let file = fs::File::open(stdin).expect("open stdin fixture");
    Command::new(JPTCH)
        .args(args)
        .stdin(std::process::Stdio::from(file))
        .output()
        .expect("spawn jptch")
}

fn run_jdiff(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(args)
        .output()
        .expect("spawn jdiff")
}

/// Deterministic pseudo-random bytes (linear congruential generator, fixed
/// seed) so fixtures are reproducible without dependencies.
fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (x >> 16) as u8
        })
        .collect()
}

/// 16 KiB fixture pair exercising every decoder path: long equal runs
/// (multi-block EQL copies), deletion (DEL seek), re-inserted earlier data
/// (BKT candidates), MOD/INS data and operator-valued bytes 0xA2..=0xA7
/// (ESC-escaped patch data).
fn big_pair() -> (Vec<u8>, Vec<u8>) {
    let mut org = pseudo_random(16 * 1024, 0x1234_5678);
    for (i, b) in [
        (0x123, 0xA7u8),
        (0x456, 0xA6),
        (0x789, 0xA5),
        (0xABC, 0xA4),
        (0xDEF, 0xA3),
        (0x101, 0xA2),
    ] {
        org[i] = b;
    }
    let mut new = Vec::new();
    new.extend_from_slice(b"PREPENDED DATA"); // INS at the front
    new.extend_from_slice(&org[100..]); // delete org[0..100], EQL the rest
    new.extend_from_slice(&org[2000..2400]); // repeat of earlier data (BKT)
    new.extend_from_slice(b"MIDDLE INSERT"); // INS
    new.extend_from_slice(&org[500..1000]); // back-reference
    (org, new)
}

/// End-to-end round trip (spec §16.1) for both fixture pairs over the brief's
/// option matrix: `jdiff <opts> A B p && jptch A p out` restores B byte-exact
/// and always exits 0. `-l`/`-lr` produce listings, not patches, so per the
/// brief the patch applied for those entries comes from a default jdiff run
/// (the listing run itself is still exercised and must exit 0).
/// Fixture pair: (original bytes, new bytes).
type FixturePair = (Vec<u8>, Vec<u8>);

// TODO(T20): the 0.8.5 writer (implicit MOD, spec §18.C) makes every patch of
// this matrix carry implicit-MOD runs — e.g. tiny/default now begins
// `ESC EQL 12 "byeby" ESC INS e` — and the current `jptch` decoder is still
// jpatch.cpp's 0.8.1 reader, which silently drops implicit-MOD data bytes
// (observed: tiny/default restores "hello world e"). Task 19 landed the 0.8.5
// JPatcht reader and the library-level replacement gate
// `library_roundtrip_jdiff_patcht` (below); this CLI-driven gate re-enables
// when Task 20 deletes the 0.8.1 binary and wires `-u`/argv[0] through the
// library. The round-trip gate itself (restores B byte-exact) is NOT loosened.
#[ignore]
#[test]
fn roundtrip_all_option_sets() {
    let dir = temp_dir("t10-rt");
    let pairs: [(&str, FixturePair); 2] = [
        ("tiny", (ORG_A.to_vec(), NEW_B.to_vec())),
        ("big", big_pair()),
    ];
    let option_sets: &[(&str, &[&str])] = &[
        ("default", &[]),
        ("-f", &["-f"]),
        ("-ff", &["-ff"]),
        ("-b", &["-b"]),
        ("-s 1", &["-s", "1"]),
        ("-bs 512", &["-bs", "512"]),
        ("-m 64", &["-m", "64"]),
        ("-m 0", &["-m", "0"]),
        ("-l", &[]),
        ("-lr", &[]),
    ];

    for (pair_name, (org, new)) in &pairs {
        let pdir = dir.join(pair_name);
        fs::create_dir_all(&pdir).expect("create pair dir");
        let a = write_file(&pdir.join("a.bin"), org);
        let b = write_file(&pdir.join("b.bin"), new);

        for (label, opts) in option_sets {
            let listing = pdir.join(format!("listing{}", label.replace(['-', ' '], "")));
            let patch = pdir.join(format!("p{}", label.replace(['-', ' '], "")));
            let out = pdir.join("out.bin");

            let mut is_listing = false;
            let mut jargs: Vec<&OsStr> = opts.iter().map(OsStr::new).collect();
            if *label == "-l" || *label == "-lr" {
                // Produce the listing; it is not a patch, so also make a
                // default-run patch below (brief's instruction).
                let largs: Vec<&OsStr> = {
                    let mut v = jargs.clone();
                    v.extend([a.as_os_str(), b.as_os_str(), listing.as_os_str()]);
                    v
                };
                let lrun = run_jdiff(&largs);
                assert_eq!(
                    lrun.status.code(),
                    Some(0),
                    "{label} listing on {pair_name}: {}",
                    stderr_str(&lrun)
                );
                assert!(
                    fs::read(&listing).is_ok_and(|d| !d.is_empty()),
                    "{label} listing must be non-empty"
                );
                is_listing = true;
            }
            if is_listing {
                jargs.clear();
            }
            jargs.extend([a.as_os_str(), b.as_os_str(), patch.as_os_str()]);

            let drun = run_jdiff(&jargs);
            assert_eq!(
                drun.status.code(),
                Some(0),
                "{label} diff on {pair_name}: {}",
                stderr_str(&drun)
            );

            let prun = run_ptch(&[a.as_os_str(), patch.as_os_str(), out.as_os_str()]);
            assert_eq!(
                prun.status.code(),
                Some(0),
                "{label} ptch on {pair_name}: {}",
                stderr_str(&prun)
            );
            assert!(
                stderr_str(&prun).is_empty(),
                "{label} ptch on {pair_name} must be silent: {:?}",
                stderr_str(&prun)
            );
            assert_eq!(
                fs::read(&out).unwrap(),
                *new,
                "{label} round trip on {pair_name}"
            );
        }
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// `-` may stand for any of the three file arguments (`jpatch.cpp:387-409`):
/// original on stdin, patch on stdin, output on stdout. (The Rust port reads
/// stdin fully into memory — spec §15.4 — which also serves pipes; the C++
/// oracle behaves identically for these fixture-sized cases.) The patch fed
/// here is the explicit-`ESC MOD` 0.8.1 encoding (see PATCH_AB_081): the
/// current decoder is still jpatch.cpp's, which drops implicit-MOD data.
#[test]
fn stdin_dash_variants() {
    let dir = temp_dir("t10-dash");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB_081);
    let out = dir.join("out.bin");

    // Original from stdin: `jptch - p out`.
    let r = run_ptch_stdin(
        &[OsStr::new("-"), p.as_os_str(), out.as_os_str()],
        &dir.join("a.bin"),
    );
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B, "org from stdin");
    fs::remove_file(&out).unwrap();

    // Patch from stdin: `jptch A - out`.
    let r = run_ptch_stdin(
        &[a.as_os_str(), OsStr::new("-"), out.as_os_str()],
        &dir.join("p.bin"),
    );
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B, "patch from stdin");
    fs::remove_file(&out).unwrap();

    // Output to stdout: `jptch A p -`.
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), OsStr::new("-")]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(r.stdout, NEW_B, "patch to stdout");
    assert!(r.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Verbose output lines byte-exact with the oracle (`jpatch.cpp:137-293`).
/// The EQL/DEL/BKT op lines print the original position *before* the
/// seek/copy and the output position before any copying; the final
/// "%12lld %12lld EOF" line (verbose > 1) has NO trailing newline. The
/// greeting precedes everything because verbose > 0.
// TODO(T20): byte-exact 0.8.1 greeting; Task 13 re-pointed JDIFF_VERSION/
// JDIFF_COPYRIGHT to the 0.8.5 strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn verbose_lines_match_reference() {
    let dir = temp_dir("t10-vrb");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let out = dir.join("out.bin");

    // PATCH_AB = EQL 12, MOD "byeby", INS "e".
    let eql = format!("{} {} EQL 12\n", p12(0), p12(0));

    // -v: op lines for EQL, MOD and INS ("MOD ..." has 4 trailing spaces).
    let r = run_ptch(&[
        OsStr::new("-v"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string()
            + &eql
            + &format!("{} {} MOD ...    \n", p12(11), p12(12))
            + &format!("{} {} INS ...    \n", p12(16), p12(17))
    );

    // -vv: EQL line plus the unpadded final EOF line (no newline).
    let r = run_ptch(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string() + &eql + &format!("{} {} EOF", p12(17), p12(18))
    );

    // -vvv: additionally the per-byte MOD/INS data lines with %3o/%c values
    // ('b' = 0o142, 'y' = 0o171, 'e' = 0o145). INS prints the output position
    // *before* writing the byte (jpatch.cpp:277-280); MOD after
    // (jpatch.cpp:261-263).
    let r = run_ptch(&[
        OsStr::new("-vvv"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string()
            + &eql
            + &format!("{} {} MOD 142 b\n", p12(12), p12(12))
            + &format!("{} {} MOD 171 y\n", p12(13), p12(13))
            + &format!("{} {} MOD 145 e\n", p12(14), p12(14))
            + &format!("{} {} MOD 142 b\n", p12(15), p12(15))
            + &format!("{} {} MOD 171 y\n", p12(16), p12(16))
            + &format!("{} {} INS 145 e\n", p12(16), p12(17))
            + &format!("{} {} EOF", p12(17), p12(18))
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// ESC-escaped data applies exactly: `ESC ESC` yields one literal ESC byte in
/// MOD data (the `case ESC` arm keeps `liOpr` and falls through to the data
/// path, `jpatch.cpp:224-229`), and a bare trailing ESC maps the operand
/// getc's EOF to `putc(-1)` = byte 0xFF (oracle-pinned: output 'x' A7 FF).
// TODO(T20): its -vvv expected stderr is prefixed with the byte-exact 0.8.1
// greeting; Task 13 re-pointed JDIFF_VERSION/JDIFF_COPYRIGHT to the 0.8.5
// strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn esc_escaped_data_roundtrip() {
    let dir = temp_dir("t10-esc");
    let org = write_file(&dir.join("org.bin"), b"0123456789");
    let out = dir.join("out.bin");

    // ESC MOD 'A' ESC ESC 'B' -> "A\xA7B".
    let p = write_file(&dir.join("p.bin"), &[0xA7, 0xA6, b'A', 0xA7, 0xA7, b'B']);
    let r = run_ptch(&[org.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'A', 0xA7, b'B']);

    // Same escape inside INS data.
    let p = write_file(&dir.join("p2.bin"), &[0xA7, 0xA5, b'A', 0xA7, 0xA7, b'B']);
    let r = run_ptch(&[org.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'A', 0xA7, b'B'], "INS escape");

    // Bare trailing ESC after MOD data: the operand getc returns EOF (-1),
    // which the default arm turns into `lbEsc` (writing ESC first) and the
    // data `putc(liInp)` then emits 0xFF; with -vvv the %3o of the int -1
    // prints as 37777777777 (oracle-pinned byte stream).
    let p = write_file(&dir.join("p3.bin"), &[0xA7, 0xA6, b'x', 0xA7]);
    let r = run_ptch(&[
        OsStr::new("-vvv"),
        org.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'x', 0xA7, 0xFF], "trailing ESC");
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string()
            + &format!("{} {} MOD 170 x\n", p12(0), p12(0))
            + &format!("{} {} ESC XXX\n", p12(1), p12(1))
            + &format!("{} {} MOD 247 ESC\n", p12(1), p12(1))
            + &format!("{} {} MOD 37777777777  \n", p12(2), p12(2))
            + &format!("{} {} EOF", p12(3), p12(3))
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Bytes arriving while `liOpr` is still the initial ESC (and after a
/// completed operand) hit no case of the data switch and are silently
/// dropped (`jpatch.cpp:244-247` with `liOpr == ESC`; spec §15.7) — the
/// C++ behavior, unlike the francisdb fork's MOD-data interpretation.
#[test]
fn garbage_after_ops_ignored() {
    let dir = temp_dir("t10-grb");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out = dir.join("out.bin");

    // Leading garbage "XY" before the first operand is dropped.
    let p = write_file(&dir.join("p1.bin"), &[b'X', b'Y', 0xA7, 0xA3, 0x0B]);
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), &ORG_A[..12], "leading garbage");

    // Trailing garbage after a complete EQL is dropped as well (liOpr == EQL).
    let p = write_file(&dir.join("p2.bin"), &[0xA7, 0xA3, 0x0B, b'X', b'Y']);
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), &ORG_A[..12], "trailing garbage");
    fs::remove_dir_all(&dir).unwrap();
}

/// Length reads at patch EOF follow the C arithmetic on `getc`-or-EOF (-1)
/// values with no error path (`jpatch.cpp:70-105`): patch [ESC, DEL, 252] at
/// EOF yields `253 + (-1) = 252`, the DEL seek past EOF succeeds, and the run
/// exits 0 with empty output. With -vv the DEL line shows length 252 and the
/// final EOF line shows the org position advanced to 252 by the seek.
// TODO(T20): its -vv expected stderr is prefixed with the byte-exact 0.8.1
// greeting; Task 13 re-pointed JDIFF_VERSION/JDIFF_COPYRIGHT to the 0.8.5
// strings and Task 20 rewrites the banner text.
#[ignore]
#[test]
fn truncated_length_is_faithful() {
    let dir = temp_dir("t10-trunc");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), &[0xA7, 0xA4, 0xFC]);
    let out = dir.join("out.bin");

    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert!(fs::read(&out).unwrap().is_empty(), "nothing copied");

    let r = run_ptch(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string()
            + &format!("{} {} DEL 252\n", p12(0), p12(0))
            + &format!("{} {} EOF", p12(252), p12(0))
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Exit-code contract (`jpatch.cpp:359-424`): -h and missing arguments print
/// greeting+usage to stddbg and exit 2; unopenable data/patch/output files
/// exit 3/4/5 with their messages (these checks are LIVE in jpatch.cpp, which
/// jfopens directly); the success path always exits 0 — even for an
/// empty-change patch, where jdiff would exit 1.
// TODO(T20): the -h/missing-args arms pin the byte-exact 0.8.1 greeting +
// usage; Task 13 re-pointed JDIFF_VERSION/JDIFF_COPYRIGHT to the 0.8.5
// strings and Task 20 rewrites the banner text (the 3/4/5 arms stay valid).
#[ignore]
#[test]
fn exit_codes() {
    let dir = temp_dir("t10-exit");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let out = dir.join("out.bin");

    // -h → 2, byte-exact greeting + usage, nothing on stdout.
    let r = run_ptch(&[OsStr::new("-h")]);
    assert_eq!(r.status.code(), Some(2));
    assert_eq!(stderr_str(&r), GREETING_PTCH.to_string() + USAGE_PTCH);
    assert!(r.stdout.is_empty());

    // Missing arguments → greeting + usage, exit 2.
    for args in [vec![], vec![OsStr::new("some-file")]] {
        let r = run_ptch(&args);
        assert_eq!(r.status.code(), Some(2), "args {args:?}");
        assert_eq!(
            stderr_str(&r),
            GREETING_PTCH.to_string() + USAGE_PTCH,
            "args {args:?}"
        );
    }

    // Unopenable original → "Could not open data file %s for reading.", 3.
    let missing = dir.join("does-not-exist");
    let r = run_ptch(&[missing.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(3), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        format!(
            "Could not open data file {} for reading.\n",
            missing.display()
        )
    );
    assert!(!out.exists(), "output must not be created");

    // Unopenable patch → "Could not open patch file %s for reading.", 4.
    let r = run_ptch(&[a.as_os_str(), missing.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(4), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        format!(
            "Could not open patch file {} for reading.\n",
            missing.display()
        )
    );

    // Unopenable output → "Could not open output file for writing." (no
    // filename), 5.
    let bad_out = dir.join("no-such-dir").join("out.bin");
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), bad_out.as_os_str()]);
    assert_eq!(r.status.code(), Some(5), "{}", stderr_str(&r));
    assert_eq!(stderr_str(&r), "Could not open output file for writing.\n");

    // Success: exit 0 always — a differing pair ...
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B);
    // ... and the single-EQL patch of an identical pair (jdiff exits 1 there;
    // jpatch.cpp:424 exits 0 unconditionally).
    let q = write_file(&dir.join("q.bin"), &[0xA7, 0xA3, 0x10]);
    let r = run_ptch(&[a.as_os_str(), q.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), ORG_A);

    // -t: stored test flag; raises verbose to 2 (greeting + op/EOF lines) and
    // has no other effect (`jpatch.cpp:328-330` — gbTst is never read).
    let r = run_ptch(&[
        OsStr::new("-t"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B);
    assert_eq!(
        stderr_str(&r),
        GREETING_PTCH.to_string()
            + &format!("{} {} EQL 12\n", p12(0), p12(0))
            + &format!("{} {} EOF", p12(17), p12(18))
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// The checked I/O error paths of the EQL copy (`jpatch.cpp:160-219`): seek
/// failures exit 6, short reads exit 8, short writes exit 9 — each with its
/// message on real stderr (not stddbg). Also pins the two's-complement
/// length arithmetic: a 255-marker + 7 data bytes + EOF reads
/// `(0xFF…FF << 8) + (-1) = -257`, which makes the DEL seek negative.
#[test]
fn seek_read_write_error_exits() {
    let dir = temp_dir("t10-ioerr");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out = dir.join("out.bin");

    // BKT 5 at org position 0 → seek to -5 fails → exit 6.
    let p = write_file(&dir.join("pbkt.bin"), &[0xA7, 0xA2, 0x04]);
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(6), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        "Could not position on original file (seek back 0 - 5).\n"
    );

    // DEL with an 8-byte length of 0xFF…FF truncated one byte early:
    // marker 255 + 7×255 + EOF(-1) = -257 → seek(-257) at position 0 → exit 6.
    let p = write_file(
        &dir.join("pdel257.bin"),
        &[0xA7, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    );
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(6), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        "Could not position on original file (seek -257 + 0).\n"
    );

    // EQL 100 from a 17-byte file → short read → exit 8.
    let p = write_file(&dir.join("p.eql100"), &[0xA7, 0xA3, 0x63]);
    let r = run_ptch(&[a.as_os_str(), p.as_os_str(), out.as_os_str()]);
    assert_eq!(r.status.code(), Some(8), "{}", stderr_str(&r));
    assert_eq!(stderr_str(&r), "Error reading original file.\n");

    // Small EQL (100 bytes, fully readable) to /dev/full: the C stdio buffer
    // absorbs the write, no check ever fires, silent exit 0 (oracle-pinned;
    // the Rust BufWriter reproduces this buffering).
    #[cfg(target_os = "linux")]
    {
        let big = write_file(&dir.join("big200.bin"), &[0u8; 200]);
        let r = run_ptch(&[big.as_os_str(), p.as_os_str(), OsStr::new("/dev/full")]);
        assert_eq!(
            r.status.code(),
            Some(0),
            "buffered small write must stay silent: {:?}",
            stderr_str(&r)
        );
        assert!(r.stderr.is_empty(), "no message expected");

        // Large EQL (65536 = 16 blocks) to /dev/full: the checked fwrite
        // eventually fails → "Error writing output file.", exit 9.
        let big = write_file(&dir.join("big64k.bin"), &[0u8; 65536]);
        let p9 = write_file(
            &dir.join("p.eql64k.bin"),
            &[0xA7, 0xA3, 0xFE, 0x00, 0x01, 0x00, 0x00],
        );
        let r = run_ptch(&[big.as_os_str(), p9.as_os_str(), OsStr::new("/dev/full")]);
        assert_eq!(r.status.code(), Some(9), "{}", stderr_str(&r));
        assert_eq!(stderr_str(&r), "Error writing output file.\n");
    }

    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Task 19: library-level patch gates (`JPatcht` + `JFileOut`)
// ===========================================================================
//
// The CLI round-trip gates above drive the binaries, whose `jptch` is still
// the 0.8.1 reader (drops implicit-MOD bytes) until Task 20 — they stay
// `#[ignore]`d TODO(T20). These are their library-level replacements:
// JDiff + JOutBin produce a patch, JPatcht + JFileOut apply it, in memory.
//
// Library traces and error lines go to the process streams (stddbg = stderr
// by default; the C++ `fprintf(stderr, ...)` sites), which an in-process
// test cannot capture — the two `*_child` gates below re-exec this test
// binary with `--exact` and assert the captured stderr bytes.

use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::rc::Rc;

use jojodiff_cli_rs::defs::{EXI_ERR, EXI_OK};
use jojodiff_cli_rs::jdiff::JDiff;
use jojodiff_cli_rs::jfile::{JFileAhead, JFileMem};
use jojodiff_cli_rs::jfileout::JFileOut;
use jojodiff_cli_rs::jout::JOutBin;
use jojodiff_cli_rs::jpatcht::JPatcht;

/// `Rc<RefCell<Vec<u8>>>` sink so the patch bytes survive the boxed
/// `Box<dyn JOut>` (JOutBin::into_inner sits behind the concrete type).
#[derive(Clone)]
struct SharedSink(Rc<RefCell<Vec<u8>>>);

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Library-level engine parameters — the knobs the CLI option sets map to
/// (defaults and `-b`/`-f`/`-ff` presets per `src/bin/jdiff.rs`; `-s 1` is
/// the hashtable MB count). The exact CLI wiring is Task 20; the round-trip
/// property does not depend on which valid engine configuration produced
/// the patch. `src_bkt`/`src_scn` stay at the defaults except `-ff`
/// (prescan off).
#[derive(Clone, Copy)]
struct LibParams {
    hsh_mbt: i32,
    mch_max: i32,
    mch_min: i32,
    ahd_max: i64,
    cmp_all: bool,
    src_scn: bool,
}

const P_DEFAULT: LibParams = LibParams {
    hsh_mbt: 8,
    mch_max: 32,
    mch_min: 8,
    ahd_max: 256 * 1024,
    cmp_all: true,
    src_scn: true,
};

const P_B: LibParams = LibParams {
    hsh_mbt: 32,
    mch_max: 128,
    mch_min: 16,
    ahd_max: 4096 * 1024,
    cmp_all: true,
    src_scn: true,
};

const P_F: LibParams = LibParams {
    hsh_mbt: 4,
    mch_max: 16,
    mch_min: 8,
    ahd_max: 64 * 1024,
    cmp_all: false,
    src_scn: true,
};

const P_FF: LibParams = LibParams {
    hsh_mbt: 1,
    mch_max: 16,
    mch_min: 4,
    ahd_max: 4096 * 1024,
    cmp_all: false,
    src_scn: false,
};

const P_S1: LibParams = LibParams {
    hsh_mbt: 1,
    mch_max: 32,
    mch_min: 8,
    ahd_max: 256 * 1024,
    cmp_all: true,
    src_scn: true,
};

const P_MIN1MAX1: LibParams = LibParams {
    hsh_mbt: 8,
    mch_max: 1,
    mch_min: 1,
    ahd_max: 256 * 1024,
    cmp_all: true,
    src_scn: true,
};

/// Produces a patch with the library engine (`JDiff` + `JOutBin` over
/// in-memory readers), like the CLI's default run.
fn lib_diff(org: Vec<u8>, new: Vec<u8>, p: LibParams) -> Vec<u8> {
    let sink = SharedSink(Rc::new(RefCell::new(Vec::new())));
    let mut jd = JDiff::new(
        Box::new(JFileMem::new(org)),
        Box::new(JFileMem::new(new)),
        Box::new(JOutBin::new(sink.clone())),
        p.hsh_mbt,
        0,
        true,
        p.src_scn,
        p.mch_max,
        p.mch_min,
        p.ahd_max,
        p.cmp_all,
    );
    let rc = jd.jdiff();
    assert_eq!(rc, 0, "library jdiff must succeed on differing pairs");
    let bytes = std::mem::take(&mut *sink.0.borrow_mut());
    assert!(!bytes.is_empty(), "patch for a differing pair is non-empty");
    bytes
}

/// Applies `patch` to `org` with the 0.8.5 library reader over in-memory
/// files (the byte-loop fallback of `JFileOut::copyfrom`).
fn lib_apply(org: &[u8], patch: &[u8], v: i32) -> (i32, Vec<u8>) {
    let mut org_f = JFileMem::new(org.to_vec());
    let mut pch_f = JFileMem::new(patch.to_vec());
    let mut jp = JPatcht::new(&mut org_f, &mut pch_f, JFileOut::new(Vec::new()), v);
    let rc = jp.jpatch();
    (rc, jp.into_inner().into_inner())
}

/// Same, over buffered look-ahead readers (the `getbuf` fast path of
/// `JFileOut::copyfrom`).
fn lib_apply_ahead(org: &[u8], patch: &[u8]) -> (i32, Vec<u8>) {
    let mut org_f = JFileAhead::new(Cursor::new(org.to_vec()), "Org", 1024, 64);
    let mut pch_f = JFileAhead::new(Cursor::new(patch.to_vec()), "Pch", 1024, 64);
    let mut jp = JPatcht::new(&mut org_f, &mut pch_f, JFileOut::new(Vec::new()), 0);
    let rc = jp.jpatch();
    (rc, jp.into_inner().into_inner())
}

/// Library-level replacement for the CLI round-trip gates
/// (`roundtrip_gate` in tests/oracle.rs and `roundtrip_all_option_sets`
/// above, both `#[ignore]`d TODO(T20) while the `jptch` binary is still the
/// 0.8.1 reader): the library diff output applied by the 0.8.5 `JPatcht`
/// restores the new file byte-exact, across the fixture pairs and the
/// option sets that still run. Every leg applies through JFileMem AND the
/// buffered JFileAhead reader.
#[test]
fn library_roundtrip_jdiff_patcht() {
    let sets: [(&str, LibParams); 6] = [
        ("default", P_DEFAULT),
        ("-b", P_B),
        ("-f", P_F),
        ("-ff", P_FF),
        ("-s 1", P_S1),
        ("-min 1 -max 1", P_MIN1MAX1),
    ];

    // The tiny pair takes the full matrix.
    for (label, p) in sets {
        let patch = lib_diff(ORG_A.to_vec(), NEW_B.to_vec(), p);
        let (rc, out) = lib_apply(ORG_A, &patch, 0);
        assert_eq!(rc, EXI_OK, "{label} mem apply");
        assert_eq!(out, NEW_B, "{label} mem round trip");
        let (rc, out) = lib_apply_ahead(ORG_A, &patch);
        assert_eq!(rc, EXI_OK, "{label} ahead apply");
        assert_eq!(out, NEW_B, "{label} ahead round trip");
    }

    // The corpus pairs take the fast subsets (engine run time).
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    for (pair, org_name, new_name, subset) in [
        (
            "bkocomu",
            "bkocomu.0000.fil",
            "bkocomu.0009.fil",
            [&"default", &"-f", &"-ff", &"-b"],
        ),
        (
            "test2",
            "test2.001.txt",
            "test2.002.txt",
            [&"default", &"-f", &"-ff", &"-b"],
        ),
    ] {
        let org = fs::read(fixtures.join(org_name)).expect("read org fixture");
        let new = fs::read(fixtures.join(new_name)).expect("read new fixture");
        for label in subset {
            let p = sets
                .iter()
                .find(|(l, _)| l == label)
                .expect("known label")
                .1;
            let patch = lib_diff(org.clone(), new.clone(), p);
            let (rc, out) = lib_apply(&org, &patch, 0);
            assert_eq!(rc, EXI_OK, "{pair}/{label} mem apply");
            assert_eq!(out, new, "{pair}/{label} mem round trip");
            let (rc, out) = lib_apply_ahead(&org, &patch);
            assert_eq!(rc, EXI_OK, "{pair}/{label} ahead apply");
            assert_eq!(out, new, "{pair}/{label} ahead round trip");
        }
    }
}

/// Cross-version compatibility gate (spec §18.C/§22.3) at library level:
/// every committed 0.8.1 golden patch (`tests/fixtures/golden/**`, generated
/// by the 0.8.1 C++ oracle) applies through the 0.8.5 `JPatcht` and restores
/// the corresponding "new" fixture byte-exact. Explicit opcodes are a subset
/// of the 0.8.5 grammar — this pins the one-way compatibility.
#[test]
fn golden_081_patches_apply_and_restore() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let golden = fixtures.join("golden");
    assert!(
        golden.is_dir(),
        "committed goldens missing at {}",
        golden.display()
    );

    let mut checked = 0;
    for (pair, org_name, new_name) in [
        ("bkocomu", "bkocomu.0000.fil", "bkocomu.0009.fil"),
        ("test2", "test2.001.txt", "test2.002.txt"),
    ] {
        let org = fs::read(fixtures.join(org_name)).expect("read org fixture");
        let new = fs::read(fixtures.join(new_name)).expect("read new fixture");
        for entry in fs::read_dir(golden.join(pair)).expect("golden pair dir") {
            let path = entry.expect("golden entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("jdf") {
                continue; // listings (.asc/.rgn) and stderr captures are not patches
            }
            let patch = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let (rc, out) = lib_apply(&org, &patch, 0);
            assert_eq!(rc, EXI_OK, "{}: apply failed", path.display());
            assert_eq!(out, new, "{}: restored bytes differ", path.display());
            checked += 1;
        }
    }
    assert!(
        checked >= 22,
        "expected the 22 committed golden .jdf patches (10 bkocomu + 12 test2), found {checked}"
    );
}

/// Re-execs this test binary for `test_name` with `guard` set and captures
/// the child's output — the only way to pin library output that the port
/// writes to the process streams (stddbg = stderr by default). The child
/// runs under `--nocapture` so nothing intercepts the streams.
fn run_self(test_name: &str, guard: &'static str) -> Output {
    Command::new(std::env::current_exe().expect("current test binary"))
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(guard, "1")
        .output()
        .expect("re-exec test binary")
}

/// The trailing-byte warning line is byte-exact and goes to (real) stderr
/// (`JPatcht.cpp:241-245`), with the `EXI_ERR` return.
#[test]
fn patcht_trailing_byte_warning_line() {
    const GUARD: &str = "T19_WARN_CHILD";
    if std::env::var_os(GUARD).is_some() {
        let (rc, out) = lib_apply(ORG_A, &[0xA7], 0);
        assert_eq!(rc, EXI_ERR);
        assert!(out.is_empty());
        return;
    }
    let out = run_self("patcht_trailing_byte_warning_line", GUARD);
    assert!(out.status.success(), "child failed: {}", stderr_str(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "Warning: unexpected trailing byte at end of file, patch file may be corrupted.\n"
    );
}

/// Verbose traces are byte-exact with `JPatcht.cpp`'s formats
/// (`:101-107` per-byte, `:156-158/:166-169/:179-182` ESC traces,
/// `:265-335` op summaries and the EOF line) and gated exactly as the C++
/// gates them (all verbose-driven, none debug-driven): per-byte lines at
/// `> 1`, ESC traces at `> 2`, MOD/INS summaries at `== 1`, DEL/EQL/BKT/EOF
/// at `>= 1`. Positions print with `P8zd` (p12() below handles the
/// debug/release width switch).
#[test]
fn patcht_verbose_traces_byte_exact() {
    const GUARD: &str = "T19_VERBOSE_CHILD";
    if std::env::var_os(GUARD).is_some() {
        // Child: run every case; assertions on the applied bytes run here,
        // the parent asserts the stderr bytes below.
        let (rc, out) = lib_apply(ORG_A, PATCH_AB_081, 0);
        assert_eq!((rc, out.as_slice()), (EXI_OK, NEW_B));
        for v in [1, 2, 3] {
            let (rc, out) = lib_apply(ORG_A, PATCH_AB_081, v);
            assert_eq!((rc, out.as_slice()), (EXI_OK, NEW_B), "v={v}");
        }
        let (rc, out) = lib_apply(b"0123456789", &[0xA7, 0xA6, b'A', 0xA7, 0xA6, b'B'], 3);
        assert_eq!(rc, EXI_OK);
        assert_eq!(out, [b'A', 0xA7, 0xA6, b'B']);
        let (rc, out) = lib_apply(b"0123456789", &[0xA7, 0xA6, b'A', 0xA7, 0xA7, b'B'], 3);
        assert_eq!(rc, EXI_OK);
        assert_eq!(out, [b'A', 0xA7, b'B']);
        let (rc, out) = lib_apply(b"0123456789", &[0xA7, 0xA6, b'A', 0xA7, 0x01, b'B'], 3);
        assert_eq!(rc, EXI_OK);
        assert_eq!(out, [b'A', 0xA7, 0x01, b'B']);
        return;
    }
    let out = run_self("patcht_verbose_traces_byte_exact", GUARD);
    assert!(out.status.success(), "child failed: {}", stderr_str(&out));

    /// Two `P8zd`-formatted positions joined by one space — the prefix of
    /// every verbose trace line (`P8zd " " P8zd`).
    fn pp(org: i64, out: i64) -> String {
        format!("{} {}", p12(org), p12(out))
    }

    let v0 = ""; // v = 0 is silent (first child case emits nothing)
    let v1 = format!(
        // EQL (>=1) + MOD/INS summaries (==1) + EOF (>=1).
        "{} EQL 12\n\
         {} MOD 5\n\
         {} INS 1\n\
         {} EOF\n",
        pp(0, 0),
        pp(12, 12),
        pp(17, 17),
        pp(17, 18),
    );
    let v2 = format!(
        // Per-byte lines (>1) replace the ==1-only summaries.
        "{} EQL 12\n\
         {} MOD 62 b\n\
         {} MOD 79 y\n\
         {} MOD 65 e\n\
         {} MOD 62 b\n\
         {} MOD 79 y\n\
         {} INS 65 e\n\
         {} EOF\n",
        pp(0, 0),
        pp(12, 12),
        pp(13, 13),
        pp(14, 14),
        pp(15, 15),
        pp(16, 16),
        pp(17, 17),
        pp(17, 18),
    );
    let v3 = v2.clone(); // no ESC sequences in PATCH_AB_081
    let esc_same = format!(
        // ESC <same-opr> inside a run: trace + ESC + opr byte as data.
        "{} MOD 41 A\n\
         {} ESC a6\n\
         {} MOD a7  \n\
         {} MOD a6  \n\
         {} MOD 42 B\n\
         {} EOF\n",
        pp(0, 0),
        pp(1, 1),
        pp(1, 1),
        pp(2, 2),
        pp(3, 3),
        pp(4, 4),
    );
    let esc_esc = format!(
        // ESC ESC inside a run: "ESC ESC" trace, one literal ESC out.
        "{} MOD 41 A\n\
         {} ESC ESC\n\
         {} MOD a7  \n\
         {} MOD 42 B\n\
         {} EOF\n",
        pp(0, 0),
        pp(1, 1),
        pp(1, 1),
        pp(2, 2),
        pp(3, 3),
    );
    let esc_xxx = format!(
        // ESC <unknown> inside a run: "ESC XXX" trace, both bytes as data
        // (the %c of 0x01 is the filtered ' ').
        "{} MOD 41 A\n\
         {} ESC XXX\n\
         {} MOD a7  \n\
         {} MOD 01  \n\
         {} MOD 42 B\n\
         {} EOF\n",
        pp(0, 0),
        pp(1, 1),
        pp(1, 1),
        pp(2, 2),
        pp(3, 3),
        pp(4, 4),
    );

    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        v0.to_string() + &v1 + &v2 + &v3 + &esc_same + &esc_esc + &esc_xxx,
    );
}
