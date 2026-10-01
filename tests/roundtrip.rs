//! Integration tests for the `jdiff` CLI (port plan Task 9): option parsing,
//! greeting/help, file handling, stats output and exit codes.
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

/// The patch jdiff produces for (ORG_A, NEW_B), pinned byte-exact against the
/// C++ oracle: EQL 12, MOD data "byeby", INS "e".
const PATCH_AB: &[u8] = &[
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
#[test]
fn help_exit_code_and_text() {
    let out = run(&[OsStr::new("-h")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr_str(&out), GREETING.to_string() + &usage(8, 32));
    assert!(out.stdout.is_empty());
}

/// 0 or 1 file args: greeting + usage on stddbg, exit 2 (`main.cpp:346-387`).
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
#[test]
fn stats_lines_match_reference() {
    let dir = temp_dir("stats");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let expected = GREETING.to_string()
        + "Lookahead buffers: 512 kb. (256 kb. per file).\n"
        + "Hastable size    : 98304 kb. (8388593 samples).\n"
        + "Prescanning:\n"
        + ".\n"
        + "Hashtable size          = 100663116 samples, 98304 KB, 96 MB\n"
        + "Hashtable prime         = 8388593\n"
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

    // -s 2048 → while >1024 divide → hashtable 2097143 samples / 24576 kb.
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
        stderr_str(&out).contains("Hastable size    : 24576 kb. (2097143 samples).\n"),
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
