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

use jojodiff_cli_rs::cli::opts::{Getopt, Opt, VAL_COMPAT_081};
use jojodiff_cli_rs::defs::{
    EXI_ARG, EXI_DIF, EXI_EQL, EXI_ERR, EXI_FRT, EXI_LRG, EXI_MEM, EXI_OK, EXI_OUT, EXI_RED,
    EXI_SCD, EXI_SEK, EXI_WRI, JDIFF_COPYRIGHT, JDIFF_VERSION, MAX_OFF_T, SMPSZE, c_atoi,
};
use jojodiff_cli_rs::jdebug::{DBG_TO_STDOUT, dbg_print};
#[cfg(feature = "debug")]
use jojodiff_cli_rs::jdebug::{
    DBGAHD, DBGAHH, DBGBKT, DBGBUF, DBGCMP, DBGDST, DBGHSH, DBGHSK, DBGMCH, DBGPRG, DBGRED, dbg_set,
};
use jojodiff_cli_rs::jdiff::JDiff;
use jojodiff_cli_rs::jfile::{JFile, JFileAhead};
use jojodiff_cli_rs::jfileout::JFileOut;
use jojodiff_cli_rs::jout::{JOut, JOutAsc, JOutBin, JOutRgn};
use jojodiff_cli_rs::jpatcht::JPatcht;

/// Function to execute (`enum {Diff, Patch, Dedup, Test} liFun`,
/// `main.cpp:293`). Dedup/Test are ported per rulings §21.4/§21.3.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Function {
    Diff,
    Patch,
    Dedup,
    Test,
}

fn main() {
    exit(real_main());
}

fn real_main() -> i32 {
    let args: Vec<OsString> = std::env::args_os().collect();
    let ai_arg_cnt = args.len(); /* aiArgCnt */

    /* Read the function from argv[0] (`main.cpp:303-315`): the basename
     * after the last '/' or '\', case-insensitively. `jpatch*` → Patch;
     * `jptch*` → Patch is the port extension (spec §21.2). The `jdedup` and
     * `jtst` routes are not ported (§21.3/§21.4) and fall through to Diff. */
    let mut li_fun = Function::Diff;
    {
        let cmd = args
            .first()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_default();
        let base = cmd.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
        if base.starts_with("jpatch") || base.starts_with("jptch") {
            li_fun = Function::Patch;
        }
    }

    /* Default settings (`main.cpp:276-293`). */
    let mut out_typ: i32 = 0; /* 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn */
    let mut verbose: i32 = 0; /* 0=no, 1=normal, 2=high */
    let mut src_bkt: bool = true; /* Backtrace on sourcefile allowed? */
    let mut cmp_all: bool = true; /* Compare even if data not in buffer? */
    let mut src_scn: i32 = 1; /* Prescan source file: 0=no, 1=do, 2=done */
    let mut mch_max: i32 = 128; /* Maximum entries in matching table. */
    let mut mch_min: i32 = 2; /* Minimum entries in matching table. */
    let mut hsh_mbt: i32 = 32; /* Hashtable size in MB (* 1024 * 1024) */
    let mut buf_org: i64 = 0; /* Default source-file buffer in MB */
    let mut buf_new: i64 = 0; /* Default destin-file buffer in MB */
    let mut blk_sze: i32 = 32 * 1024; /* Default block size (in bytes) */
    let mut ahd_max: i32 = 0; /* Lookahead range (0=same as llBufSze) */
    let mut li_hlp: i32 = 0; /* -h/--help flag: 0=no, 1=-h, 2=-hh, 3=error */
    let mut lb_stdio: bool = false; /* use stdio */
    let mut li_tst: i32 = 0; /* test to execute : 0 = normal, 1 etc... see JTest */
    let mut seq_org: bool = false; /* Sequential source file? */
    let mut seq_new: bool = false; /* Sequential destination file? */
    let mut compat_081 = false; /* --compat-081: 0.8.1-format patch output (§21.16) */

    /* Parse option-switches (`main.cpp:318-476`): getopt_long with GNU
     * permutation; `?` (unknown option or argument error) sets liHlp=1 and
     * parsing CONTINUES. */
    let mut opt = Getopt::new(args);
    loop {
        let code = match opt.next_opt() {
            Opt::End => break,
            Opt::Unknown => {
                li_hlp = 1;
                continue;
            }
            Opt::Code(c) => c,
        };
        let optarg = opt.optarg.take();
        match code {
            'b' => {
                // try-harder: increase hashtable size and more searching
                cmp_all = true; // verify all hashtable matches
                src_bkt = true; // allow going back on source file
                src_scn = 1; // create full index on source file
                mch_min = mch_min.wrapping_mul(2); // increase minimum number of matches to search
                mch_max = mch_max.wrapping_mul(4); // increase maximum number of matches to search
                hsh_mbt = hsh_mbt.wrapping_mul(4); // Increase index table size

                // larger buffers (more soft-ahead searching)
                buf_org = (if buf_org <= 0 { 1 } else { buf_org }) * 4;
            }
            'f' => {
                // faster (or rather: lazier)
                if cmp_all {
                    cmp_all = false; // No compares out-of-buffer
                    src_bkt = true;
                    src_scn = 1;
                    mch_min = mch_min.wrapping_mul(2);
                    mch_max /= 2;

                    // increase buffer size to have more lookahead indexing
                    buf_org = (if buf_org <= 0 { 1 } else { buf_org }) * 16;
                } else {
                    // even faster (lazier)
                    src_scn = 0; // No indexing scan
                    mch_min /= 2; // Reduce lookahead
                    mch_max /= 2;
                }
                hsh_mbt /= 2; // Reduce index table by 2
            }
            'p' => {
                // sequential source file
                seq_org = true;
                cmp_all = false; // only compare data within the buffer
                src_bkt = false; // only backtrack on source file in buffer
                src_scn = 0; // no pre-scan indexing
            }
            'q' => {
                // sequential destination file
                seq_new = true;
                mch_min = 0; // only search within the buffer
            }

            'c' => {
                // verbose-stdout
                DBG_TO_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            'h' => {
                // help
                li_hlp += 1;
            }
            'j' => {
                // jdiff
                li_fun = Function::Diff;
            }
            'l' => {
                // "list-details"
                out_typ = 1;
            }
            'r' => {
                // "list-groups"
                out_typ = 2;
            }
            's' => {
                // use stdio: accepted and recorded; nothing observable in the
                // port (one buffered engine over both backends, §21.11)
                lb_stdio = true;
            }
            't' => {
                // test: patch and unpatch in one go (broken upstream, §21.3)
                li_fun = Function::Test;
                li_tst = optarg.as_ref().map_or(0, |a| c_atoi(a)); // test number
            }
            'u' => {
                // unpatch
                li_fun = Function::Patch;
            }
            'v' => {
                // "verbose"
                verbose += 1;
            }
            'y' => {
                // deduplicate (compiled out upstream; §21.4)
                li_fun = Function::Dedup;
                out_typ = 3;
                lb_stdio = true;
                li_tst = optarg.as_ref().map_or(0, |a| c_atoi(a)); // never used
            }

            VAL_COMPAT_081 => {
                // --compat-081 (port-only, §21.16): 0.8.1-format patch
                // output on the diff side; accepted and ignored when
                // patching (and for the `-l`/`-r` listings, which are
                // diagnostic formats no patcher applies).
                compat_081 = true;
            }

            'a' => {
                // search-ahead-size
                ahd_max = match optarg {
                    Some(arg) => c_atoi(&arg).wrapping_mul(1024),
                    None => 0,
                };
            }

            'i' => {
                // index-size
                hsh_mbt = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if hsh_mbt <= 0 {
                    hsh_mbt = 1;
                    dbg_print(format_args!(
                        "Warning: invalid --index-size/-i specified, set to 1.\n"
                    ));
                }
            }
            'k' => {
                // "block-size"
                blk_sze = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if blk_sze <= 0 {
                    blk_sze = 1;
                    dbg_print(format_args!(
                        "Warning: invalid --block-size/-k specified, set to 1.\n"
                    ));
                }
            }
            'm' => {
                // "buffer-size": MB total, split evenly (`main.cpp:422-434`).
                let val = i64::from(optarg.as_ref().map_or(0, |a| c_atoi(a)));
                if buf_new == 0 {
                    // first -m
                    buf_new = val / 2;
                    buf_org = buf_new; // first -m specifies source and destination buffer
                } else if buf_org == buf_new {
                    // second -m
                    buf_org *= 2;
                    buf_new = val;
                } else {
                    // third and subsequent -m: do nothing
                }
            }
            'n' => {
                // "search-min"
                mch_min = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if mch_min < 0 {
                    mch_min = 0;
                }
            }
            'x' => {
                // "search-max": floored 1024; the unclamped value reaches
                // JMatchTable's bucket prime via JDiff's ctor (§21.15)
                mch_max = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if mch_max <= 0 {
                    mch_max = 1024;
                }
            }

            'd' => {
                // debug flag by name (`main.cpp:446-472`); unknown names are
                // silently ignored, like the release C++ build whose strcmp
                // arms are compiled out.
                #[cfg(feature = "debug")]
                if let Some(name) = optarg.as_ref().map(|a| a.to_string_lossy().into_owned()) {
                    let flag = match name.as_str() {
                        "hsh" => Some(DBGHSH),
                        "ahd" => Some(DBGAHD),
                        "cmp" => Some(DBGCMP),
                        "prg" => Some(DBGPRG),
                        "buf" => Some(DBGBUF),
                        "hsk" => Some(DBGHSK),
                        "ahh" => Some(DBGAHH),
                        "bkt" => Some(DBGBKT),
                        "red" => Some(DBGRED),
                        "mch" => Some(DBGMCH),
                        "dst" => Some(DBGDST),
                        _ => None,
                    };
                    if let Some(idx) = flag {
                        dbg_set(idx, true);
                    }
                }
            }
            _ => unreachable!("Getopt only returns codes from the option table"),
        }
    }
    let li_opt_arg_cnt = opt.optind() - 1;
    // The recorded-but-unread knobs (§21.11/§21.3): `-s` picks the file
    // backend in C++ (one engine here); `liTst` is parsed and never used.
    let _ = lb_stdio;
    let _ = li_tst;

    /* Output greetings (`main.cpp:480-509`). */
    let nargs = ai_arg_cnt - li_opt_arg_cnt;
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
            exit(-EXI_ARG);
        }
    } else if verbose > 0 {
        dbg_print(format_args!(
            "\nUse -h for additional help and usage description.\n"
        ));
    }

    /* Read filenames (`main.cpp:604-610`); the operand indexes are in range
     * because the exit above guarantees nargs >= 3. */
    let operands = opt.operands();
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
        exit(-EXI_ARG);
    }

    // Set default values for llBlk and liBlk (`main.cpp:617-621`).
    let mut ll_buf_org: i64 = if buf_org > 0 {
        buf_org
    } else if seq_org {
        32
    } else {
        1
    };
    let mut ll_buf_new: i64 = (if buf_new > 0 {
        buf_new
    } else if seq_new {
        16
    } else {
        ll_buf_org
    }) * 1024
        * 1024;
    ll_buf_org *= 1024 * 1024;
    let blk_sze = if blk_sze < 4096 { 4096 } else { blk_sze };

    // Buffer size cannot be zero and must be aligned on block size
    // Block size  cannot be larger than buffer size (`main.cpp:623-636`)
    if ll_buf_org % i64::from(blk_sze) != 0 {
        ll_buf_org -= ll_buf_org % i64::from(blk_sze);
        if ll_buf_org <= 0 {
            ll_buf_org = i64::from(blk_sze);
        }
        dbg_print(format_args!(
            "Warning: Source buffer size misaligned with block size: set to {}.\n",
            ll_buf_org
        ));
    }
    if ll_buf_new % i64::from(blk_sze) != 0 {
        ll_buf_new -= ll_buf_new % i64::from(blk_sze);
        if ll_buf_new <= 0 {
            ll_buf_new = i64::from(blk_sze);
        }
        dbg_print(format_args!(
            "Warning: Destination buffer size misaligned with block size: set to {}.\n",
            ll_buf_new
        ));
    }

    // Default search ahead window (`main.cpp:638-645`): the C++ narrows the
    // long difference to int (wrapping for absurd -m values, like the C++
    // int conversion).
    if ahd_max == 0 {
        let diff = ll_buf_new - i64::from(blk_sze);
        ahd_max = diff as i32;
        if ahd_max < 4096 {
            ahd_max = 4096;
        }
    }

    /* Open files and create file handlers (`main.cpp:647-752`). The inputs
     * may be opened a second time for `-t`'s patch phase (deviation 2). */
    let inputs = open_inputs(&nam_org, &nam_new, ll_buf_org, ll_buf_new, blk_sze);

    /* Open output (`main.cpp:754-774`); Dedup does not open one (its crash
     * path is replaced by the EXI_ERR exit, §21.4). */
    let out_is_stdout = nam_out == *"-";
    let file_out: Option<Sink> = if li_fun == Function::Dedup {
        None
    } else if out_is_stdout {
        Some(Sink::Stdout(std::io::stdout().lock()))
    } else {
        Some(Sink::File(open_output_file(&nam_out, false)))
    };

    /* Execute required function (`main.cpp:776-876`). */
    let mut li_ret: i32 = EXI_ARG; /* default return code */

    if li_fun == Function::Dedup {
        // Dedup is not ported (§21.4): the files opened, then the C++ crash
        // path becomes the EXI_ERR exit ("Error occurred !").
        li_ret = EXI_ERR;
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
         * 1), decided by `out.dta > 0`. */
        li_ret = lo_jdiff.jdiff();
        let stats = lo_jdiff.out_stats();
        if li_ret == EXI_OK {
            if stats.dta > 0 {
                li_ret = EXI_DIF;
            } else {
                li_ret = EXI_EQL;
            }
        }

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
        let mut inputs = open_inputs(&nam_org, &nam_new, ll_buf_org, ll_buf_new, blk_sze);
        let patch_sink = if out_is_stdout {
            Sink::Stdout(std::io::stdout().lock())
        } else if li_fun == Function::Test {
            // Append to the diff output just flushed (C++: the same FILE*).
            Sink::File(open_output_file(&nam_out, true))
        } else {
            Sink::File(open_output_file(&nam_out, false))
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
        li_ret = lo_jpatcht.jpatch();
        // Flush the patch writer at scope end, like the C++ exit-time flush.
        drop(lo_jpatcht.into_inner());
    } /* liFun == Patch or Test */

    /* Cleanup: the readers and writers drop in the blocks above. */

    /* Exit (`main.cpp:897-932`). */
    exit_switch(li_ret, verbose)
}

/// The exit-code switch (`main.cpp:897-932`): engine errors print their
/// message and exit `-EXI_*` (the positive process code); patch success
/// exits 0; EXI_EQL/EXI_DIF map to the swapped 0/1 with the verbose verdict
/// lines. Every arm diverges, so the `i32` tail is never reached.
fn exit_switch(li_ret: i32, verbose: i32) -> i32 {
    match li_ret {
        r if r == EXI_SEK => {
            dbg_print(format_args!("\nSeek error !\n"));
            exit(-EXI_SEK);
        }
        r if r == EXI_LRG => {
            dbg_print(format_args!("\nError: 64-bit offsets not supported !\n"));
            exit(-EXI_LRG);
        }
        r if r == EXI_RED => {
            dbg_print(format_args!("\nError reading file !\n"));
            exit(-EXI_RED);
        }
        r if r == EXI_WRI => {
            dbg_print(format_args!("\nError writing file !\n"));
            exit(-EXI_WRI);
        }
        r if r == EXI_MEM => {
            dbg_print(format_args!("\nError allocating memory !\n"));
            exit(-EXI_MEM);
        }
        r if r == EXI_ARG => {
            dbg_print(format_args!("\nError in arguments !\n"));
            exit(-EXI_ARG);
        }
        r if r == EXI_ERR => {
            dbg_print(format_args!("\nError occurred !\n"));
            exit(-EXI_ERR);
        }
        EXI_OK => exit(EXI_OK),
        EXI_EQL => {
            if verbose > 1 {
                dbg_print(format_args!("\nFound all data within source file.\n"));
            }
            exit(EXI_OK);
        }
        EXI_DIF => {
            if verbose > 1 {
                dbg_print(format_args!(
                    "\nNot all data has been found in source file.\n"
                ));
            }
            exit(EXI_DIF);
        }
        _ => {
            dbg_print(format_args!("\nUnknown exit code {}\n", li_ret));
            exit(-EXI_ERR);
        }
    }
}

/// The two input readers (`lpJflOrg`/`lpJflNew`).
struct Inputs {
    org: Box<dyn JFile>,
    new: Box<dyn JFile>,
}

/// Opens both input files as buffered look-ahead readers
/// (`main.cpp:647-752`): `-` re-opens `/dev/stdin` (deviation 1), anything
/// else opens the named file; failures print the `main.cpp` messages and
/// exit 3/4.
fn open_inputs(
    nam_org: &OsString,
    nam_new: &OsString,
    buf_org: i64,
    buf_new: i64,
    blk_sze: i32,
) -> Inputs {
    let org: Box<dyn JFile> = if *nam_org == *"-" {
        Box::new(JFileAhead::new(open_dash(), "Org", buf_org, blk_sze))
    } else {
        match File::open(nam_org) {
            Ok(file) => Box::new(JFileAhead::new(file, "Org", buf_org, blk_sze)),
            Err(_) => {
                dbg_print(format_args!(
                    "Could not open first file {} for reading.\n",
                    nam_org.to_string_lossy()
                ));
                exit(-EXI_FRT);
            }
        }
    };

    let new: Box<dyn JFile> = if *nam_new == *"-" {
        Box::new(JFileAhead::new(open_dash(), "New", buf_new, blk_sze))
    } else {
        match File::open(nam_new) {
            Ok(file) => Box::new(JFileAhead::new(file, "New", buf_new, blk_sze)),
            Err(_) => {
                dbg_print(format_args!(
                    "Could not open second file {} for reading.\n",
                    nam_new.to_string_lossy()
                ));
                exit(-EXI_SCD);
            }
        }
    };

    Inputs { org, new }
}

/// Opens the output file like the C++ (`main.cpp:754-774`): on failure
/// prints the pinned message and exits `-EXI_OUT` (5). `append` selects
/// the `-t` reopen path (the same FILE* appended after the diff output).
fn open_output_file(nam_out: &OsStr, append: bool) -> File {
    let attempt = if append {
        File::options().append(true).open(nam_out)
    } else {
        File::create(nam_out)
    };
    match attempt {
        Ok(file) => file,
        Err(_) => {
            dbg_print(format_args!(
                "Could not open output file {} for writing.\n",
                nam_out.to_string_lossy()
            ));
            exit(-EXI_OUT);
        }
    }
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

/// Greeting block (`main.cpp:480-509`), written line by line like the C++
/// `fprintf` calls — including the 0.8.5 GPL wording and the "adressing"
/// (sic) line computed from `MAX_OFF_T` (no 0.8.1 `+1`).
fn print_greeting() {
    dbg_print(format_args!(
        "\nJDIFF - binary diff version {JDIFF_VERSION}\n"
    ));
    dbg_print(format_args!("{JDIFF_COPYRIGHT}\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JojoDiff is free software: you can redistribute it and/or modify it\n"
    ));
    dbg_print(format_args!(
        "under the terms of the  GNU General Public License  as published by\n"
    ));
    dbg_print(format_args!(
        "the Free Software Foundation,  either version 3 of the License,  or\n"
    ));
    dbg_print(format_args!("(at your option) any later version.\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "This program is distributed in the hope that it will be useful,\n"
    ));
    dbg_print(format_args!(
        "but WITHOUT ANY WARRANTY; without even the implied warranty of\n"
    ));
    dbg_print(format_args!(
        "MERCHANTABILITY  or  FITNESS FOR A PARTICULAR PURPOSE. See the\n"
    ));
    dbg_print(format_args!(
        "GNU General Public License for more details.\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "You should have received a copy of the GNU General Public License\n"
    ));
    dbg_print(format_args!(
        "along with this program. If not, see www.gnu.org/licenses/gpl-3.0\n\n"
    ));

    /* `main.cpp:497-508`: MAX_OFF_T >> 30 GB, shifted to TB when above 1024;
     * "%d bit" is sizeof(off_t) * 8 with off_t = i64. */
    let mut maxoff_t_gb = MAX_OFF_T >> 30;
    let mut maxoff_t_mul = "GB";
    if maxoff_t_gb > 1024 {
        maxoff_t_gb >>= 10;
        maxoff_t_mul = "TB";
    }
    dbg_print(format_args!(
        "File adressing is {} bit for files up to {maxoff_t_gb}{maxoff_t_mul}, samples are {SMPSZE} bytes.\n",
        std::mem::size_of::<i64>() as i32 * 8
    ));
}

/// Usage/help block (`main.cpp:511-557`); the `-n`/`-x` lines print the
/// current (post-parse) match limits, and the stale texts are replicated
/// verbatim (spec §21.10).
fn print_usage(mch_min: i32, mch_max: i32) {
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JDiff differentiates two files so that the second file can be recreated from\n"
    ));
    dbg_print(format_args!(
        "the first by \"undiffing\". JDiff aims for the smallest possible diff file.\n\n"
    ));
    dbg_print(format_args!(
        "Usage: jdiff -j [options] <source file> <destination file> [<diff file>]\n"
    ));
    dbg_print(format_args!(
        "   or: jdiff -u [options] <source file> <diff file> [<destination file>]\n\n"
    ));
    dbg_print(format_args!(
        "  -j                       JDiff:  create a difference file.\n"
    ));
    dbg_print(format_args!(
        "  -u                       Undiff: undiff a difference file.\n\n"
    ));

    dbg_print(format_args!(
        "  -v --verbose             Verbose: greeting, results and tips.\n"
    ));
    dbg_print(format_args!(
        "  -vv                      Extra Verbose: progress info and statistics.\n"
    ));
    dbg_print(format_args!(
        "  -vvv                     Ultra Verbose: all info, including help and details.\n"
    ));
    dbg_print(format_args!(
        "  -h --help -hh            Help, additional help (-hh) and exit.\n"
    ));
    dbg_print(format_args!(
        "  -l --listing             Detailed human readable output.\n"
    ));
    dbg_print(format_args!(
        "  -r --regions             Grouped  human readable output.\n"
    ));
    dbg_print(format_args!(
        "  -c --console             Write verbose and debug info to stdout.\n\n"
    ));

    dbg_print(format_args!(
        "  -b --better -bb...       Better: use more memory, search more.\n"
    ));
    dbg_print(format_args!(
        "  -bb                      Best:   even more memory, search more.\n"
    ));
    dbg_print(format_args!(
        "  -f --lazy                Lazy:   no unbuffered searching (often slower).\n"
    ));
    dbg_print(format_args!(
        "  -ff                      Lazier: no full index table.\n"
    ));
    dbg_print(format_args!(
        "  -p --sequential-source   Sequential source (to avoid !) (with - for stdin).\n"
    ));
    dbg_print(format_args!(
        "  -q --sequential-dest     Sequential destination (with - for stdin).\n"
    ));
    dbg_print(format_args!(
        "  -s --stdio               Use stdio files (for testing).\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "  -a --search-size <size>  Size (in KB) to search (default=buffer-size).\n"
    ));
    dbg_print(format_args!(
        "  -i --index-size  <size>  Size (in MB) for index table    (default 64).\n"
    ));
    dbg_print(format_args!(
        "  -k --block-size  <size>  Block size in bytes for reading (default 8192).\n"
    ));
    dbg_print(format_args!(
        "  -m --buffer-size <size>  Size (in KB) for search buffers (0=no buffering)\n"
    ));
    dbg_print(format_args!(
        "  -n --search-min <count>  Minimum number of matches to search (default {mch_min}).\n"
    ));
    dbg_print(format_args!(
        "  -x --search-max <count>  Maximum number of matches to search (default {mch_max}).\n\n"
    ));

    dbg_print(format_args!(
        "Make  diff-file: jdiff -j old-file new-file diff-file.jdf\n"
    ));
    dbg_print(format_args!(
        "Apply diff-file: jdiff -u old-file diff-file.jdf recreated-new-file\n\n"
    ));

    dbg_print(format_args!("Hint:\n"));
    dbg_print(format_args!(
        "  Do not use jdiff on compressed files. Rather use jdiff first and compress\n"
    ));
    dbg_print(format_args!(
        "  afterwards, e.g.: jdiff -j old new | gzip >dif.jdf.gz (or 7z with -si)\n"
    ));
}

/// The `-hh` notes block (`main.cpp:559-593`, printed when liHlp > 1 or
/// verbose > 2), verbatim including the two-space "blank" lines.
fn print_notes() {
    dbg_print(format_args!("\nNotes:\n"));
    dbg_print(format_args!(
        " - Options -b, -bb, -f, -ff, ... should be used before other options.\n"
    ));
    dbg_print(format_args!(
        " - Accuracy may be improved by increasing the index table size (-i) or\n"
    ));
    dbg_print(format_args!("   the buffer size (-m), see below.\n"));
    dbg_print(format_args!(
        " - The index table size is always lowered to the nearest lower prime number.\n"
    ));
    dbg_print(format_args!(
        " - Output is sent to standard output if no output file is specified.\n"
    ));
    dbg_print(format_args!("\nAdditional explications:\n"));
    dbg_print(format_args!(
        "  JDiff starts by comparing source and destination files.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  When a difference is found, JDiff will first index the source file.\n"
    ));
    dbg_print(format_args!(
        "  Normally, the full source file is indexed, but this can be disabled by the\n"
    ));
    dbg_print(format_args!(
        "  -ff or -p options, in which case only the buffered part of the source file\n"
    ));
    dbg_print(format_args!(
        "  will be indexed. This may be faster, but at a loss of accuracy.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  Using the index, JDiff will search for equal regions between both files.\n"
    ));
    dbg_print(format_args!(
        "  The index table however has two problems:\n"
    ));
    dbg_print(format_args!(
        "  - too small, because a full index would require too much memory.\n"
    ));
    dbg_print(format_args!(
        "  - inaccurate, because the hash-keys are only 32 or 64 bit check-sums.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!("  The inaccuracy is reduced by either:\n"));
    dbg_print(format_args!(
        "  - comparing the found matches from the index, which is slower but certain\n"
    ));
    dbg_print(format_args!(
        "  - confirmation from subsequent matches, which is faster but uncertain\n"
    ));
    dbg_print(format_args!(
        "  Inaccuracy of course can also be reduced with a bigger index table (-i option)\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  Also, the first found solution is not always the best solution.\n"
    ));
    dbg_print(format_args!(
        "  Therefore, JDiff searches a minimum (-n) number of solutions, and\n"
    ));
    dbg_print(format_args!(
        "  will continue up to a maximum (-x) number of solutions if data is buffered.\n"
    ));
    dbg_print(format_args!(
        "  That's why, bigger buffers (-m) can improve accuracy.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  The -b/-bb options increase the index table, buffers and solutions to search.\n"
    ));
    dbg_print(format_args!(
        "  The -f/-ff options will only compare buffered data to gain some speed, but\n"
    ));
    dbg_print(format_args!(
        "  will often be slower due to the lower accuracy.\n"
    ));
}
