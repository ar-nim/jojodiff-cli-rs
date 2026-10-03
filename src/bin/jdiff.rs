//! `jdiff` CLI — 1:1 port of the 0.8.5 `main.cpp` (spec §18.A/§18.D/§18.F):
//! the `getopt_long` option surface (`src/cli/opts.rs`), argv[0] dispatch,
//! greeting/help, file handling (stdin via `-`, sequential auto-detection),
//! engine wiring, pre-run echo + post-run statistics and the swapped exit
//! codes (differences → 1, identical → 0).
//!
//! One binary (spec §21.2): the basename of argv[0] starting `jpatch` routes
//! to Patch (upstream `main.cpp:303-315`, case-insensitive) and — as a port
//! extension — so does a basename starting `jptch`, making a symlink/copy
//! named `jptch` behave exactly like `jdiff -u`. The `jdedup`/`jtst` argv[0]
//! routes are NOT ported (spec §21.3/§21.4): such names fall through to the
//! default Diff function. Function options: `-j` Diff, `-u` Patch, `-t` Test
//! (ported faithfully although broken upstream, §21.3), `-y` Dedup (accepted,
//! exits 20, §21.4).
//!
//! All printed strings are verbatim from `main.cpp` — including the stale
//! texts (spec §21.10): the usage claims `-i` "(default 64)" (actual 32) and
//! `-k` "(default 8192)" (actual 32768), calls `-m` sizes "(in KB)" (actual
//! MB) and offers "0=no buffering" (no such mode); the verbose echo says
//! `(-s)` for the index size, `(-b)` for the block size and carries the
//! "disbale" typo.
//!
//! # Deviations from the stock C++ binary (spec-documented)
//!
//! 1. **stdin inputs** re-open `/dev/stdin` so the `chkSeq` seek-EOF probe
//!    sees exactly what the C++ `FILE*`/`cin` sees on this platform: a
//!    redirected regular file is seekable (no warning, random access), a
//!    pipe fails the probe and runs sequential (auto `-p`/`-q` with the
//!    §18.A warnings). If `/dev/stdin` cannot be opened the input falls back
//!    to the un-seekable global stdin (sequential semantics).
//! 2. **`-t` Test mode re-opens the inputs for its patch phase** (the diff
//!    phase owns the readers). The C++ reuses its file handles mid-cursor,
//!    which is precisely the upstream bug (§21.3): the appended garbage may
//!    start at a different offset than the oracle's (release oracle appends
//!    one stale 0x00 first), while the shape — diff output followed by
//!    misparsed destination data, exit per the patch phase — matches.
//!    Upstream's debug build trips the `JFileAhead::getbuf` assert on the
//!    handoff's -1 cursor (`getbuf(New,-1,1,0) … failed !`, exit 6 —
//!    verified against the debug oracle); the port never creates that state
//!    and the negative-EOF gate (jfile/ahead.rs deviation 4) ends the patch
//!    phase cleanly, so its debug `-t` behaves like the release one.
//! 3. **Write errors are swallowed like the C++ `putc`s on the diff path.**
//!    No diff-path component checks `putc` (`JOut*` never check), so the CLI
//!    wraps the diff sink in [`IgnoringWriter`], keeping a failing output
//!    sink silent (oracle: `jdiff A B /dev/full` → exit 0, no message). The
//!    patch path (`JFileOut`) DOES check its writes and exits 9, like the
//!    C++ (oracle: `-u <64k file> <EQL 64k patch> /dev/full` → exit 9).
//! 4. **`-c` + stdout patch ordering**: with `-c`, a stdout patch and the
//!    verbose stream share Rust's global stdout buffer (like the C++ single
//!    `FILE*`), so verbose lines and patch bytes keep write order; without
//!    `-c` the patch is `BufWriter`-buffered and flushed at scope end,
//!    mirroring the C++ exit-time flush.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::process::exit;

use jojodiff_cli_rs::cli::config::{self, Function};
use jojodiff_cli_rs::cli::error::report;
use jojodiff_cli_rs::cli::report::{print_greeting, print_notes, print_usage};
use jojodiff_cli_rs::defs::{EXI_DIF, EXI_EQL, EXI_OK};
use jojodiff_cli_rs::error::JDiffError;
use jojodiff_cli_rs::jdebug::{DBG_TO_STDOUT, dbg_print};
use jojodiff_cli_rs::jdiff::JDiff;
use jojodiff_cli_rs::jfile::{JFile, JFileAhead};
use jojodiff_cli_rs::jfileout::JFileOut;
use jojodiff_cli_rs::jout::{JOut, JOutAsc, JOutBin, JOutRgn};
use jojodiff_cli_rs::jpatcht::JPatcht;

fn main() {
    exit(real_main());
}

fn real_main() -> i32 {
    let args: Vec<OsString> = std::env::args_os().collect();
    let ai_arg_cnt = args.len(); /* aiArgCnt */

    /* Parse option-switches (`main.cpp:276-476`): argv[0] dispatch, default
     * settings and the getopt_long loop with GNU permutation moved to
     * `cli::config::parse` (unit-tested there); `?` (unknown option or
     * argument error) sets liHlp=1 and parsing CONTINUES. */
    let opts = config::parse(&args);
    let li_fun = opts.fun;
    let out_typ = opts.out_typ; /* 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn */
    let verbose = opts.verbose;
    let mut src_bkt = opts.src_bkt; /* Backtrace on sourcefile allowed? */
    let mut cmp_all = opts.cmp_all; /* Compare even if data not in buffer? */
    let mut src_scn = opts.src_scn; /* Prescan source file: 0=no, 1=do, 2=done */
    let mch_max = opts.mch_max; /* Maximum entries in matching table. */
    let mut mch_min = opts.mch_min; /* Minimum entries in matching table. */
    let hsh_mbt = opts.hsh_mbt; /* Hashtable size in MB (* 1024 * 1024) */
    let compat_081 = opts.compat_081; /* --compat-081 (§21.16) */
    let mut seq_org = opts.seq_org; /* Sequential source file? */
    let mut seq_new = opts.seq_new; /* Sequential destination file? */
    let li_hlp = opts.li_hlp; /* -h/--help flag: 0=no, 1=-h, 2=-hh, 3=error */

    /* Output greetings (`main.cpp:480-509`). */
    let nargs = ai_arg_cnt - opts.opt_arg_cnt;
    if verbose > 0 || li_hlp > 0 || nargs < 3 {
        print_greeting();
    }

    /* Usage / exit on missing args or help (`main.cpp:511-602`). */
    if nargs < 3 || li_hlp > 0 || verbose > 2 {
        print_usage(mch_min, mch_max);
        if li_hlp > 1 || verbose > 2 {
            print_notes();
        }
        if nargs < 3 {
            if li_hlp == 0 {
                dbg_print(format_args!(
                    "Error: Not enough arguments have been specified !\n"
                ));
            }
            /* The boundary prints nothing for `Args`: this site's text is
             * `liHlp`-gated (pinned by `missing_args_exit_2` and
             * `help_exit_code_and_text`), so only the code crosses. */
            return report(Err(JDiffError::Args), verbose);
        }
    } else if verbose > 0 {
        dbg_print(format_args!(
            "\nUse -h for additional help and usage description.\n"
        ));
    }

    /* Read filenames (`main.cpp:604-610`); the operand indexes are in range
     * because the exit above guarantees nargs >= 3. */
    let operands = &opts.operands;
    let nam_org = operands[0].clone();
    let nam_new = operands[1].clone();
    let nam_out: OsString = if operands.len() >= 3 {
        operands[2].clone()
    } else {
        OsString::from("-")
    };

    if nam_new == *"-" && nam_org == *"-" {
        dbg_print(format_args!(
            "{}",
            "Error: Original and destination files cannot both be from standard input !\n"
        ));
        /* Like above: this site's own text is pinned
         * (`both_inputs_dash_exit_2`), the boundary carries only the code. */
        return report(Err(JDiffError::Args), verbose);
    }

    // Set default values for llBlk and liBlk (`main.cpp:617-645`): the
    // computation — including the two misalignment warnings and the ahd_max
    // default — moved to `cli::config::size_buffers` (unit-tested there).
    let config::Buffers {
        ll_buf_org,
        ll_buf_new,
        blk_sze,
        ahd_max,
    } = config::size_buffers(&opts);

    /* Open files and create file handlers (`main.cpp:647-752`). The inputs
     * may be opened a second time for `-t`'s patch phase (deviation 2). */
    let inputs = match open_inputs(&nam_org, &nam_new, ll_buf_org, ll_buf_new, blk_sze) {
        Ok(inputs) => inputs,
        Err(e) => return report(Err(e), verbose),
    };

    /* Open output (`main.cpp:754-774`); Dedup does not open one (its crash
     * path is replaced by the EXI_ERR exit, §21.4). */
    let out_is_stdout = nam_out == *"-";
    let file_out: Option<Sink> = if li_fun == Function::Dedup {
        None
    } else if out_is_stdout {
        Some(Sink::Stdout(std::io::stdout().lock()))
    } else {
        match open_output_file(&nam_out, false) {
            Ok(file) => Some(Sink::File(file)),
            Err(e) => return report(Err(e), verbose),
        }
    };

    /* Execute required function (`main.cpp:776-876`). */
    let mut li_ret: Result<i32, JDiffError> = Err(JDiffError::Args); /* default return code */

    if li_fun == Function::Dedup {
        // Dedup is not ported (§21.4): the files opened, then the C++ crash
        // path becomes the EXI_ERR exit ("Error occurred !").
        li_ret = Err(JDiffError::NotPorted);
    }

    if li_fun == Function::Diff || li_fun == Function::Test {
        /* Perform JDiff */
        // Switch to sequential source file (`main.cpp:780-788`)
        if !seq_org && inputs.org.is_sequential() {
            seq_org = true;
            cmp_all = false; // only compare data within the buffer
            src_bkt = false; // only backtrack on source file in buffer
            src_scn = 0; // no pre-scan indexing

            dbg_print(format_args!(
                "\n{}\n",
                "Warning: Source file is a sequential file, assuming -p."
            ));
        }

        // Switch to sequential destination file (`main.cpp:790-795`)
        if !seq_new && inputs.new.is_sequential() {
            seq_new = true;
            mch_min = 0; // only search within the buffer
            dbg_print(format_args!(
                "\n{}\n",
                "Warning: Destination file is a sequential file, assuming -q."
            ));
        }

        /* Init output (`main.cpp:797-815`): the writers sit on a sink that
         * swallows I/O errors like the C++ `putc`s (module docs, deviation
         * 3). With `-c` and a stdout patch the raw lock is used so patch
         * bytes and verbose lines share one ordered buffer, like the C++
         * single `FILE*`; otherwise a `BufWriter` batches the per-byte
         * writes and flushes at scope end, like the C++ exit-time flush. */
        let out_sink = file_out.expect("output open above guarantees a sink");
        let writer: Box<dyn Write> =
            wrap_buffered(IgnoringWriter { inner: out_sink }, out_is_stdout);
        let jout: Box<dyn JOut> = match out_typ {
            1 => Box::new(JOutAsc::new(writer)),
            // --compat-081 (§21.16) selects the byte-exact 0.8.1 writer
            // policy; the default keeps the 0.8.5 implicit-MOD format.
            0 => Box::new(JOutBin::with_compat_081(writer, compat_081)),
            _ => Box::new(JOutRgn::new(writer)),
        };

        /* Initialize JDiff object (`main.cpp:817-820`). */
        let mut lo_jdiff = JDiff::new(
            inputs.org,
            inputs.new,
            jout,
            hsh_mbt,
            verbose,
            src_bkt,
            src_scn != 0,
            mch_max,
            mch_min,
            i64::from(ahd_max),
            cmp_all,
        );

        /* Show execution parameters (`main.cpp:822-836`), verbatim including
         * the stale `(-s)`/`(-b)` letters and the "disbale" typo. */
        if verbose > 1 {
            let hashsize = i64::from(lo_jdiff.hash().hash_size_bytes());
            dbg_print(format_args!("\n"));
            dbg_print(format_args!(
                "Index table size (default: 64Mb) (-s): {}Mb ({} samples)\n",
                ((hashsize + 512) / 1024 + 512) / 1024,
                lo_jdiff.hash().hash_prime()
            ));
            dbg_print(format_args!(
                "Search size     (0 = buffersize) (-a): {}kb\n",
                ahd_max / 1024
            ));
            dbg_print(format_args!(
                "Buffer size       (default  2Mb) (-m): {}Mb\n",
                (ll_buf_org + ll_buf_new) / 1024 / 1024
            ));
            dbg_print(format_args!(
                "Block  size       (default 32kb) (-b): {}kb\n",
                blk_sze / 1024
            ));
            dbg_print(format_args!(
                "Min number of matches to search  (-n): {}\n",
                mch_min
            ));
            dbg_print(format_args!(
                "Max number of matches to search  (-x): {}\n",
                mch_max
            ));
            dbg_print(format_args!(
                "Compare out-of-buffer (-f to disable): {}\n",
                if cmp_all { "yes" } else { "no" }
            ));
            dbg_print(format_args!(
                "Full indexing scan   (-ff to disbale): {}\n",
                if src_scn > 0 { "yes" } else { "no" }
            ));
            dbg_print(format_args!(
                "Backtrace allowed     (-p to disable): {}\n",
                if src_bkt { "yes" } else { "no" }
            ));
        }

        /* Execute... (`main.cpp:838-845`): the 0.8.5 exit swap — identical
         * files yield EXI_EQL (process exit 0), differences EXI_DIF (exit
         * 1), decided by `out.dta > 0`. Engine errors flow to `report` as
         * `Err(JDiffError)`; the error TEXTS are not printed here — the
         * boundary prints the pinned family once (and the library-side
         * `JFileOut` family prints at its own failure point), so a Display
         * print here would double them. */
        let engine = lo_jdiff.jdiff();
        let stats = lo_jdiff.out_stats();
        li_ret = match engine {
            Ok(()) if stats.dta > 0 => Ok(EXI_DIF),
            Ok(()) => Ok(EXI_EQL),
            Err(e) => Err(e),
        };

        /* Write statistics (`main.cpp:847-869`). */
        if verbose > 1 {
            let hsh = lo_jdiff.hash();
            dbg_print(format_args!("\n"));
            dbg_print(format_args!(
                "Index table hits        = {}\n",
                hsh.hash_hits()
            ));
            dbg_print(format_args!(
                "Index table repairs     = {}\n",
                lo_jdiff.hsh_rpr()
            ));
            dbg_print(format_args!(
                "Index table overloading = {}\n",
                hsh.hash_colmax() / 4 - 1
            ));
            dbg_print(format_args!(
                "Reliability distance    = {}\n",
                hsh.reliability()
            ));
            dbg_print(format_args!(
                "Inaccurate  solutions   = {}\n",
                lo_jdiff.hsh_err()
            ));
            dbg_print(format_args!(
                "Source      seeks       = {}\n",
                lo_jdiff.org_seekcount()
            ));
            dbg_print(format_args!(
                "Destination seeks       = {}\n",
                lo_jdiff.new_seekcount()
            ));
            dbg_print(format_args!("Delete      bytes       = {}\n", stats.del));
            dbg_print(format_args!("Backtrack   bytes       = {}\n", stats.bkt));
            dbg_print(format_args!("Escape      bytes       = {}\n", stats.esc));
            dbg_print(format_args!("Control     bytes       = {}\n", stats.ctl));
        }
        if verbose > 0 {
            dbg_print(format_args!("\n"));
            dbg_print(format_args!("Equal       bytes       = {}\n", stats.eql));
            dbg_print(format_args!("Data        bytes       = {}\n", stats.dta));
            dbg_print(format_args!(
                "Control-Esc bytes       = {}\n",
                stats.ctl + stats.esc
            ));
            dbg_print(format_args!(
                "Total       bytes       = {}\n",
                stats.ctl + stats.esc + stats.dta
            ));
        }

        /* The diff-phase writer is dropped here with `lo_jdiff` (C++: the
         * FILE* stays open; the Rust writer must flush before `-t`'s patch
         * phase appends). */
    } /* liFun == Diff or Test */

    // The C++ keeps the post-warning `lbSeqOrg`/`lbSeqNew` locals alive until
    // end of main without reading them; the mirror assignments above are dead
    // for us too — one explicit read keeps the lint quiet without changing
    // the 1:1 shape.
    let _ = (seq_org, seq_new);

    if li_fun == Function::Patch || li_fun == Function::Test {
        // The patch phase reads the source file and — for `-u` the patch,
        // for the faithful-broken `-t` (§21.3) the DESTINATION file — as the
        // patch (`main.cpp:871-876`). The readers are opened fresh here:
        // for `-u` this is their first and only open, for `-t` the C++
        // reuses its mid-cursor handles (the upstream bug — see module docs,
        // deviation 2). `-t`'s output appends to what the diff wrote.
        let mut inputs = match open_inputs(&nam_org, &nam_new, ll_buf_org, ll_buf_new, blk_sze) {
            Ok(inputs) => inputs,
            Err(e) => return report(Err(e), verbose),
        };
        let patch_sink = if out_is_stdout {
            Sink::Stdout(std::io::stdout().lock())
        } else if li_fun == Function::Test {
            // Append to the diff output just flushed (C++: the same FILE*).
            match open_output_file(&nam_out, true) {
                Ok(file) => Sink::File(file),
                Err(e) => return report(Err(e), verbose),
            }
        } else {
            match open_output_file(&nam_out, false) {
                Ok(file) => Sink::File(file),
                Err(e) => return report(Err(e), verbose),
            }
        };

        /* The patch phase wraps its sink in a BufWriter flushed at scope
         * end (like the C++ exit-time flush), but NOT in the diff path's
         * IgnoringWriter: JFileOut checks its writes (copyfrom → EXI_WRI,
         * `JFileOut.cpp:47-50,63-66`) and `-u <big> <p> /dev/full` must
         * exit 9. With `-c` and a stdout patch the raw lock is used so
         * patch bytes and verbose lines share one ordered buffer, like the
         * C++ single `FILE*`. */
        let patch_writer: Box<dyn Write> = wrap_buffered(patch_sink, out_is_stdout);
        let lo_fil_out = JFileOut::new(patch_writer);
        let mut lo_jpatcht = JPatcht::new(
            inputs.org.as_mut(),
            inputs.new.as_mut(),
            lo_fil_out,
            verbose,
        );
        li_ret = lo_jpatcht.jpatch().map(|()| EXI_OK);
        // Flush the patch writer at scope end, like the C++ exit-time flush.
        drop(lo_jpatcht.into_inner());
    } /* liFun == Patch or Test */

    /* Cleanup: the readers and writers drop in the blocks above. */

    /* Exit (`main.cpp:897-932`): the single boundary prints the pinned error
     * text (if any) and yields the positive process exit code. */
    report(li_ret, verbose)
}

/// The two input readers (`lpJflOrg`/`lpJflNew`).
struct Inputs {
    org: Box<dyn JFile>,
    new: Box<dyn JFile>,
}

/// Opens both input files as buffered look-ahead readers
/// (`main.cpp:647-752`): `-` re-opens `/dev/stdin` (deviation 1), anything
/// else opens the named file; failures return `OpenFirst`/`OpenSecond` with
/// the name moved in — the boundary (`cli::error::report`) prints the
/// `main.cpp` messages and exits 3/4.
fn open_inputs(
    nam_org: &OsString,
    nam_new: &OsString,
    buf_org: i64,
    buf_new: i64,
    blk_sze: i32,
) -> Result<Inputs, JDiffError> {
    let org: Box<dyn JFile> = if *nam_org == *"-" {
        Box::new(JFileAhead::new(open_dash(), "Org", buf_org, blk_sze))
    } else {
        match File::open(nam_org) {
            Ok(file) => Box::new(JFileAhead::new(file, "Org", buf_org, blk_sze)),
            Err(source) => {
                return Err(JDiffError::OpenFirst {
                    name: nam_org.clone(),
                    source,
                });
            }
        }
    };

    let new: Box<dyn JFile> = if *nam_new == *"-" {
        Box::new(JFileAhead::new(open_dash(), "New", buf_new, blk_sze))
    } else {
        match File::open(nam_new) {
            Ok(file) => Box::new(JFileAhead::new(file, "New", buf_new, blk_sze)),
            Err(source) => {
                return Err(JDiffError::OpenSecond {
                    name: nam_new.clone(),
                    source,
                });
            }
        }
    };

    Ok(Inputs { org, new })
}

/// Opens the output file like the C++ (`main.cpp:754-774`); on failure
/// returns `OpenOutput` with the name and the `append` mode moved in — the
/// boundary (`cli::error::report`) prints the pinned message and exits
/// `-EXI_OUT` (5). `append` selects the `-t` reopen path (the same FILE*
/// appended after the diff output).
fn open_output_file(nam_out: &OsStr, append: bool) -> Result<File, JDiffError> {
    let attempt = if append {
        File::options().append(true).open(nam_out)
    } else {
        File::create(nam_out)
    };
    attempt.map_err(|source| JDiffError::OpenOutput {
        name: nam_out.to_os_string(),
        append,
        source,
    })
}

/// The diff/patch writer-buffering decision: with `-c` and a stdout patch
/// the raw sink is used so patch bytes and verbose lines share one ordered
/// buffer (the C++ single `FILE*`); otherwise a `BufWriter` batches the
/// per-byte writes and flushes at scope end (the C++ exit-time flush).
fn wrap_buffered<W: Write + 'static>(sink: W, out_is_stdout: bool) -> Box<dyn Write> {
    if out_is_stdout && DBG_TO_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        Box::new(sink)
    } else {
        Box::new(BufWriter::new(sink))
    }
}

/// An input byte source: a real file or stdin (via its `/dev/stdin` re-open;
/// deviation 1). `Seek` failures mark the input sequential in `chkSeq`.
enum Input {
    File(File),
    Stdin(std::io::Stdin),
}

/// Opens a `-` input: `/dev/stdin` re-opened when possible (a redirected
/// regular file then seeks like the C++ `FILE*`; a pipe fails the seek probe
/// and runs sequential), else the global stdin (never seekable).
fn open_dash() -> Input {
    match File::open("/dev/stdin") {
        Ok(file) => Input::File(file),
        Err(_) => Input::Stdin(std::io::stdin()),
    }
}

impl Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Input::File(file) => file.read(buf),
            Input::Stdin(stdin) => stdin.read(buf),
        }
    }
}

impl Seek for Input {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Input::File(file) => file.seek(pos),
            // Stdin has no usable seek here: a failed probe is exactly the
            // chkSeq sequential signal (`JFile.cpp:37-46`).
            Input::Stdin(_) => Err(std::io::Error::other("stdin is not seekable")),
        }
    }
}

/// Output sink: a real file or locked stdout (`main.cpp:754-774`).
enum Sink {
    File(File),
    Stdout(std::io::StdoutLock<'static>),
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Sink::File(file) => file.write(buf),
            Sink::Stdout(lock) => lock.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sink::File(file) => file.flush(),
            Sink::Stdout(lock) => lock.flush(),
        }
    }
}

/// Swallows every write/flush error and reports success, mirroring the C++
/// `putc`/`fprintf` calls whose results are never checked (module docs,
/// deviation 3: `EXI_WRI` is unreachable on the diff path and write failures
/// produce a silently truncated patch with the normal exit code).
struct IgnoringWriter<W: Write> {
    inner: W,
}

impl<W: Write> Write for IgnoringWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.inner.write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.inner.flush();
        Ok(())
    }
}
