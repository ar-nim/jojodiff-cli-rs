//! Integration tests for the single `jdiff` CLI — the 0.8.5 `main.cpp` port
//! (spec §18.A/§18.D/§18.F; port plan Task 20): getopt_long option surface,
//! `-u` patch mode, argv[0] dispatch, stdin/sequential handling, greeting/
//! usage/echo/statistics text and the swapped exit codes.
//!
//! Every expected string and byte below was pinned against the vendored 0.8.5
//! C++ oracle built by `scripts/build-oracle.sh` (`target/oracle/jdiff`):
//! greetings, usage (including the stale "-i 64"/"-k 8192"/"(in KB)"/
//! "0=no buffering"/"disbale" texts, spec §21.10), the getopt error lines
//! (glibc format), progress/statistics blocks and patch bytes. One value is a
//! port-vs-oracle deviation with a spec-pinned port value (see the -vv test):
//! the 0-initialized `Inaccurate solutions` baseline (§21.6: C++ stack
//! garbage). The index tables are element-identical on both sides (§21.18:
//! 12 bytes/element everywhere).
//!
//! The tests spawn the real binary (`env!("CARGO_BIN_EXE_jdiff")`) and capture
//! stdout/stderr via `Command::output()`. argv[0] dispatch is exercised by
//! copying the binary to `jpatch`/`jptch`/`jdedup` names in a temp dir
//! (spec §21.2 — there is exactly one binary). The port-only `--compat-081`
//! (spec §21.16, Task 23) has its own section after the option-matrix gate:
//! writer vectors live inline in `src/jout/bin.rs`, the 0.8.1-oracle gate in
//! `tests/oracle.rs`.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use jojodiff_cli_rs::defs::Op;

/// Byte-exact greeting block (`main.cpp:480-509`): version line, copyright,
/// blank line, GPL block (0.8.5 wording, final line
/// "along with this program. If not, see www.gnu.org/licenses/gpl-3.0"), then
/// the "File adressing" (sic) line. Oracle-verified byte-for-byte.
const GREETING: &str = concat!(
    "\n",
    "JDIFF - binary diff version 0.8.5 (beta) 2020\n",
    "Copyright (C) 2002-2020 Joris Heirbaut\n",
    "\n",
    "JojoDiff is free software: you can redistribute it and/or modify it\n",
    "under the terms of the  GNU General Public License  as published by\n",
    "the Free Software Foundation,  either version 3 of the License,  or\n",
    "(at your option) any later version.\n",
    "\n",
    "This program is distributed in the hope that it will be useful,\n",
    "but WITHOUT ANY WARRANTY; without even the implied warranty of\n",
    "MERCHANTABILITY  or  FITNESS FOR A PARTICULAR PURPOSE. See the\n",
    "GNU General Public License for more details.\n",
    "\n",
    "You should have received a copy of the GNU General Public License\n",
    "along with this program. If not, see www.gnu.org/licenses/gpl-3.0\n",
    "\n",
    "File adressing is 64 bit for files up to 8388607TB, samples are 32 bytes.\n",
);

/// Usage/help block (`main.cpp:511-557`); the `-n`/`-x` lines print the
/// current (post-parse) match limits, so they are formatted in. The stale
/// texts are replicated verbatim (spec §21.10): "-i" says "(default 64)"
/// (actual 32), "-k" "(default 8192)" (actual 32768), "-m" "(in KB)"
/// (actual MB) and "0=no buffering" (no such mode).
fn usage(mch_min: i32, mch_max: i32) -> String {
    format!(
        concat!(
            "\n",
            "JDiff differentiates two files so that the second file can be recreated from\n",
            "the first by \"undiffing\". JDiff aims for the smallest possible diff file.\n\n",
            "Usage: jdiff -j [options] <source file> <destination file> [<diff file>]\n",
            "   or: jdiff -u [options] <source file> <diff file> [<destination file>]\n\n",
            "  -j                       JDiff:  create a difference file.\n",
            "  -u                       Undiff: undiff a difference file.\n\n",
            "  -v --verbose             Verbose: greeting, results and tips.\n",
            "  -vv                      Extra Verbose: progress info and statistics.\n",
            "  -vvv                     Ultra Verbose: all info, including help and details.\n",
            "  -h --help -hh            Help, additional help (-hh) and exit.\n",
            "  -l --listing             Detailed human readable output.\n",
            "  -r --regions             Grouped  human readable output.\n",
            "  -c --console             Write verbose and debug info to stdout.\n\n",
            "  -b --better -bb...       Better: use more memory, search more.\n",
            "  -bb                      Best:   even more memory, search more.\n",
            "  -f --lazy                Lazy:   no unbuffered searching (often slower).\n",
            "  -ff                      Lazier: no full index table.\n",
            "  -p --sequential-source   Sequential source (to avoid !) (with - for stdin).\n",
            "  -q --sequential-dest     Sequential destination (with - for stdin).\n",
            "  -s --stdio               Use stdio files (for testing).\n\n",
            "  -a --search-size <size>  Size (in KB) to search (default=buffer-size).\n",
            "  -i --index-size  <size>  Size (in MB) for index table    (default 64).\n",
            "  -k --block-size  <size>  Block size in bytes for reading (default 8192).\n",
            "  -m --buffer-size <size>  Size (in KB) for search buffers (0=no buffering)\n",
            "  -n --search-min <count>  Minimum number of matches to search (default {}).\n",
            "  -x --search-max <count>  Maximum number of matches to search (default {}).\n\n",
            "Make  diff-file: jdiff -j old-file new-file diff-file.jdf\n",
            "Apply diff-file: jdiff -u old-file diff-file.jdf recreated-new-file\n\n",
            "Hint:\n",
            "  Do not use jdiff on compressed files. Rather use jdiff first and compress\n",
            "  afterwards, e.g.: jdiff -j old new | gzip >dif.jdf.gz (or 7z with -si)\n",
        ),
        mch_min, mch_max
    )
}

/// The `-hh` notes block (`main.cpp:559-593`, printed when `liHlp > 1` or
/// verbose > 2), verbatim including the two-space "blank" lines.
const NOTES: &str = concat!(
    "\nNotes:\n",
    " - Options -b, -bb, -f, -ff, ... should be used before other options.\n",
    " - Accuracy may be improved by increasing the index table size (-i) or\n",
    "   the buffer size (-m), see below.\n",
    " - The index table size is always lowered to the nearest lower prime number.\n",
    " - Output is sent to standard output if no output file is specified.\n",
    "\nAdditional explications:\n",
    "  JDiff starts by comparing source and destination files.\n",
    "  \n",
    "  When a difference is found, JDiff will first index the source file.\n",
    "  Normally, the full source file is indexed, but this can be disabled by the\n",
    "  -ff or -p options, in which case only the buffered part of the source file\n",
    "  will be indexed. This may be faster, but at a loss of accuracy.\n",
    "  \n",
    "  Using the index, JDiff will search for equal regions between both files.\n",
    "  The index table however has two problems:\n",
    "  - too small, because a full index would require too much memory.\n",
    "  - inaccurate, because the hash-keys are only 32 or 64 bit check-sums.\n",
    "  \n",
    "  The inaccuracy is reduced by either:\n",
    "  - comparing the found matches from the index, which is slower but certain\n",
    "  - confirmation from subsequent matches, which is faster but uncertain\n",
    "  Inaccuracy of course can also be reduced with a bigger index table (-i option)\n",
    "  \n",
    "  Also, the first found solution is not always the best solution.\n",
    "  Therefore, JDiff searches a minimum (-n) number of solutions, and\n",
    "  will continue up to a maximum (-x) number of solutions if data is buffered.\n",
    "  That's why, bigger buffers (-m) can improve accuracy.\n",
    "  \n",
    "  The -b/-bb options increase the index table, buffers and solutions to search.\n",
    "  The -f/-ff options will only compare buffered data to gain some speed, but\n",
    "  will often be slower due to the lower accuracy.\n",
);

/// 18-byte fixture differing only in the tail: A = "hello world hello",
/// B = "hello world byebye".
const ORG_A: &[u8] = b"hello world hello";
const NEW_B: &[u8] = b"hello world byebye";

/// The patch jdiff produces for (ORG_A, NEW_B), byte-exact with the 0.8.5
/// oracle (implicit MOD, spec §18.C): EQL 12, MOD data "byeby" riding the
/// implicit MOD, INS "e".
const PATCH_AB: &[u8] = &[
    0xA7, 0xA3, 0x0B, // ESC EQL len=12
    b'b', b'y', b'e', b'b', b'y', // implicit MOD "byeby" (no ESC MOD pair)
    0xA7, 0xA5, b'e', // ESC INS "e"
];

/// The same patch in the 0.8.1 wire format (explicit `ESC MOD` record) — the
/// 0.8.5 reader accepts explicit opcodes as a subset of its grammar (spec
/// §18.C one-way compatibility).
const PATCH_AB_081: &[u8] = &[
    0xA7, 0xA3, 0x0B, // ESC EQL len=12
    0xA7, 0xA6, b'b', b'y', b'e', b'b', b'y', // ESC MOD "byeby"
    0xA7, 0xA5, b'e', // ESC INS "e"
];

/// Unique temp directory per test (parallel-safe), cleaned up by the caller.
fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jdiff-t20-{}-{}-{}",
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

/// Runs `jdiff` with the given args, `file` wired to stdin as a real file
/// (fd 0 seekable — the C++ `cin` behaves the same on redirected files).
fn run_stdin_file(args: &[&OsStr], file: &Path) -> Output {
    let f = fs::File::open(file).expect("open stdin fixture");
    Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(args)
        .stdin(Stdio::from(f))
        .output()
        .expect("spawn jdiff")
}

/// Runs `jdiff` with `content` fed over a real PIPE (unseekable — exercises
/// the chkSeq auto-detection and its sequential warnings). The content must
/// stay well under the 64 KiB pipe buffer so the parent can write it before
/// waiting (all fixtures here are <= 16 KiB).
fn run_stdin_pipe(args: &[&OsStr], content: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jdiff"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn jdiff");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(content)
        .expect("write stdin pipe");
    child.wait_with_output().expect("wait jdiff")
}

fn stderr_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Copies the test binary into `dir` under the given argv[0] base name and
/// returns the copy's path. The `.exe` suffix is appended on Windows
/// (`std::env::consts::EXE_SUFFIX`): `CreateProcessW` would not find an
/// extensionless image, while the dispatch only inspects the basename
/// prefix (`jpatch.exe` still starts with `jpatch`, spec §21.2).
fn copy_binary_as(dir: &Path, name: &str) -> PathBuf {
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_jdiff"), &exe).expect("copy binary");
    exe
}

/// Spawns a copy of the binary made with [`copy_binary_as`], retrying on
/// `ETXTBSY`: a concurrently-running test's child may inherit the copy's
/// still-open write handle at clone time (fd tables are copied at fork and
/// `fs::copy` opens without `O_CLOEXEC`) and keep it open through exec,
/// failing `execve` with `Text file busy` until that child exits.
fn run_copied(exe: &Path, args: &[&OsStr]) -> Output {
    for attempt in 0..100 {
        match Command::new(exe).args(args).output() {
            Ok(out) => return out,
            Err(err) if err.raw_os_error() == Some(26) && attempt < 99 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(err) => panic!("spawn {}: {err}", exe.display()),
        }
    }
    unreachable!("retry loop above either returns or panics")
}

/// P8zd width (`JDefs.h:118-126`): release `%12lld`, debug `%10lld`.
fn pw(v: i64) -> String {
    if cfg!(feature = "debug") {
        format!("{v:>10}")
    } else {
        format!("{v:>12}")
    }
}

/// The getopt error prefix: glibc prints `<argv0>: …` with argv[0] exactly as
/// passed (`main.cpp` inherits this from libc; the port replicates it).
fn getopt_err(rest: &str) -> String {
    format!("{}: {}\n", env!("CARGO_BIN_EXE_jdiff"), rest)
}

// ===========================================================================
// Help, usage, greeting (§18.A) — exit codes per §18.D
// ===========================================================================

/// `jdiff -h`: greeting + usage on stderr, exit 2, stdout empty
/// (`main.cpp:480-599`; oracle-pinned byte-for-byte).
#[test]
fn help_exit_code_and_text() {
    let out = run(&[OsStr::new("-h")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr_str(&out), GREETING.to_string() + &usage(2, 128));
    assert!(out.stdout.is_empty());
}

/// `jdiff -hh`: additionally the Notes / Additional explications block
/// (`liHlp > 1`, `main.cpp:559-593`); still exit 2 (nargs < 3) without the
/// "Not enough arguments" line (liHlp != 0).
#[test]
fn hh_shows_notes() {
    let out = run(&[OsStr::new("-hh")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_str(&out),
        GREETING.to_string() + &usage(2, 128) + NOTES
    );
    assert!(out.stdout.is_empty());
}

/// 0 or 1 file args: greeting + usage + the "Not enough arguments" error line,
/// exit 2 (`main.cpp:594-599` — the error line only when liHlp == 0).
#[test]
fn missing_args_exit_2() {
    let expected = GREETING.to_string()
        + &usage(2, 128)
        + "Error: Not enough arguments have been specified !\n";
    for args in [
        vec![],
        vec![OsStr::new("some-file")],
        vec![OsStr::new("-v"), OsStr::new("some-file")],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2), "args {args:?}");
        assert_eq!(stderr_str(&out), expected, "args {args:?}");
        assert!(out.stdout.is_empty());
    }
}

/// The stale usage texts are replicated verbatim (spec §21.10): "(default 64)"
/// for -i, "(default 8192)" for -k, "(in KB)" and "0=no buffering" for -m, and
/// the "disbale" typo in the verbose echo.
#[test]
fn usage_contains_stale_texts() {
    let out = run(&[OsStr::new("-h")]);
    let text = stderr_str(&out);
    assert!(text.contains("(default 64)"), "stale -i default: {text:?}");
    assert!(
        text.contains("(default 8192)"),
        "stale -k default: {text:?}"
    );
    assert!(text.contains("(in KB)"), "stale -m unit: {text:?}");
    assert!(text.contains("0=no buffering"), "stale -m 0 mode: {text:?}");
    // The verbose echo's "disbale" typo (main.cpp:834) — assertable via -vv.
    let dir = temp_dir("disbale");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let out = run(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        dir.join("p.bin").as_os_str(),
    ]);
    assert!(
        stderr_str(&out).contains("(-ff to disbale)"),
        "verbose echo typo: {:?}",
        stderr_str(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Exit-code swap and patch output (§18.D)
// ===========================================================================

/// Identical fixture pair: exit 0 (0.8.5 swapped 0.8.1's mapping) and the
/// patch is the single EQL record — `ESC EQL <len-1>` = 3 bytes for the 18-byte
/// file, oracle-pinned (`A7 A3 11`).
#[test]
fn equal_files_exit_0_and_eql_patch() {
    let dir = temp_dir("equal");
    let a = write_file(&dir.join("a.bin"), b"abcdefghijklmnopqr");
    let b = write_file(&dir.join("b.bin"), b"abcdefghijklmnopqr");
    let outp = dir.join("p.bin");

    let out = run(&[a.as_os_str(), b.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(fs::read(&outp).unwrap(), [0xA7, 0xA3, 0x11]);
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Differing fixture pair: exit 1, patch byte-exact with the oracle.
#[test]
fn differ_files_exit_1_nonempty_patch() {
    let dir = temp_dir("differ");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run(&[a.as_os_str(), b.as_os_str(), outp.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// With only two file arguments the patch goes to stdout and stddbg stays
/// empty (`main.cpp:607-610`).
#[test]
fn missing_output_goes_to_stdout() {
    let dir = temp_dir("stdout");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, PATCH_AB);
    assert!(out.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Unopenable original file: exit 3, `Could not open first file %s for
/// reading.` (`main.cpp:744-747`).
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

/// Unopenable new file: exit 4 (`main.cpp:749-752`).
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

/// Unopenable output file: exit 5 (`main.cpp:770-773`).
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
    assert!(!bad_out.exists(), "output must not be created");
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Verbose streams (§18.A/§18.F), oracle-pinned
// ===========================================================================

/// `jdiff -v` on the tiny pair: stderr is the byte-exact greeting, the
/// "Use -h" hint, the engine's progress blocks and the four verbose>0
/// statistics lines (oracle capture, 1072 bytes).
#[test]
fn greeting_matches_reference() {
    let dir = temp_dir("greet");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let expected = concat!(
        "\nJDIFF - binary diff version 0.8.5 (beta) 2020\n",
        "Copyright (C) 2002-2020 Joris Heirbaut\n\n",
        "JojoDiff is free software: you can redistribute it and/or modify it\n",
        "under the terms of the  GNU General Public License  as published by\n",
        "the Free Software Foundation,  either version 3 of the License,  or\n",
        "(at your option) any later version.\n\n",
        "This program is distributed in the hope that it will be useful,\n",
        "but WITHOUT ANY WARRANTY; without even the implied warranty of\n",
        "MERCHANTABILITY  or  FITNESS FOR A PARTICULAR PURPOSE. See the\n",
        "GNU General Public License for more details.\n\n",
        "You should have received a copy of the GNU General Public License\n",
        "along with this program. If not, see www.gnu.org/licenses/gpl-3.0\n\n",
        "File adressing is 64 bit for files up to 8388607TB, samples are 32 bytes.\n",
        "\nUse -h for additional help and usage description.\n",
        "Comparing : ...           \n",
        "Indexing  : ...           \u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}           0Mb\n",
        "Comparing : ...           \u{d}Comparing :            0Mb\n",
        "Equal       bytes       = 12\n",
        "Data        bytes       = 6\n",
        "Control-Esc bytes       = 5\n",
        "Total       bytes       = 11\n",
    );

    let out = run(&[
        OsStr::new("-v"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr_str(&out), expected);
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// `jdiff -vv` on the tiny pair: additionally the pre-run echo and the
/// verbose>1 statistics blocks, ending with the "Not all data …" verdict
/// (`main.cpp:823-869`; oracle capture with the one documented port
/// deviation applied):
///
/// 1. "Index table size" prints the port's table — the same prime the
///    32-bit-hkey oracle prints since the §21.18 divisor ruling (default
///    32 MB → 2796181 samples; before it, the port's /16 table printed
///    24Mb/2097143). The 1:1 main.cpp formula prints whatever table the
///    port built — port value pinned, oracle-identical.
/// 2. "Inaccurate  solutions" prints 0: the C++ never initializes the
///    counter (stack garbage, oracle printed 908602116 here) and the port
///    deterministically 0-initializes it (spec §21.6).
#[test]
fn stats_lines_match_reference() {
    let dir = temp_dir("stats");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let expected = concat!(
        "\nJDIFF - binary diff version 0.8.5 (beta) 2020\n",
        "Copyright (C) 2002-2020 Joris Heirbaut\n\n",
        "JojoDiff is free software: you can redistribute it and/or modify it\n",
        "under the terms of the  GNU General Public License  as published by\n",
        "the Free Software Foundation,  either version 3 of the License,  or\n",
        "(at your option) any later version.\n\n",
        "This program is distributed in the hope that it will be useful,\n",
        "but WITHOUT ANY WARRANTY; without even the implied warranty of\n",
        "MERCHANTABILITY  or  FITNESS FOR A PARTICULAR PURPOSE. See the\n",
        "GNU General Public License for more details.\n\n",
        "You should have received a copy of the GNU General Public License\n",
        "along with this program. If not, see www.gnu.org/licenses/gpl-3.0\n\n",
        "File adressing is 64 bit for files up to 8388607TB, samples are 32 bytes.\n",
        "\nUse -h for additional help and usage description.\n",
        "\n",
        "Index table size (default: 64Mb) (-s): 32Mb (2796181 samples)\n",
        "Search size     (0 = buffersize) (-a): 992kb\n",
        "Buffer size       (default  2Mb) (-m): 2Mb\n",
        "Block  size       (default 32kb) (-b): 32kb\n",
        "Min number of matches to search  (-n): 2\n",
        "Max number of matches to search  (-x): 128\n",
        "Compare out-of-buffer (-f to disable): yes\n",
        "Full indexing scan   (-ff to disbale): yes\n",
        "Backtrace allowed     (-p to disable): yes\n",
        "Comparing : ...           \n",
        "Indexing  : ...           \u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}           0Mb\n",
        "Comparing : ...           \u{d}Comparing :            0Mb\n",
        "Index table hits        = 0\n",
        "Index table repairs     = 0\n",
        "Index table overloading = 0\n",
        "Reliability distance    = 48\n",
        "Inaccurate  solutions   = 0\n",
        "Source      seeks       = 0\n",
        "Destination seeks       = 0\n",
        "Delete      bytes       = 0\n",
        "Backtrack   bytes       = 0\n",
        "Escape      bytes       = 0\n",
        "Control     bytes       = 5\n",
        "\n",
        "Equal       bytes       = 12\n",
        "Data        bytes       = 6\n",
        "Control-Esc bytes       = 5\n",
        "Total       bytes       = 11\n",
        "\n",
        "Not all data has been found in source file.\n",
    );

    let out = run(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr_str(&out), expected);
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// An identical pair at -vv ends with exit 0 and the "Found all data …"
/// verdict (`main.cpp:921-924`).
#[test]
fn identical_at_verbose_ends_found_all() {
    let dir = temp_dir("foundall");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("a2.bin"), ORG_A);
    let outp = dir.join("p.bin");

    let out = run(&[
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stderr_str(&out).ends_with("\nFound all data within source file.\n"));
    fs::remove_dir_all(&dir).unwrap();
}

/// Option parsing precedes the greeting: the `-i`/`-k` warnings print BEFORE
/// it, `-c` moves the whole stream to stdout (main.cpp:412/419 write to
/// JDebug::stddbg during parsing).
#[test]
fn warnings_precede_greeting_and_console_switch() {
    let dir = temp_dir("warnorder");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    // -i 0: warning first, then greeting (verbose), exit 1.
    let out = run(&[
        OsStr::new("-i"),
        OsStr::new("0"),
        OsStr::new("-v"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_str(&out).starts_with(
        "Warning: invalid --index-size/-i specified, set to 1.\n\nJDIFF - binary diff version"
    ));

    // -c: the same stream on stdout, stderr silent.
    let out = run(&[
        OsStr::new("-c"),
        OsStr::new("-i"),
        OsStr::new("0"),
        OsStr::new("-v"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "{:?}", stderr_str(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .starts_with("Warning: invalid --index-size/-i specified, set to 1.\n")
    );
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// getopt_long surface (§18.D): permutation, --, errors, presets
// ===========================================================================

/// Unknown option: glibc prints its error line, `?` sets liHlp=1 and parsing
/// CONTINUES — help prints, then the diff runs (`main.cpp:473-475`):
/// `-Z a b` diffs to stdout, exit 1.
#[test]
fn unknown_option_help_then_diff() {
    let dir = temp_dir("unkopt");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[OsStr::new("-Z"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr_str(&out),
        getopt_err("invalid option -- 'Z'") + GREETING + &usage(2, 128)
    );
    assert_eq!(out.stdout, PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// `-h a b c`: help printed AND the diff runs to the third operand
/// (liHlp > 0, nargs >= 3 → no exit; `main.cpp:594-599` only exits below
/// 3 operands).
#[test]
fn help_then_diff_three_operands() {
    let dir = temp_dir("helpdiff");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("c.out");

    let out = run(&[
        OsStr::new("-h"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr_str(&out), GREETING.to_string() + &usage(2, 128));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    assert!(out.stdout.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// GNU permutation: options may follow the filenames (`jdiff a b -l` lists to
/// stdout; `jdiff a -l b out` writes the listing to out).
#[test]
fn gnu_permutation_options_after_files() {
    let dir = temp_dir("perm");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[a.as_os_str(), b.as_os_str(), OsStr::new("-l")]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout
            .starts_with(b"           0            0 EQL 68 68 h-h\n"),
        "trailing -l must list: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let outp = dir.join("o.asc");
    let out = run(&[
        a.as_os_str(),
        OsStr::new("-l"),
        b.as_os_str(),
        OsStr::new("-v"),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(fs::read(&outp).unwrap().starts_with(b"           0"));
    assert!(stderr_str(&out).contains("Use -h for additional help"));
    fs::remove_dir_all(&dir).unwrap();
}

/// `--` ends option processing; everything after it is an operand (including
/// further `--` tokens): `jdiff -- a b -- o` writes the patch to a file
/// literally named `--` (oracle-verified).
#[test]
fn double_dash_ends_options() {
    let dir = temp_dir("ddash");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    // The third operand is a file NAMED `--`; absolute so the patch lands in
    // the temp dir, not the test process's working directory.
    let dash = dir.join("--");

    let out = run(&[
        OsStr::new("--"),
        a.as_os_str(),
        b.as_os_str(),
        dash.as_os_str(),
        OsStr::new("o.bin"),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        fs::read(&dash).unwrap(),
        PATCH_AB,
        "patch must land in the file named --"
    );
    assert!(
        !dir.join("o.bin").exists(),
        "o.bin is a 5th operand, unused"
    );
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Missing required argument at the end of argv (bare `-d`): glibc error,
/// `?` → help, nargs < 3 → exit 2.
#[test]
fn missing_arg_bare_d_exit_2() {
    let out = run(&[OsStr::new("-d")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_str(&out),
        getopt_err("option requires an argument -- 'd'") + GREETING + &usage(2, 128)
    );
    assert!(out.stdout.is_empty());
}

/// Long-option errors (glibc texts, oracle-pinned):
/// - unrecognized `--nope` → help + diff continues (exit 1);
/// - `--help=x` → "doesn't allow an argument", help + diff continues;
/// - bare `--index-size` → "requires an argument", help, exit 2;
/// - ambiguous prefix `--s` → "is ambiguous; possibilities: …", diff continues.
#[test]
fn long_option_errors() {
    let dir = temp_dir("lngerr");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[OsStr::new("--nope"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_str(&out).starts_with(&getopt_err("unrecognized option '--nope'")));
    assert_eq!(out.stdout, PATCH_AB);

    let out = run(&[OsStr::new("--help=x"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).starts_with(&getopt_err("option '--help' doesn't allow an argument")),
        "{:?}",
        stderr_str(&out)
    );
    assert_eq!(out.stdout, PATCH_AB);

    let out = run(&[OsStr::new("--index-size")]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr_str(&out).starts_with(&getopt_err("option '--index-size' requires an argument")),
        "{:?}",
        stderr_str(&out)
    );

    let out = run(&[OsStr::new("--s"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).starts_with(&getopt_err(
            "option '--s' is ambiguous; possibilities: '--sequential-source' \
             '--sequential-dest' '--stdio' '--search-size' '--search-min' '--search-max'"
        )),
        "{:?}",
        stderr_str(&out)
    );
    assert_eq!(out.stdout, PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// Unambiguous long-option prefixes resolve (`--und` → --undiff) and
/// `--opt=value` attaches arguments (`--index-size=1`).
#[test]
fn long_option_abbreviation_and_equals() {
    let dir = temp_dir("lngok");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let outp = dir.join("o.bin");

    // --und applies the patch.
    let out = run(&[
        OsStr::new("--und"),
        a.as_os_str(),
        p.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_str(&out));
    assert_eq!(fs::read(&outp).unwrap(), NEW_B);

    // --index-size=1 pins the 1 MB table ("1Mb" in the echo).
    let out = run(&[
        OsStr::new("--index-size=1"),
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        dir.join("p2.bin").as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).contains("Index table size (default: 64Mb) (-s): 1Mb (87359 samples)"),
        "port 1MB table: {:?}",
        stderr_str(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `-d <name>` consumes a name argument (in release builds any name is
/// silently accepted — the strcmp arms are compiled out, `main.cpp:446-472`):
/// `jdiff -d hsh a b p` diffs normally; a BARE `-dhsh` is `-d` with attached
/// argument "hsh", NOT a filename (0.8.5 getopt syntax).
#[test]
fn d_option_consumes_argument() {
    let dir = temp_dir("darg");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[
        OsStr::new("-d"),
        OsStr::new("hsh"),
        a.as_os_str(),
        b.as_os_str(),
        dir.join("p.bin").as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    assert_eq!(fs::read(dir.join("p.bin")).unwrap(), PATCH_AB);

    // Attached form: -dhsh = -d hsh → not a filename.
    let out = run(&[
        OsStr::new("-dhsh"),
        a.as_os_str(),
        b.as_os_str(),
        dir.join("p2.bin").as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(dir.join("p2.bin")).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

/// Multiplicative presets in parse order leak into the usage `-n`/`-x` lines:
/// `-b` doubles min and quadruples max; a second `-b` compounds
/// (`main.cpp:320-330`).
#[test]
fn presets_leak_into_usage_defaults() {
    let out = run(&[OsStr::new("-b"), OsStr::new("-h")]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr_str(&out);
    assert!(
        text.contains("Minimum number of matches to search (default 4)."),
        "-b -n line: {text:?}"
    );
    assert!(
        text.contains("Maximum number of matches to search (default 512)."),
        "-b -x line: {text:?}"
    );

    // -bb: 2*2=4 min, 128*4*4=2048 max, hshMbt 32*4*4=512.
    let out = run(&[OsStr::new("-bb"), OsStr::new("-h")]);
    assert_eq!(out.status.code(), Some(2));
    let text = stderr_str(&out);
    assert!(
        text.contains("Minimum number of matches to search (default 8)."),
        "-bb -n line: {text:?}"
    );
    assert!(
        text.contains("Maximum number of matches to search (default 2048)."),
        "-bb -x line: {text:?}"
    );

    // Presets then explicit values: -n/-x override multiplicatively.
    let out = run(&[
        OsStr::new("-b"),
        OsStr::new("-n"),
        OsStr::new("1"),
        OsStr::new("-h"),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_str(&out).contains("Minimum number of matches to search (default 1)."));
}

/// `-m` echoes (MB total, split evenly; main.cpp:422-434/617-620):
/// `-m 0` → defaults (2Mb), `-m 7` → 6Mb (3+3), `-m 2048` → 2048Mb,
/// second `-m` → org doubles and new resets (`-m 7 -m 5` → 11Mb),
/// third and subsequent ignored. Oracle-verified values.
#[test]
fn m_option_echo_lines() {
    let dir = temp_dir("mecho");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    fn buffer_line(out: &Output) -> String {
        stderr_str(out)
            .lines()
            .find(|l| l.starts_with("Buffer size"))
            .expect("Buffer size echo line")
            .to_string()
    }
    fn search_line(out: &Output) -> String {
        stderr_str(out)
            .lines()
            .find(|l| l.starts_with("Search size"))
            .expect("Search size echo line")
            .to_string()
    }

    for (args, want) in [
        (
            vec!["-m", "0"],
            "Buffer size       (default  2Mb) (-m): 2Mb",
        ),
        (
            vec!["-m", "7"],
            "Buffer size       (default  2Mb) (-m): 6Mb",
        ),
        (
            vec!["-m", "2048"],
            "Buffer size       (default  2Mb) (-m): 2048Mb",
        ),
        (
            vec!["-m", "7", "-m", "5"],
            "Buffer size       (default  2Mb) (-m): 11Mb",
        ),
        (
            vec!["-m", "7", "-m", "5", "-m", "9"],
            "Buffer size       (default  2Mb) (-m): 11Mb",
        ),
    ] {
        let mut full: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        full.extend([
            OsStr::new("-vv"),
            a.as_os_str(),
            b.as_os_str(),
            outp.as_os_str(),
        ]);
        let out = run(&full);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(buffer_line(&out), want, "{args:?}");
    }

    // Search size defaults to dest buffer minus block size.
    let out = run(&[
        OsStr::new("-m"),
        OsStr::new("7"),
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(
        search_line(&out),
        "Search size     (0 = buffersize) (-a): 3040kb"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `-i`/`-k` clamping (main.cpp:408-421, 617-621): `-i 1` → 1Mb table
/// (87359 samples in the port, §21.18), `-i 0` warns and pins 1; `-k 0`
/// warns and floors to 4096 ("4kb", search 1020kb); `-k 65565` misaligns
/// both buffers (warnings, "set to 983475", search 896kb, "64kb" block
/// line). All oracle-verified values.
#[test]
fn i_and_k_clamps_and_misalign_warnings() {
    let dir = temp_dir("ikclamp");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let run_vv = |extra: &[&str]| -> Output {
        let mut full: Vec<&OsStr> = extra.iter().map(OsStr::new).collect();
        full.extend([
            OsStr::new("-vv"),
            a.as_os_str(),
            b.as_os_str(),
            outp.as_os_str(),
        ]);
        run(&full)
    };

    let out = run_vv(&["-i", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).contains("Index table size (default: 64Mb) (-s): 1Mb (87359 samples)"),
        "-i 1: {:?}",
        stderr_str(&out)
    );

    let out = run_vv(&["-i", "0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).starts_with("Warning: invalid --index-size/-i specified, set to 1.\n")
    );
    assert!(stderr_str(&out).contains("Index table size (default: 64Mb) (-s): 1Mb"));

    let out = run_vv(&["-k", "0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).starts_with("Warning: invalid --block-size/-k specified, set to 1.\n")
    );
    let text = stderr_str(&out);
    assert!(
        text.contains("Block  size       (default 32kb) (-b): 4kb"),
        "{text:?}"
    );
    assert!(
        text.contains("Search size     (0 = buffersize) (-a): 1020kb"),
        "{text:?}"
    );

    let out = run_vv(&["-k", "65565"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stderr_str(&out);
    assert!(
        text.contains("Warning: Source buffer size misaligned with block size: set to 983475.\n"),
        "{text:?}"
    );
    assert!(
        text.contains(
            "Warning: Destination buffer size misaligned with block size: set to 983475.\n"
        ),
        "{text:?}"
    );
    assert!(
        text.contains("Search size     (0 = buffersize) (-a): 896kb"),
        "{text:?}"
    );
    assert!(
        text.contains("Block  size       (default 32kb) (-b): 64kb"),
        "{text:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `-x 5` runs (the §21.15 quirk: the unclamped value sizes the bucket prime
/// — getLowerPrime(10) = 7 — while the table itself is clamped to 13) and
/// `-x 0` floors to 1024; `-n` floors at 0.
#[test]
fn x_option_floors_and_runs() {
    let dir = temp_dir("xfloor");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run(&[
        OsStr::new("-x"),
        OsStr::new("5"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);

    let out = run(&[
        OsStr::new("-x"),
        OsStr::new("0"),
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).contains("Max number of matches to search  (-x): 1024"),
        "{:?}",
        stderr_str(&out)
    );

    let out = run(&[
        OsStr::new("-n"),
        OsStr::new("-7"),
        OsStr::new("-vv"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).contains("Min number of matches to search  (-n): 0"),
        "{:?}",
        stderr_str(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `-s` is accepted and recorded without observable effect (spec §21.11):
/// the run behaves exactly like the default.
#[test]
fn s_option_accepted_without_effect() {
    let dir = temp_dir("sopt");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run(&[
        OsStr::new("-s"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    assert!(stderr_str(&out).is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// stdin / stdout / sequential (§18.D)
// ===========================================================================

/// Both inputs `-`: exit 2 with the exact message and NO greeting
/// (`main.cpp:612-615`; nargs = 3 → greeting condition false).
#[test]
fn both_inputs_dash_exit_2() {
    let out = run(&[OsStr::new("-"), OsStr::new("-")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_str(&out),
        "Error: Original and destination files cannot both be from standard input !\n"
    );
    assert!(out.stdout.is_empty());
}

/// `- a b` with a file-redirected stdin: normal random-access diff on Unix
/// (`/dev/stdin` re-open is seekable, like the C++ `cin` on a redirected
/// file) — patch identical to the file variant, exit 1, silent. On Windows
/// there is no `/dev/stdin`, so the input falls back to the un-seekable
/// stdin handle, chkSeq auto-assumes `-p` and the documented warning prints
/// (README platform note); the patch then equals the explicit `-p` run —
/// the same adjudication as [`pipe_source_auto_p_warning_and_roundtrip`]
/// (bytes verified identical to the explicit `-p` output on the oracle).
#[test]
fn dash_reads_stdin_org() {
    let dir = temp_dir("dashorg");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run_stdin_file(&[OsStr::new("-"), b.as_os_str(), outp.as_os_str()], &a);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    if cfg!(windows) {
        assert_eq!(
            stderr_str(&out),
            "\nWarning: Source file is a sequential file, assuming -p.\n"
        );
        // Sequential (-p) semantics: bytes equal the explicit `-p` run.
        let reff = dir.join("pref.bin");
        let refo = run(&[
            OsStr::new("-p"),
            a.as_os_str(),
            b.as_os_str(),
            reff.as_os_str(),
        ]);
        assert_eq!(refo.status.code(), Some(1), "{}", stderr_str(&refo));
        assert_eq!(
            fs::read(&outp).unwrap(),
            fs::read(&reff).unwrap(),
            "stdin-org patch must equal the explicit -p patch"
        );
    } else {
        assert!(stderr_str(&out).is_empty());
        assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// Piped destination: `cat b | jdiff a -` auto-detects sequential input,
/// prints the -q warning, exits 1 and the patch round-trips through -u
/// (`main.cpp:791-795`).
#[test]
fn pipe_destination_auto_q_warning_and_roundtrip() {
    let dir = temp_dir("pipeq");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let outp = dir.join("p.bin");

    let out = run_stdin_pipe(&[a.as_os_str(), OsStr::new("-"), outp.as_os_str()], NEW_B);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr_str(&out),
        "\nWarning: Destination file is a sequential file, assuming -q.\n"
    );

    let out2 = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        outp.as_os_str(),
        dir.join("restored.bin").as_os_str(),
    ]);
    assert_eq!(out2.status.code(), Some(0), "{}", stderr_str(&out2));
    assert_eq!(fs::read(dir.join("restored.bin")).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// Piped source with explicit `-p` (`cat a | jdiff -p - b`): no warning (the
/// option was explicit), exit 1, round-trips.
#[test]
fn pipe_source_explicit_p_roundtrip() {
    let dir = temp_dir("pipep");
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run_stdin_pipe(
        &[
            OsStr::new("-p"),
            OsStr::new("-"),
            b.as_os_str(),
            outp.as_os_str(),
        ],
        ORG_A,
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    assert!(stderr_str(&out).is_empty(), "explicit -p must not warn");

    // Restore from the piped-source patch with the original from a file.
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out2 = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        outp.as_os_str(),
        dir.join("restored.bin").as_os_str(),
    ]);
    assert_eq!(out2.status.code(), Some(0), "{}", stderr_str(&out2));
    assert_eq!(fs::read(dir.join("restored.bin")).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// Piped source without `-p` (`cat a | jdiff - b`): the sequential source is
/// auto-detected, the -p warning prints, exit 1, and the patch is
/// byte-identical to the explicit `-p` one (oracle-verified: the branch's
/// `cmp_all`/`src_bkt`/`src_scn` mutations change no patch bytes on this
/// fixture) and round-trips through -u (`main.cpp:781-788`).
#[test]
fn pipe_source_auto_p_warning_and_roundtrip() {
    let dir = temp_dir("pipeautop");
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("p.bin");

    let out = run_stdin_pipe(&[OsStr::new("-"), b.as_os_str(), outp.as_os_str()], ORG_A);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr_str(&out),
        "\nWarning: Source file is a sequential file, assuming -p.\n"
    );
    assert_eq!(
        fs::read(&outp).unwrap(),
        PATCH_AB,
        "auto-detect patch must equal the explicit -p patch (oracle-verified)"
    );

    // Restore from the piped-source patch with the original from a file.
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out2 = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        outp.as_os_str(),
        dir.join("restored.bin").as_os_str(),
    ]);
    assert_eq!(out2.status.code(), Some(0), "{}", stderr_str(&out2));
    assert_eq!(fs::read(dir.join("restored.bin")).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// Piped patch: `cat p | jdiff -u a -` applies to stdout, exit 0
/// (spec §18.D verified pipe flow).
#[test]
fn pipe_patch_through_u() {
    let dir = temp_dir("pipeu");
    let a = write_file(&dir.join("a.bin"), ORG_A);

    let out = run_stdin_pipe(
        &[OsStr::new("-u"), a.as_os_str(), OsStr::new("-")],
        PATCH_AB,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, NEW_B);
    assert!(out.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// argv[0] dispatch (§18.B/§21.2)
// ===========================================================================

/// Copies of the binary named `jpatch` and `jptch` both apply patches (the
/// `jptch` match is the port extension — upstream matches `jpatch` only, spec
/// §21.2); `jdedup` is NOT a ported route and falls through to Diff.
#[test]
fn argv0_dispatch_jpatch_jptch_jdedup() {
    let dir = temp_dir("argv0");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);

    for name in ["jpatch", "jptch", "jdedup", "jtst"] {
        let exe = copy_binary_as(&dir, name);
        let outp = dir.join(format!("out-{name}.bin"));
        if name.starts_with("jpatch") || name.starts_with("jptch") {
            let out = run_copied(&exe, &[a.as_os_str(), p.as_os_str(), outp.as_os_str()]);
            assert_eq!(out.status.code(), Some(0), "{name}: {}", stderr_str(&out));
            assert_eq!(
                fs::read(&outp).unwrap(),
                NEW_B,
                "{name} must patch (argv[0] dispatch)"
            );
            assert!(out.stderr.is_empty(), "{name}: {:?}", stderr_str(&out));
        } else {
            // jdedup/jtst routes are not ported: fall through to Diff
            // (no crash).
            let out = run_copied(&exe, &[a.as_os_str(), b.as_os_str(), outp.as_os_str()]);
            assert_eq!(out.status.code(), Some(1), "{name} must diff");
            assert_eq!(fs::read(&outp).unwrap(), PATCH_AB, "{name} diff output");
        }
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// The argv[0] copies carry a `.exe` suffix on Windows (CreateProcessW only
/// finds `name.exe`, and the dispatch matches the basename prefix —
/// [`copy_binary_as`]) and the plain name elsewhere.
#[test]
fn argv0_copy_name_matches_platform() {
    let dir = temp_dir("argv0-suffix");
    let exe = copy_binary_as(&dir, "jptch");
    let name = exe.file_name().expect("copied file name").to_string_lossy();
    if cfg!(windows) {
        assert_eq!(name, "jptch.exe");
    } else {
        assert_eq!(name, "jptch");
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// `-j` forces Diff even under a `jpatch`-named argv[0]
/// (`main.cpp:366-368` overrides the dispatch).
#[test]
fn argv0_jpatch_with_jflag_diffs() {
    let dir = temp_dir("argv0j");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let exe = copy_binary_as(&dir, "jpatch");

    let outp = dir.join("p.bin");
    let out = run_copied(
        &exe,
        &[
            OsStr::new("-j"),
            a.as_os_str(),
            b.as_os_str(),
            outp.as_os_str(),
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&outp).unwrap(), PATCH_AB);
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Function options -t and -y (§21.3/§21.4)
// ===========================================================================

/// `-t` (Test) is ported faithfully although broken upstream (spec §21.3):
/// the diff runs, then JPatcht is fed the DESTINATION as the patch, appending
/// misparsed data to the patch output; exit follows the patch phase (0).
/// For the tiny pair the destination is pure data (no ESC), so the appended
/// garbage is the whole destination file.
#[test]
fn t_option_release_corrupt_mixed_output() {
    let dir = temp_dir("topt");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("t.bin");

    let out = run(&[
        OsStr::new("-t"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_str(&out));
    let mut corrupt = PATCH_AB.to_vec();
    corrupt.extend_from_slice(NEW_B);
    assert_eq!(
        fs::read(&outp).unwrap(),
        corrupt,
        "-t appends the misparsed destination"
    );

    // Same without an output file: corrupt stream on stdout.
    let out = run(&[OsStr::new("-t"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, corrupt);
    fs::remove_dir_all(&dir).unwrap();
}

/// `-t` in debug builds: the port exits 0 with the same corrupt mixed output
/// as the release build. Upstream's debug build asserts at the mid-cursor
/// handoff instead — the diff phase leaves the destination reader's cursor
/// at -1 (EOF) and the patch phase's first zero-arg `get()` re-issues there,
/// tripping the always-on getbuf assert (debug oracle, verified today:
/// `JFileAhead::getbuf(New,-1,1,0)-> 0 (sto 0x…) failed !`, exit 6). The
/// port's `-t` re-opens its inputs fresh (deviation 2), so that -1 state
/// never exists, and the negative-EOF gate (ahead.rs deviation 4) ends the
/// file there in every build — the assert is unreachable (spec §21.3
/// release shape kept; the debug-specific upstream exit 6 is not).
#[cfg(feature = "debug")]
#[test]
fn t_option_debug_matches_release() {
    let dir = temp_dir("tdebug");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let outp = dir.join("t.bin");

    let out = run(&[
        OsStr::new("-t"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{:?}", stderr_str(&out));
    assert!(stderr_str(&out).is_empty(), "{:?}", stderr_str(&out));
    let mut corrupt = PATCH_AB.to_vec();
    corrupt.extend_from_slice(NEW_B);
    assert_eq!(
        fs::read(&outp).unwrap(),
        corrupt,
        "-t appends the misparsed destination in debug builds too"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `-y` (Dedup) is accepted but the function is not ported (spec §21.4):
/// exit 20 via the "Error occurred !" path.
#[test]
fn y_option_exit_20() {
    let dir = temp_dir("yopt");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[OsStr::new("-y"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(20));
    assert_eq!(stderr_str(&out), "\nError occurred !\n");
    assert!(out.stdout.is_empty());

    // Long form and open errors keep their precedence (files open first).
    let out = run(&[OsStr::new("--reflink"), a.as_os_str(), b.as_os_str()]);
    assert_eq!(out.status.code(), Some(20));
    let out = run(&[
        OsStr::new("-y"),
        dir.join("missing.bin").as_os_str(),
        b.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(3));
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Round-trip matrix (spec §22.1): `jdiff OPTS a b p && jdiff -u a p out`
// over both fixture pairs; -l/-r produce listings, so their round-trip
// patch comes from a default run (brief's instruction).
// ===========================================================================

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (x >> 16) as u8
        })
        .collect()
}

/// 16 KiB fixture pair exercising every engine path: long equal runs,
/// deletion, re-inserted earlier data (BKT), MOD/INS data and operator-valued
/// bytes 0xA2..=0xA7 (ESC-escaped patch data).
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

/// The §22.1 option matrix (patch-producing sets and the two listing sets).
fn option_matrix() -> Vec<(&'static str, Vec<&'static str>, bool)> {
    vec![
        ("default", vec![], false),
        ("-b", vec!["-b"], false),
        ("-bb", vec!["-bb"], false),
        ("-f", vec!["-f"], false),
        ("-ff", vec!["-ff"], false),
        ("-p", vec!["-p"], false),
        ("-q", vec!["-q"], false),
        ("-p -q", vec!["-p", "-q"], false),
        ("-s", vec!["-s"], false),
        ("-i 1", vec!["-i", "1"], false),
        ("-i 8", vec!["-i", "8"], false),
        ("-i 512", vec!["-i", "512"], false),
        ("-k 0", vec!["-k", "0"], false),
        ("-k 1", vec!["-k", "1"], false),
        ("-k 65565", vec!["-k", "65565"], false),
        ("-n 1 -x 2", vec!["-n", "1", "-x", "2"], false),
        ("-x 5", vec!["-x", "5"], false),
        ("-m 0", vec!["-m", "0"], false),
        ("-m 7", vec!["-m", "7"], false),
        ("-m 2048", vec!["-m", "2048"], false),
        ("-a 0", vec!["-a", "0"], false),
        ("-a 1", vec!["-a", "1"], false),
        ("-l", vec!["-l"], true),
        ("-r", vec!["-r"], true),
    ]
}

/// The §22.1 round-trip gate over the CLI: every option set must produce a
/// patch (exit 1 — the pairs differ) that `jdiff -u` applies back to the new
/// file byte-exact (exit 0). Listing sets produce non-empty listings and
/// round-trip a default patch. Stderr stays clean (no warnings) for all
/// file-based runs.
#[test]
fn roundtrip_all_option_sets() {
    let dir = temp_dir("rt-matrix");
    let pairs: [(&str, Vec<u8>, Vec<u8>); 2] = [
        ("tiny", ORG_A.to_vec(), NEW_B.to_vec()),
        ("big", big_pair().0, big_pair().1),
    ];

    for (pair_name, org, new) in &pairs {
        let pdir = dir.join(pair_name);
        fs::create_dir_all(&pdir).expect("create pair dir");
        let a = write_file(&pdir.join("a.bin"), org);
        let b = write_file(&pdir.join("b.bin"), new);

        for (label, opts, is_listing) in option_matrix() {
            let tag = label.replace(['-', ' '], "");
            let patch = pdir.join(format!("p{tag}.jdf"));
            let out = pdir.join("out.bin");

            if is_listing {
                // Listings are diagnostics, not patches: run the listing set
                // (non-empty output) and round-trip a default patch.
                let listing = pdir.join(format!("l{tag}.txt"));
                let mut largs: Vec<&OsStr> = opts.iter().map(OsStr::new).collect();
                largs.extend([a.as_os_str(), b.as_os_str(), listing.as_os_str()]);
                let lrun = run(&largs);
                assert_eq!(
                    lrun.status.code(),
                    Some(1),
                    "{label} listing on {pair_name}: {}",
                    stderr_str(&lrun)
                );
                assert!(
                    fs::read(&listing).is_ok_and(|d| !d.is_empty()),
                    "{label} listing must be non-empty"
                );
                let mut dargs: Vec<&OsStr> = vec![];
                dargs.extend([a.as_os_str(), b.as_os_str(), patch.as_os_str()]);
                let drun = run(&dargs);
                assert_eq!(
                    drun.status.code(),
                    Some(1),
                    "{label} default diff on {pair_name}: {}",
                    stderr_str(&drun)
                );
            } else {
                let mut jargs: Vec<&OsStr> = opts.iter().map(OsStr::new).collect();
                jargs.extend([a.as_os_str(), b.as_os_str(), patch.as_os_str()]);
                let drun = run(&jargs);
                assert_eq!(
                    drun.status.code(),
                    Some(1),
                    "{label} diff on {pair_name}: {}",
                    stderr_str(&drun)
                );
                // `-k 0` prints main.cpp:419's clamp warning and `-k 65565`
                // main.cpp:629/635's misalignment warnings to stddbg
                // (oracle-verified); every other file-based run is silent.
                let expected: String = match label {
                    "-k 0" => "Warning: invalid --block-size/-k specified, set to 1.\n".into(),
                    "-k 65565" => concat!(
                        "Warning: Source buffer size misaligned with block size: set to 983475.\n",
                        "Warning: Destination buffer size misaligned with block size: set to 983475.\n",
                    )
                    .into(),
                    _ => String::new(),
                };
                assert_eq!(
                    stderr_str(&drun),
                    expected,
                    "{label} diff on {pair_name} stderr"
                );
            }

            let mut uargs: Vec<&OsStr> = vec![OsStr::new("-u")];
            uargs.extend([a.as_os_str(), patch.as_os_str(), out.as_os_str()]);
            let urun = run(&uargs);
            assert_eq!(
                urun.status.code(),
                Some(0),
                "{label} apply on {pair_name}: {}",
                stderr_str(&urun)
            );
            assert!(
                stderr_str(&urun).is_empty(),
                "{label} apply on {pair_name} must be silent: {:?}",
                stderr_str(&urun)
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

// ===========================================================================
// `--compat-081`: 0.8.1-format patch output (§21.16, §22.1)
// ===========================================================================

/// A bundled corpus fixture (`tests/fixtures/<name>`).
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Counts the data runs that begin **without** an explicit `ESC MOD` /
/// `ESC INS` operator ("implicit-MOD segments"), walking `patch` with the
/// JPatcht sequence grammar (`src/jpatcht.rs`, `JPatcht.cpp:209-338`): at a
/// sequence start (file start, after a DEL/EQL/BKT record, or at the end of
/// the stream) the 0.8.5 decoder defaults the operator to MOD and consumes
/// data unless the `ESC <opcode>` header is present. Inside a MOD/INS data
/// run, `ESC <same-opr>`, `ESC ESC` and `ESC <non-opcode>` are data;
/// `ESC <other-opcode>` starts a new record. A 0.8.1-style patch (the
/// `--compat-081` output, §21.16) has zero implicit-MOD segments.
fn count_implicit_mod_segments(patch: &[u8]) -> usize {
    // Opcode range BKT..=MOD (0xA2..=0xA6): BKT, EQL, DEL, INS, MOD.
    let is_opcode = |b: u8| (Op::Bkt.byte()..=Op::Mod.byte()).contains(&b);
    // Advances past one `ufGetInt` length tier (`JPatcht.cpp:50-85`); a
    // truncated length simply ends the walk.
    let skip_len = |patch: &[u8], i: &mut usize| {
        let Some(extra) = patch.get(*i).map(|b| match *b {
            0..=251 => 0,
            252 => 1,
            253 => 2,
            254 => 4,
            _ => 8,
        }) else {
            return;
        };
        *i = (*i + 1 + extra).min(patch.len());
    };

    let mut i = 0;
    let mut implicit = 0;
    // Some(op) = inside the data run of the explicit `ESC op` record
    // (op ∈ {MOD, INS}); None = at a sequence start.
    let mut run: Option<u8> = None;
    while i < patch.len() {
        match run {
            Some(op) => {
                if patch[i] == Op::Esc.byte() {
                    if i + 1 >= patch.len() {
                        break; // trailing lone ESC at end of stream
                    }
                    let nxt = patch[i + 1];
                    if is_opcode(nxt) && nxt != op {
                        i += 2; // new explicit record
                        if nxt == Op::Mod.byte() || nxt == Op::Ins.byte() {
                            run = Some(nxt);
                        } else {
                            run = None;
                            skip_len(patch, &mut i);
                        }
                    } else {
                        i += 2; // ESC ESC / ESC same-op / ESC unknown: data
                    }
                } else {
                    i += 1;
                }
            }
            None => {
                if patch[i] == Op::Esc.byte() && i + 1 < patch.len() && is_opcode(patch[i + 1]) {
                    let nxt = patch[i + 1];
                    i += 2;
                    if nxt == Op::Mod.byte() || nxt == Op::Ins.byte() {
                        run = Some(nxt);
                    } else {
                        skip_len(patch, &mut i);
                    }
                } else if patch[i] == Op::Esc.byte() && i + 1 == patch.len() {
                    break; // trailing lone ESC (JPatcht trailing-byte case)
                } else {
                    // Raw data byte, ESC ESC or ESC <unknown> at a sequence
                    // start: the decoder defaults to MOD — implicit segment.
                    implicit += 1;
                    run = Some(Op::Mod.byte());
                }
            }
        }
    }
    implicit
}

/// Positive control for the walker: the default 0.8.5 patch of the tiny
/// pair (PATCH_AB) rides the implicit MOD exactly once ("byeby" after the
/// EQL record), while the explicit 0.8.1 form has zero implicit segments.
#[test]
fn implicit_mod_walker_classifies_the_reference_patches() {
    assert_eq!(count_implicit_mod_segments(PATCH_AB), 1);
    assert_eq!(count_implicit_mod_segments(PATCH_AB_081), 0);
}

/// §22.1 gate for the port-only `--compat-081` (§21.16): every patch
/// produced over the corpus (both synthetic pairs and both bundled fixture
/// pairs × a few option sets) contains **zero** implicit-MOD segments
/// (structural decoder walk) and round-trips through `jdiff -u`.
#[test]
fn compat_081_patches_have_zero_implicit_mod_segments() {
    let corpus: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        ("tiny", ORG_A.to_vec(), NEW_B.to_vec()),
        ("big", big_pair().0, big_pair().1),
        (
            "test2",
            fs::read(fixture("test2.001.txt")).expect("read test2.001"),
            fs::read(fixture("test2.002.txt")).expect("read test2.002"),
        ),
        (
            "bkocomu",
            fs::read(fixture("bkocomu.0000.fil")).expect("read bkocomu.0000"),
            fs::read(fixture("bkocomu.0009.fil")).expect("read bkocomu.0009"),
        ),
    ];
    let option_sets: [&[&str]; 5] = [&[], &["-b"], &["-f"], &["-p", "-q"], &["-x", "5"]];

    let dir = temp_dir("compat-struct");
    for (name, org, new) in &corpus {
        let pdir = dir.join(name);
        fs::create_dir_all(&pdir).expect("create corpus dir");
        let a = write_file(&pdir.join("a.bin"), org);
        let b = write_file(&pdir.join("b.bin"), new);

        for opts in option_sets {
            let tag: String = opts.join("").replace('-', "");
            let patch = pdir.join(format!("p{tag}.jdf"));
            let mut jargs: Vec<&OsStr> = vec![OsStr::new("--compat-081")];
            jargs.extend(opts.iter().map(OsStr::new));
            jargs.extend([a.as_os_str(), b.as_os_str(), patch.as_os_str()]);
            let drun = run(&jargs);
            assert_eq!(
                drun.status.code(),
                Some(1),
                "--compat-081 {opts:?} diff on {name}: {}",
                stderr_str(&drun)
            );
            assert!(
                stderr_str(&drun).is_empty(),
                "--compat-081 {opts:?} diff on {name} must be silent"
            );

            let patch_bytes = fs::read(&patch).expect("read patch");
            let segments = count_implicit_mod_segments(&patch_bytes);
            assert_eq!(
                segments, 0,
                "--compat-081 {opts:?} patch on {name} must have zero implicit-MOD segments"
            );

            let out = pdir.join("out.bin");
            let mut uargs: Vec<&OsStr> = vec![OsStr::new("-u")];
            uargs.extend([a.as_os_str(), patch.as_os_str(), out.as_os_str()]);
            let urun = run(&uargs);
            assert_eq!(
                urun.status.code(),
                Some(0),
                "--compat-081 {opts:?} apply on {name}: {}",
                stderr_str(&urun)
            );
            assert!(
                stderr_str(&urun).is_empty(),
                "--compat-081 {opts:?} apply on {name} must be silent"
            );
            assert_eq!(
                fs::read(&out).unwrap(),
                *new,
                "{opts:?} round trip on {name}"
            );
        }
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// `--compat-081` on the diff side (§21.16): the tiny pair's patch comes out
/// in the explicit 0.8.1 format — byte-identical to PATCH_AB_081, since the
/// engine decisions stay 0.8.5 and this pair's records only differ in the
/// opcode prefix — and both `jdiff -u` and an argv[0]=`jptch` copy restore
/// it exactly.
#[test]
fn compat_081_diff_produces_explicit_patch() {
    let dir = temp_dir("compat-cli");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);
    let patch = dir.join("p.jdf");

    let out = run(&[
        OsStr::new("--compat-081"),
        a.as_os_str(),
        b.as_os_str(),
        patch.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    assert!(stderr_str(&out).is_empty());
    assert!(out.stdout.is_empty());
    assert_eq!(
        fs::read(&patch).unwrap(),
        PATCH_AB_081,
        "--compat-081 patch must be the explicit 0.8.1 format"
    );

    // `jdiff -u` restores.
    let o1 = dir.join("out1.bin");
    let urun = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        patch.as_os_str(),
        o1.as_os_str(),
    ]);
    assert_eq!(urun.status.code(), Some(0), "{}", stderr_str(&urun));
    assert!(stderr_str(&urun).is_empty());
    assert_eq!(fs::read(&o1).unwrap(), NEW_B);

    // An argv[0]=`jptch` copy restores too (spec §21.2 dispatch).
    let jptch = copy_binary_as(&dir, "jptch");
    let o2 = dir.join("out2.bin");
    let prun = run_copied(&jptch, &[a.as_os_str(), patch.as_os_str(), o2.as_os_str()]);
    assert_eq!(prun.status.code(), Some(0), "{}", stderr_str(&prun));
    assert!(stderr_str(&prun).is_empty());
    assert_eq!(fs::read(&o2).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// GNU permutation (`--compat-081` after the filenames) and the patch side's
/// accept-and-ignore (`-u --compat-081`, and a `jpatch`-named argv[0] copy):
/// the flag only selects the diff-side writer, so patches still apply.
#[test]
fn compat_081_permutation_and_patch_side_ignored() {
    let dir = temp_dir("compat-perm");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    // Flag after the filenames: identical explicit patch.
    let p = dir.join("p.jdf");
    let out = run(&[
        a.as_os_str(),
        b.as_os_str(),
        p.as_os_str(),
        OsStr::new("--compat-081"),
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    assert!(stderr_str(&out).is_empty());
    assert_eq!(fs::read(&p).unwrap(), PATCH_AB_081);

    // Patch side accepts and ignores: `jdiff -u --compat-081`.
    let o1 = dir.join("out1.bin");
    let urun = run(&[
        OsStr::new("--compat-081"),
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        o1.as_os_str(),
    ]);
    assert_eq!(urun.status.code(), Some(0), "{}", stderr_str(&urun));
    assert!(stderr_str(&urun).is_empty());
    assert_eq!(fs::read(&o1).unwrap(), NEW_B);

    // … and under an argv[0]=`jpatch` copy.
    let jpatch = copy_binary_as(&dir, "jpatch");
    let o2 = dir.join("out2.bin");
    let prun = run_copied(
        &jpatch,
        &[
            OsStr::new("--compat-081"),
            a.as_os_str(),
            p.as_os_str(),
            o2.as_os_str(),
        ],
    );
    assert_eq!(prun.status.code(), Some(0), "{}", stderr_str(&prun));
    assert!(stderr_str(&prun).is_empty());
    assert_eq!(fs::read(&o2).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Patch mode (`jdiff -u`) surface — replaces the removed jptch CLI
// ===========================================================================

/// `-` may stand for any of the three file arguments in patch mode:
/// original on stdin, patch on stdin, output on stdout.
#[test]
fn u_dash_variants() {
    let dir = temp_dir("u-dash");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let out = dir.join("out.bin");

    // Original from (seekable, file-redirected) stdin: `jdiff -u - p out`.
    let r = run_stdin_file(
        &[
            OsStr::new("-u"),
            OsStr::new("-"),
            p.as_os_str(),
            out.as_os_str(),
        ],
        &a,
    );
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B, "org from stdin");
    fs::remove_file(&out).unwrap();

    // Patch from stdin: `jdiff -u a - out`.
    let r = run_stdin_file(
        &[
            OsStr::new("-u"),
            a.as_os_str(),
            OsStr::new("-"),
            out.as_os_str(),
        ],
        &p,
    );
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B, "patch from stdin");
    fs::remove_file(&out).unwrap();

    // Output to stdout: `jdiff -u a p -`.
    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        OsStr::new("-"),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(r.stdout, NEW_B, "patch to stdout");
    assert!(r.stderr.is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Both inputs `-` is rejected in patch mode too (the check precedes the
/// function dispatch, `main.cpp:612-615`).
#[test]
fn u_both_dash_exit_2() {
    let out = run(&[OsStr::new("-u"), OsStr::new("-"), OsStr::new("-")]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr_str(&out),
        "Error: Original and destination files cannot both be from standard input !\n"
    );
}

/// `-u -v` verbose trace: greeting, "Use -h" hint, then the op summary and
/// EOF lines — byte-exact with the oracle capture (949 bytes; the JDIFF
/// banner replaces the 0.8.1 jptch's JPATCH banner). The positions print
/// with P8zd (`pw`).
#[test]
fn u_verbose_lines_match_reference() {
    let dir = temp_dir("u-vrb");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let out = dir.join("out.bin");

    let expected = format!(
        "{}\nUse -h for additional help and usage description.\n\
         {} {} EQL 12\n\
         {} {} MOD 5\n\
         {} {} INS 1\n\
         {} {} EOF\n",
        GREETING,
        pw(0),
        pw(0),
        pw(12),
        pw(12),
        pw(17),
        pw(17),
        pw(17),
        pw(18),
    );

    let r = run(&[
        OsStr::new("-u"),
        OsStr::new("-v"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(stderr_str(&r), expected);
    assert_eq!(fs::read(&out).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// The 0.8.1-format patch (explicit `ESC MOD`) applies identically — the
/// explicit opcodes are a subset of the 0.8.5 grammar (spec §18.C).
#[test]
fn u_applies_081_format_patches() {
    let dir = temp_dir("u-081");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB_081);
    let out = dir.join("out.bin");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), NEW_B);
    fs::remove_dir_all(&dir).unwrap();
}

/// ESC-escaped data applies exactly through the CLI (`ESC ESC` → one literal
/// ESC in MOD/INS data; a bare trailing ESC yields byte 0xFF from the -1
/// operand, oracle-pinned in the library gates).
#[test]
fn u_esc_escaped_data() {
    let dir = temp_dir("u-esc");
    let org = write_file(&dir.join("org.bin"), b"0123456789");
    let out = dir.join("out.bin");

    let p = write_file(&dir.join("p1.bin"), &[0xA7, 0xA6, b'A', 0xA7, 0xA7, b'B']);
    let r = run(&[
        OsStr::new("-u"),
        org.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'A', 0xA7, b'B']);

    let p = write_file(&dir.join("p2.bin"), &[0xA7, 0xA5, b'A', 0xA7, 0xA7, b'B']);
    let r = run(&[
        OsStr::new("-u"),
        org.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'A', 0xA7, b'B'], "INS escape");

    let p = write_file(&dir.join("p3.bin"), &[0xA7, 0xA6, b'x', 0xA7]);
    let r = run(&[
        OsStr::new("-u"),
        org.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), [b'x', 0xA7, 0xFF], "trailing ESC");
    fs::remove_dir_all(&dir).unwrap();
}

/// Leading/trailing garbage around a complete record is handled by the 0.8.5
/// decoder (bare leading bytes ride the default MOD operator, `JPatcht.cpp:
/// 246-254`): garbage is APPLIED as MOD data and advances the source
/// position, so a following EQL copies from after it (oracle-verified:
/// "XY" + EQL 12 yields "XY" + org[2..14]).
#[test]
fn u_garbage_bytes_are_mod_data() {
    let dir = temp_dir("u-grb");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out = dir.join("out.bin");

    // Leading garbage "XY": default-MOD data before the EQL record; the MOD
    // run advances the source position mirror to 2.
    let p = write_file(&dir.join("p1.bin"), &[b'X', b'Y', 0xA7, 0xA3, 0x0B]);
    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    let mut want = b"XY".to_vec();
    want.extend_from_slice(&ORG_A[2..14]);
    assert_eq!(fs::read(&out).unwrap(), want, "leading garbage");

    // Trailing garbage after a complete EQL likewise rides MOD.
    let p = write_file(&dir.join("p2.bin"), &[0xA7, 0xA3, 0x0B, b'X', b'Y']);
    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    let mut want = ORG_A[..12].to_vec();
    want.extend_from_slice(b"XY");
    assert_eq!(fs::read(&out).unwrap(), want, "trailing garbage");
    fs::remove_dir_all(&dir).unwrap();
}

/// A lone ESC at a sequence start is the trailing-byte corruption warning
/// plus the CLI's "\nError occurred !" line, both on real stderr, with
/// exit 20 (`JPatcht.cpp:241-245` warning, `main.cpp:916-918` exit path;
/// oracle-verified byte-exact).
#[test]
fn u_trailing_byte_warning_exit_20() {
    let dir = temp_dir("u-trail");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), &[0xA7]);
    let out = dir.join("out.bin");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(20));
    assert_eq!(
        stderr_str(&r),
        "Warning: unexpected trailing byte at end of file, patch file may be corrupted.\n\nError occurred !\n"
    );
    assert!(fs::read(&out).unwrap().is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// A DEL-only patch never touches the source: exit 0, empty output
/// (JPatcht 0.8.5 defers the position change to the next record — the 0.8.1
/// jpatch.cpp "seek to -5" error is gone).
#[test]
fn u_del_only_patch_is_silent() {
    let dir = temp_dir("u-del");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), &[0xA7, 0xA4, 0xFC]); // DEL 252
    let out = dir.join("out.bin");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert!(fs::read(&out).unwrap().is_empty(), "nothing copied");
    assert!(r.stderr.is_empty());

    // -vv shows the DEL line and the org position advanced by the seek.
    let r = run(&[
        OsStr::new("-u"),
        OsStr::new("-vv"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    let text = stderr_str(&r);
    assert!(
        text.contains(&format!("{} {} DEL 252\n", pw(0), pw(0))),
        "{text:?}"
    );
    assert!(
        text.contains(&format!("{} {} EOF\n", pw(252), pw(0))),
        "{text:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Length reads at patch EOF follow the C arithmetic on getc-or-EOF (-1)
/// with no error path: `ESC DEL 255×8` reads (0xFF…FF << 8) + (-1) = -257,
/// which JPatcht returns as a negative offset — the CLI reports the unknown
/// code path "\nUnknown exit code -257\n" and exits 20 (oracle-pinned
/// arithmetic; the exit-code mapping is main.cpp:929-931).
#[test]
fn u_negative_length_unknown_exit_code() {
    let dir = temp_dir("u-neglen");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(
        &dir.join("p.bin"),
        &[0xA7, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    );
    let out = dir.join("out.bin");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(20), "{}", stderr_str(&r));
    assert_eq!(stderr_str(&r), "\nUnknown exit code -257\n");
    assert!(fs::read(&out).unwrap().is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

/// Checked I/O error paths of the EQL copy (`JFileOut.cpp:33-75`): a short
/// read prints "Error reading source file." on real stderr plus the CLI's
/// "\nError reading file !\n" and exits 8; a failing large write to
/// /dev/full prints "Error writing output file." and exits 9; a small write
/// is absorbed by the stdio-style buffer (silent exit 0).
#[test]
fn u_read_write_error_exits() {
    let dir = temp_dir("u-ioerr");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let out = dir.join("out.bin");

    // EQL 100 from a 17-byte file: the fast path writes the 17 available
    // bytes, the re-issued getbuf hits EOF → exit 8 (oracle-verified: the
    // partial 17-byte output stays on disk).
    let p = write_file(&dir.join("p.eql100"), &[0xA7, 0xA3, 0x63]);
    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(8), "{}", stderr_str(&r));
    assert_eq!(
        stderr_str(&r),
        "Error reading source file.\n\nError reading file !\n"
    );
    assert_eq!(fs::read(&out).unwrap(), ORG_A, "partial copy is kept");

    // Small EQL (100 bytes, fully readable) to /dev/full: the stdio-style
    // buffer absorbs the write, silent exit 0 (oracle-pinned).
    #[cfg(target_os = "linux")]
    {
        let big = write_file(&dir.join("big200.bin"), &[0u8; 200]);
        let r = run(&[
            OsStr::new("-u"),
            big.as_os_str(),
            p.as_os_str(),
            OsStr::new("/dev/full"),
        ]);
        assert_eq!(
            r.status.code(),
            Some(0),
            "buffered small write must stay silent: {:?}",
            stderr_str(&r)
        );
        assert!(r.stderr.is_empty(), "no message expected");

        // Large EQL (65536 = 16 blocks) to /dev/full: the checked write
        // eventually fails → "Error writing output file.", exit 9.
        let big = write_file(&dir.join("big64k.bin"), &[0u8; 65536]);
        let p9 = write_file(
            &dir.join("p.eql64k.bin"),
            &[0xA7, 0xA3, 0xFE, 0x00, 0x01, 0x00, 0x00],
        );
        let r = run(&[
            OsStr::new("-u"),
            big.as_os_str(),
            p9.as_os_str(),
            OsStr::new("/dev/full"),
        ]);
        assert_eq!(r.status.code(), Some(9), "{}", stderr_str(&r));
        assert!(stderr_str(&r).contains("Error writing output file.\n"));
    }

    fs::remove_dir_all(&dir).unwrap();
}

/// Patch-mode open errors reuse the diff CLI's messages and exits (one
/// binary, `main.cpp:744-773`): missing original → 3, missing patch → 4,
/// unopenable output → 5. (The 0.8.1 jpatch.cpp texts "Could not open data/
/// patch file" are gone.)
#[test]
fn u_open_error_exits_3_4_5() {
    let dir = temp_dir("u-open");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), PATCH_AB);
    let out = dir.join("out.bin");
    let missing = dir.join("does-not-exist");

    let r = run(&[
        OsStr::new("-u"),
        missing.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(3));
    assert_eq!(
        stderr_str(&r),
        format!(
            "Could not open first file {} for reading.\n",
            missing.display()
        )
    );
    assert!(!out.exists(), "output must not be created");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        missing.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(4));
    assert_eq!(
        stderr_str(&r),
        format!(
            "Could not open second file {} for reading.\n",
            missing.display()
        )
    );

    let bad_out = dir.join("no-such-dir").join("out.bin");
    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        bad_out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(5));
    assert_eq!(
        stderr_str(&r),
        format!(
            "Could not open output file {} for writing.\n",
            bad_out.display()
        )
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// A patch of an identical pair applies to the org exactly (exit 0 — patch
/// success always exits 0).
#[test]
fn u_eql_only_patch_restores() {
    let dir = temp_dir("u-eql");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let p = write_file(&dir.join("p.bin"), &[0xA7, 0xA3, 0x10]); // EQL 17
    let out = dir.join("out.bin");

    let r = run(&[
        OsStr::new("-u"),
        a.as_os_str(),
        p.as_os_str(),
        out.as_os_str(),
    ]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr_str(&r));
    assert_eq!(fs::read(&out).unwrap(), ORG_A);
    fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Write-error resolution on the diff path (carried from Task 9): the C++
// diff CLI never checks putc — /dev/full yields a silently truncated patch
// and the per-stats exit code.
// ===========================================================================

/// `jdiff a b /dev/full` exits per the swapped stats mapping (dta > 0 →
/// DIF → 1; oracle-verified silent exit 1 on this differing pair; JOutBin
/// never checks putc).
#[cfg(target_os = "linux")]
#[test]
fn write_error_dev_full_matches_oracle() {
    let dir = temp_dir("devfull");
    let a = write_file(&dir.join("a.bin"), ORG_A);
    let b = write_file(&dir.join("b.bin"), NEW_B);

    let out = run(&[a.as_os_str(), b.as_os_str(), OsStr::new("/dev/full")]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "oracle: silent exit 1 on write errors (differences found)"
    );
    assert!(
        out.stderr.is_empty(),
        "no EXI_WRI message: {:?}",
        stderr_str(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Engine verbose pin carried from Task 17, re-pinned to the 0.8.5 oracle
/// (byte-identical `-vvv` stderr on this pair, one miss): the 0.8.5 engine
/// finds exactly one inaccurate solution at 1922269/1854717 with the 8 MB
/// table (`-i 8` — port and oracle build the same table since the §21.18
/// divisor ruling; re-verified byte-identical against `target/oracle/jdiff
/// -vvv -i 8`). The stats counter itself stays 1 in the port (the oracle
/// prints uninitialized heap garbage there — C++ never zeroes `miHshErr`;
/// port deviation per §21.5/§21.6 determinism stance).
#[test]
fn inaccurate_solution_lines_at_verbose_3() {
    let dir = temp_dir("t17-inacc");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let a = fixtures.join("bkocomu.0000.fil");
    let b = fixtures.join("bkocomu.0009.fil");
    let outp = dir.join("p.bin");

    let out = run(&[
        OsStr::new("-vvv"),
        OsStr::new("-i"),
        OsStr::new("8"),
        a.as_os_str(),
        b.as_os_str(),
        outp.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr_str(&out));
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("\nInaccurate solution at positions 1922269/1854717!\n"),
        "the single miss line: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("Inaccurate solution at positions").count(),
        1,
        "exactly one miss on this pair (oracle-pinned)"
    );
    // The miss prints the "Comparing : ...           " restart marker
    // directly after the line (miss line ends "!\n").
    assert_eq!(
        stderr.matches("!\nComparing : ...           ").count(),
        1,
        "restart marker after the miss"
    );
    // The verbose>2 buildFullIndex distribution (JDiff.cpp:784-787,
    // `dist(pos, 10)`), byte-identical to the oracle (first/last bucket, the
    // summary and load lines). The positions print in the P8zd width
    // (`pw`), so the bucket lines are built per feature set.
    assert!(stderr.contains("Hash Dist Overload    = 2\n"), "{stderr:?}");
    assert!(
        stderr.contains(&format!(
            "Hash Dist        0 Pos={}:{} Cnt=   43642 Rlb=4\n",
            pw(0),
            pw(192358)
        )),
        "{stderr:?}"
    );
    assert!(
        stderr.contains(&format!(
            "Hash Dist        9 Pos={}:{} Cnt=   47964 Rlb=4\n",
            pw(1731222),
            pw(1923580)
        )),
        "{stderr:?}"
    );
    assert!(
        stderr.contains("Hash Dist Avg/Min/Max/% = 49830/39955/75432/48%\n"),
        "{stderr:?}"
    );
    assert!(
        stderr.contains("Hash Dist Load          = 498306/699037=71%\n"),
        "{stderr:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}
