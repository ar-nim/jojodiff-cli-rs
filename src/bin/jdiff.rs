//! `jdiff` CLI — byte-compatible port of JojoDiff 0.8.1's `src/main.cpp`
//! (spec §4): option parsing, greeting/help, file handling, engine wiring,
//! statistics output and exit codes.
//!
//! All printed strings are verbatim from `main.cpp`, including the
//! "Hastable" typo, "File adressing is …" (sic) and the unpadded `PRIzd`
//! statistics values (the `P8zd` width applies only to the engine's debug
//! prints). Greeting/usage/stats go to the [`jojodiff_cli_rs::jdebug`]
//! stream (`JDebug::stddbg`: stderr by default, stdout with `-do`).
//!
//! # Deviations from the stock C++ binary (spec-documented or oracle-pinned;
//! see the Task 9 report for the full discussion)
//!
//! 1. **Linux `fillBuffer` bug not ported (spec §15.1).** The stock binary
//!    pre-reads both inputs on pthreads using a garbage `stat` size when an
//!    input is missing (aborting with `bad_alloc`, exit 134, before any
//!    open-error check) and leaves the ifstreams at EOF otherwise. This port
//!    opens the files directly and checks the opens in `main.cpp` order,
//!    which matches the spec's fixed-oracle semantics: missing inputs produce
//!    `Could not open first/second file … for reading.` with exit 3/4 — the
//!    behavior of the ported `main.cpp:482-485,504-507` code itself.
//! 2. **`-m 0` NUL-truncation fixed (spec §15.3).** The in-memory reader
//!    serves the whole file; the C++ `istringstream(char*)` truncates at the
//!    first NUL byte.
//! 3. **Write errors are swallowed like the C++ `putc`s.** No diff-path
//!    component in the C++ ever produces `-EXI_WRI` (`JOut*` never check
//!    `putc`; only `jpatch.cpp` exits `EXI_WRI`), so the `case -EXI_WRI`
//!    switch in `main.cpp:599-601` is dead code, and a failing output sink
//!    yields a silently truncated patch with the normal exit code
//!    (oracle: `jdiff A B /dev/full` → exit 0, no message). The CLI therefore
//!    wraps its sink in [`IgnoringWriter`], which drops every I/O error,
//!    keeping the library writers' own error panics unreachable from the CLI
//!    while direct library users keep the more informative behavior. The six
//!    `EXI_*` error arms of the switch are ported 1:1 (the `-EXI_SEK` arm is
//!    reachable through [`JFileAhead`]'s seek-error sentinel; the other five
//!    are dead-code parity with the release build).
//! 4. **`-do` + stdout patch ordering.** With `-do`, a stdout patch and the
//!    verbose stream share Rust's global stdout buffer (like the C++ single
//!    `FILE*`), so verbose/statistics lines and patch bytes keep write order;
//!    without `-do` the patch is `BufWriter`-buffered and flushed at scope
//!    end, mirroring the C++ exit-time flush.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::process::exit;

use jojodiff_cli_rs::defs::{
    EXI_ARG, EXI_ERR, EXI_FRT, EXI_LRG, EXI_MEM, EXI_OUT, EXI_RED, EXI_SCD, EXI_SEK, EXI_WRI,
    JDIFF_COPYRIGHT, JDIFF_VERSION, MAX_OFF_T, MCH_MAX, SMPSZE, c_atoi,
};
use jojodiff_cli_rs::jdebug::{DBG_TO_STDOUT, dbg_print};
use jojodiff_cli_rs::jdiff::JDiff;
use jojodiff_cli_rs::jfile::{JFile, JFileAhead, JFileMem};
use jojodiff_cli_rs::jout::{JOut, JOutAsc, JOutBin, JOutRgn};

fn main() {
    exit(real_main());
}

/// Default settings (`main.cpp:190-200`).
struct Options {
    /// 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn (`liOutTyp`).
    out_typ: i32,
    /// Verbose level 0=no, 1=normal, 2=high (`liVerbse`).
    verbose: i32,
    /// Backtrace on sourcefile allowed? (`lbSrcBkt`).
    src_bkt: bool,
    /// Compare even if data not in buffer? (`lbCmpAll`).
    cmp_all: bool,
    /// Prescan source file: 0=no, 1=do, 2=done (`liSrcScn`).
    src_scn: i32,
    /// Maximum entries in matching table (`liMchMax`).
    mch_max: i32,
    /// Minimum entries in matching table (`liMchMin`).
    mch_min: i32,
    /// Hashtable size in mega-samples (`liHshMbt`).
    hsh_mbt: i32,
    /// File-buffer size (`llBufSze`); 0 = in-memory mode.
    buf_sze: i64,
    /// Block size (`liBlkSze`).
    blk_sze: i32,
    /// Lookahead range, 0 = same as `buf_sze` (`liAhdMax`).
    ahd_max: i32,
    /// Help requested (`lcHlp == 'h'`).
    help: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            out_typ: 0,
            verbose: 0,
            src_bkt: true,
            cmp_all: true,
            src_scn: 1,
            mch_max: 32,
            mch_min: 8,
            hsh_mbt: 8,
            buf_sze: 256 * 1024,
            blk_sze: 4096,
            ahd_max: 0,
            help: false,
        }
    }
}

fn real_main() -> i32 {
    /* Read options (`main.cpp:205-315`): options must precede the filenames;
     * the first non-option token ends parsing and is re-queued as the first
     * filename. `li_opt_arg_cnt` mirrors the C++ `liOptArgCnt` index into the
     * argument vector (which includes argv[0]). */
    let args: Vec<OsString> = std::env::args_os().collect();
    let arg_cnt = args.len(); /* aiArgCnt */
    let mut li_opt_arg_cnt: usize = 0;
    let mut lb_opt_arg_dne = false;
    let mut o = Options::default();

    while !lb_opt_arg_dne && arg_cnt > li_opt_arg_cnt + 1 {
        li_opt_arg_cnt += 1;
        let tok = args[li_opt_arg_cnt].as_os_str();
        if is(tok, "-v") {
            o.verbose = 1;
        } else if is(tok, "-vv") {
            o.verbose = 2;
        } else if is(tok, "-vvv") {
            o.verbose = 3;
        } else if is(tok, "-h") {
            o.help = true;
        } else if is(tok, "-a") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                /* Integer division FIRST, then *1024 (C++ `int` arithmetic;
                 * wrapping because `atoi` saturation can overflow the C++
                 * `int`, which is UB there and a defined wrap here). */
                o.ahd_max = (c_atoi(&args[li_opt_arg_cnt]) / 2).wrapping_mul(1024);
            }
        } else if is(tok, "-m") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                o.buf_sze = i64::from((c_atoi(&args[li_opt_arg_cnt]) / 2).wrapping_mul(1024));
            }
        } else if is(tok, "-bs") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                o.blk_sze = c_atoi(&args[li_opt_arg_cnt]);
            }
        } else if is(tok, "-s") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                o.hsh_mbt = c_atoi(&args[li_opt_arg_cnt]);
                while o.hsh_mbt > 1024 {
                    o.hsh_mbt /= 1024;
                }
            }
        } else if is(tok, "-min") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                o.mch_min = c_atoi(&args[li_opt_arg_cnt]);
                if o.mch_min > MCH_MAX {
                    o.mch_min = MCH_MAX;
                }
            }
        } else if is(tok, "-max") {
            li_opt_arg_cnt += 1;
            if arg_cnt > li_opt_arg_cnt {
                o.mch_max = c_atoi(&args[li_opt_arg_cnt]);
                if o.mch_max > MCH_MAX {
                    o.mch_max = MCH_MAX;
                }
            }
        } else if is(tok, "-l") {
            o.out_typ = 1;
        } else if is(tok, "-lr") {
            o.out_typ = 2;
        } else if is(tok, "-b") {
            // Larger hashtables
            o.cmp_all = true;
            o.buf_sze = 4096 * 1024;
            o.src_bkt = true;
            o.src_scn = 1;
            o.mch_min = 16;
            o.mch_max = 128;
            o.hsh_mbt = 32; // 32meg elements
        } else if is(tok, "-f") {
            // No compare out-of-buffer
            o.cmp_all = false;
            o.buf_sze = 64 * 1024;
            o.src_bkt = true;
            o.src_scn = 1;
            o.mch_min = 8;
            o.mch_max = 16;
            o.hsh_mbt = 4; // 4Meg samples
        } else if is(tok, "-ff") {
            // No compare out-of-buffer and no backtracing
            o.cmp_all = false;
            o.buf_sze = 4096 * 1024;
            o.src_bkt = true;
            o.src_scn = 0;
            o.mch_min = 4;
            o.mch_max = 16;
            o.hsh_mbt = 1; // 1Meg samples
        } else if is(tok, "-do") {
            DBG_TO_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
        } else if is_dbg_flag(tok) {
            /* Consumed by `is_dbg_flag`, which set the flag (debug builds).
             * In default builds `is_dbg_flag` is always false and the token
             * falls through to the filename branch below, exactly like the
             * release C++ build whose `#if debug` strcmp arms are compiled
             * out (`main.cpp:286-309`). */
        } else {
            lb_opt_arg_dne = true;
            li_opt_arg_cnt -= 1;
        }
    }

    /* Output greetings (`main.cpp:317-344`) */
    let nargs = arg_cnt - li_opt_arg_cnt;
    if o.verbose > 0 || o.help || nargs < 3 {
        print_greeting();
    }

    /* Usage / exit on missing args or help (`main.cpp:346-387`). The 0.8.5
     * `EXI_*` codes are negative; `exit(-EXI_*)` yields the positive process
     * exit code (here 2), like the C++. */
    if nargs < 3 || o.help || o.verbose > 2 {
        print_usage(o.mch_min, o.mch_max);
        if nargs < 3 || o.help {
            exit(-EXI_ARG);
        }
    }

    /* Read filenames (`main.cpp:389-395`); the indexes are in range because
     * the exit above guarantees nargs >= 3. */
    let nam_org = args[1 + li_opt_arg_cnt].clone();
    let nam_new = args[2 + li_opt_arg_cnt].clone();
    let nam_out: OsString = if nargs >= 4 {
        args[3 + li_opt_arg_cnt].clone()
    } else {
        OsString::from("-")
    };

    /* Open first file (`main.cpp:482-485`) */
    let file_org = match File::open(&nam_org) {
        Ok(file) => file,
        Err(_) => {
            dbg_print(format_args!(
                "Could not open first file {} for reading.\n",
                nam_org.to_string_lossy()
            ));
            exit(-EXI_FRT);
        }
    };

    /* Open second file (`main.cpp:504-507`) */
    let file_new = match File::open(&nam_new) {
        Ok(file) => file,
        Err(_) => {
            dbg_print(format_args!(
                "Could not open second file {} for reading.\n",
                nam_new.to_string_lossy()
            ));
            exit(-EXI_SCD);
        }
    };

    /* Open output (`main.cpp:510-517`): "-" means stdout. */
    let out_is_stdout = nam_out == *"-";
    let sink = if out_is_stdout {
        Sink::Stdout(std::io::stdout().lock())
    } else {
        match File::create(&nam_out) {
            Ok(file) => Sink::File(file),
            Err(_) => {
                dbg_print(format_args!(
                    "Could not open output file {} for writing.\n",
                    nam_out.to_string_lossy()
                ));
                exit(-EXI_OUT);
            }
        }
    };

    /* Init output (`main.cpp:519-532`): the writers sit on a sink that
     * swallows I/O errors like the C++ `putc`s (module docs, deviation 3).
     * With `-do` and a stdout patch the raw lock is used so patch bytes and
     * verbose lines share one ordered buffer, like the C++ single `FILE*`;
     * otherwise a `BufWriter` batches the per-byte writes and flushes at
     * scope end, like the C++ exit-time flush. */
    let writer: Box<dyn Write> =
        if out_is_stdout && DBG_TO_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
            Box::new(IgnoringWriter { inner: sink })
        } else {
            Box::new(BufWriter::new(IgnoringWriter { inner: sink }))
        };
    let jout: Box<dyn JOut> = match o.out_typ {
        0 => Box::new(JOutBin::new(writer)),
        1 => Box::new(JOutAsc::new(writer)),
        _ => Box::new(JOutRgn::new(writer)),
    };

    /* Build the file readers (`main.cpp:465-503`): buffered look-ahead with
     * `buf_sze`/`blk_sze`, or the whole file in memory for `-m 0` (spec
     * §15.3: the C++ NUL truncation is not ported; see `read_whole` for the
     * read-error divergence). */
    let (reader_org, reader_new): (Box<dyn JFile>, Box<dyn JFile>) = if o.buf_sze > 0 {
        (
            Box::new(JFileAhead::new(file_org, "Org", o.buf_sze, o.blk_sze)),
            Box::new(JFileAhead::new(file_new, "New", o.buf_sze, o.blk_sze)),
        )
    } else {
        let data_org = read_whole(file_org);
        let data_new = read_whole(file_new);
        (
            Box::new(JFileMem::new(data_org)),
            Box::new(JFileMem::new(data_new)),
        )
    };

    /* Go … (`main.cpp:534-541`) */
    let mut jd = JDiff::new(
        reader_org,
        reader_new,
        jout,
        // Hashtable size in MB (0.8.5 `main.cpp:819` passes liHshMbt, the
        // option-parsed MB count, straight through; `JDiff.cpp:87` ->
        // `JHashPos::new` converts MB to elements). The -s/-i parsing and
        // the 0.8.5 default of 32 MB are Task 20's.
        o.hsh_mbt,
        o.verbose,
        o.src_bkt,
        o.src_scn != 0,
        o.mch_max,
        o.mch_min,
        if o.ahd_max == 0 {
            o.buf_sze
        } else {
            i64::from(o.ahd_max)
        },
        o.cmp_all,
    );
    if o.verbose > 1 {
        dbg_print(format_args!(
            "Lookahead buffers: {} kb. ({} kb. per file).\n",
            (o.buf_sze.wrapping_mul(2) / 1024) as u64,
            (o.buf_sze / 1024) as u64
        ));
        let hsh = jd.hash();
        dbg_print(format_args!(
            "Hastable size    : {} kb. ({} samples).\n",
            (i64::from(hsh.hash_size_bytes()) + 512) / 1024,
            hsh.hash_prime()
        ));
    }

    let ret = jd.jdiff();

    /* Write statistics (`main.cpp:545-567`). The `%d`/`%ld`/`PRIzd` values
     * are printed unpadded — `P8zd`'s width is only used by the engine's
     * debug prints. "Random accesses" sums both readers' seek counts. */
    let stats = jd.out_stats();
    if o.verbose > 1 {
        let hsh = jd.hash();
        let hashsize = i64::from(hsh.hash_size_bytes());
        let hashsize_kb = (hashsize + 512) / 1024;
        dbg_print(format_args!(
            "Hashtable size          = {hashsize} samples, {hashsize_kb} KB, {} MB\n",
            (hashsize_kb + 512) / 1024
        ));
        dbg_print(format_args!(
            "Hashtable prime         = {}\n",
            hsh.hash_prime()
        ));
        dbg_print(format_args!(
            "Hashtable hits          = {}\n",
            hsh.hash_hits()
        ));
        dbg_print(format_args!("Hashtable errors        = {}\n", jd.hsh_err()));
        // 0.8.5: instance counter via getHshRpr (JMatchTable.cpp:930-932);
        // the 0.8.1 global static is retired (spec §18.E).
        dbg_print(format_args!(
            "Hashtable repairs       = {}\n",
            jd.hsh_rpr()
        ));
        dbg_print(format_args!(
            "Hashtable overloading   = {}\n",
            hsh.hash_colmax() / 3 - 1
        ));
        dbg_print(format_args!(
            "Reliability distance    = {}\n",
            hsh.reliability()
        ));
        dbg_print(format_args!(
            "Random    accesses      = {}\n",
            jd.org_seekcount() + jd.new_seekcount()
        ));
        dbg_print(format_args!("Delete    bytes         = {}\n", stats.del));
        dbg_print(format_args!("Backtrack bytes         = {}\n", stats.bkt));
        dbg_print(format_args!("Escape    bytes written = {}\n", stats.esc));
        dbg_print(format_args!("Control   bytes written = {}\n", stats.ctl));
    }
    if o.verbose > 0 {
        dbg_print(format_args!("Equal     bytes         = {}\n", stats.eql));
        dbg_print(format_args!("Data      bytes written = {}\n", stats.dta));
        dbg_print(format_args!(
            "Overhead  bytes written = {}\n",
            stats.ctl + stats.esc
        ));
    }

    /* Exit (`main.cpp:588-613`): engine error codes print their message
     * (WITHOUT trailing newline, like the C++ fprintf) and exit; otherwise
     * 1 = no differences found, 0 = differences found. The 0.8.5 engine
     * returns the raw negative `EXI_*` codes (`JFileAhead.h:115`), matched
     * here by guard (a negated-const pattern would parse as a binding); the
     * arms produce `-EXI_*`, the positive process exit code, exactly the
     * C++ `case EXI_SEK: exit(-EXI_SEK)` mapping (main.cpp:894-928). */
    let code = match ret {
        r if r == EXI_SEK => {
            dbg_print(format_args!("Seek error !"));
            -EXI_SEK
        }
        r if r == EXI_LRG => {
            dbg_print(format_args!("64-bit offsets not supported !"));
            -EXI_LRG
        }
        r if r == EXI_RED => {
            dbg_print(format_args!("Error reading file !"));
            -EXI_RED
        }
        r if r == EXI_WRI => {
            dbg_print(format_args!("Error writing file !"));
            -EXI_WRI
        }
        r if r == EXI_MEM => {
            dbg_print(format_args!("Error allocating memory !"));
            -EXI_MEM
        }
        r if r == EXI_ERR => {
            dbg_print(format_args!("Spurious error occured !"));
            -EXI_ERR
        }
        _ if stats.dta == 0 && stats.del == 0 => 1, /* no differences found */
        _ => 0,                                     /* differences found */
    };

    /* Exit-time flush, like the C runtime: the patch writer is flushed by its
     * own Drop when `jd`/`jout` are dropped on return; the debug stream's
     * shared stdout buffer is flushed here. */
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    code
}

/// Byte-equality of an option token with a C string (`strcmp == 0`).
fn is(tok: &OsStr, opt: &str) -> bool {
    tok == OsStr::new(opt)
}

/// The 11 `-d*` debug flags with their `gbDbg` indices, in `main.cpp:286-308`
/// order (`DBGHSH..DBGDST`, `JDebug.h:37-47`). Debug builds only.
#[cfg(feature = "debug")]
const DBG_FLAGS: &[(&str, usize)] = &[
    ("-dhsh", jojodiff_cli_rs::jdebug::DBGHSH),
    ("-dahd", jojodiff_cli_rs::jdebug::DBGAHD),
    ("-dcmp", jojodiff_cli_rs::jdebug::DBGCMP),
    ("-dprg", jojodiff_cli_rs::jdebug::DBGPRG),
    ("-dbuf", jojodiff_cli_rs::jdebug::DBGBUF),
    ("-dhsk", jojodiff_cli_rs::jdebug::DBGHSK),
    ("-dahh", jojodiff_cli_rs::jdebug::DBGAHH),
    ("-dbkt", jojodiff_cli_rs::jdebug::DBGBKT),
    ("-dred", jojodiff_cli_rs::jdebug::DBGRED),
    ("-dmch", jojodiff_cli_rs::jdebug::DBGMCH),
    ("-ddst", jojodiff_cli_rs::jdebug::DBGDST),
];

/// Recognizes (and consumes) the `-dhsh`…`-ddst` debug flags (`main.cpp:286-309`).
/// Debug builds only: without the feature this always answers false, so the
/// tokens fall through to the filename branch like in the release C++ build.
fn is_dbg_flag(tok: &OsStr) -> bool {
    #[cfg(feature = "debug")]
    {
        DBG_FLAGS.iter().any(|&(name, idx)| {
            if is(tok, name) {
                jojodiff_cli_rs::jdebug::dbg_set(idx, true);
                true
            } else {
                false
            }
        })
    }
    #[cfg(not(feature = "debug"))]
    {
        let _ = tok;
        false
    }
}

/// Reads a whole input file for the `-m 0` in-memory reader. A read failure
/// after a successful open maps to the `Error reading file !` exit; the C++
/// ignores such errors (its pthread pre-read discards them), which is
/// unreachable for regular files — see module docs, deviation 3.
fn read_whole(mut file: File) -> Vec<u8> {
    let mut data = Vec::new();
    if file.read_to_end(&mut data).is_err() {
        dbg_print(format_args!("Error reading file !"));
        exit(-EXI_RED);
    }
    data
}

/// Greeting block (`main.cpp:317-344`), written line by line like the C++
/// `fprintf` calls.
fn print_greeting() {
    dbg_print(format_args!(
        "JDIFF - Jojo's binary diff version {JDIFF_VERSION}\n"
    ));
    dbg_print(format_args!("{JDIFF_COPYRIGHT}\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JojoDiff is free software: you can redistribute it and/or modify\n"
    ));
    dbg_print(format_args!(
        "it under the terms of the GNU General Public License as published by\n"
    ));
    dbg_print(format_args!(
        "the Free Software Foundation, either version 3 of the License, or\n"
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
        "MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\n"
    ));
    dbg_print(format_args!(
        "GNU General Public License for more details.\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "You should have received a copy of the GNU General Public License\n"
    ));
    dbg_print(format_args!(
        "along with this program.  If not, see <http://www.gnu.org/licenses/>.\n"
    ));
    dbg_print(format_args!("\n"));

    /* `main.cpp:336-343`: ((MAX_OFF_T >> 30) + 1) GB, shifted to TB when
     * above 1024; "%d bit" is sizeof(off_t) * 8 with off_t = i64. */
    let mut maxoff_t_gb = (MAX_OFF_T >> 30) + 1;
    let mut maxoff_t_mul = "GB";
    if maxoff_t_gb > 1024 {
        maxoff_t_gb >>= 10;
        maxoff_t_mul = "TB";
    }
    dbg_print(format_args!(
        "File adressing is {} bit (files up to {maxoff_t_gb} {maxoff_t_mul}), samples are {SMPSZE} bytes.\n\n",
        std::mem::size_of::<i64>() as i32 * 8
    ));
}

/// Usage/help block (`main.cpp:346-383`); the `-min`/`-max` lines print the
/// current (post-parse) values and `MCH_MAX`.
fn print_usage(mch_min: i32, mch_max: i32) {
    dbg_print(format_args!(
        "Usage: jdiff [options] <original file> <new file> [<output file>]\n"
    ));
    dbg_print(format_args!(
        "  -v          Verbose (greeting, results and tips).\n"
    ));
    dbg_print(format_args!("  -vv         Verbose (debug info).\n"));
    dbg_print(format_args!("  -h          Help (this text).\n"));
    dbg_print(format_args!("  -l          Listing (ascii output).\n"));
    dbg_print(format_args!("  -lr         Regions (ascii output).\n"));
    dbg_print(format_args!(
        "  -do         Write verbose and debug info to stdout instead of stddbg.\n"
    ));
    dbg_print(format_args!(
        "  -b          Try to be better (using more memory).\n"
    ));
    dbg_print(format_args!(
        "  -f          Try to be faster: no out of buffer compares.\n"
    ));
    dbg_print(format_args!(
        "  -ff         Try to be faster: no out of buffer compares, nor pre-scanning.\n"
    ));
    dbg_print(format_args!(
        "  -m size     Size (in kB) for look-ahead buffer (default 512kB, 0=no buffers).\n"
    ));
    dbg_print(format_args!(
        "  -bs size    Block size (in bytes) for reading from files (default 4096).\n"
    ));
    dbg_print(format_args!(
        "  -s size     Number of samples per file in MB (default 8).\n"
    ));
    dbg_print(format_args!(
        "  -a size     Number of kB to look ahead (default=same as buffer-size).\n"
    ));
    dbg_print(format_args!(
        "  -min count  Minimum number of solutions to find (default {mch_min}, max {MCH_MAX}).\n"
    ));
    dbg_print(format_args!(
        "  -max count  Maximum number of solutions to find (default {mch_max}, max {MCH_MAX}).\n"
    ));
    dbg_print(format_args!("Principles:\n"));
    dbg_print(format_args!(
        "  JDIFF tries to find equal regions between two binary files using a heuristic\n"
    ));
    dbg_print(format_args!(
        "  hash algorithm and outputs the differences between both files.\n"
    ));
    dbg_print(format_args!(
        "  Heuristics are generally used for improving performance and memory usage,\n"
    ));
    dbg_print(format_args!(
        "  at the cost of accuracy. Therefore, this program may not find a minimal set\n"
    ));
    dbg_print(format_args!("  of differences between files.\n"));
    dbg_print(format_args!("Notes:\n"));
    dbg_print(format_args!(
        "  Options -b, -f or -ff should be used before other options.\n"
    ));
    dbg_print(format_args!(
        "  Accuracy may be improved by increasing the number of samples.\n"
    ));
    dbg_print(format_args!(
        "  Sample size is always lowered to the largest n-bit prime (n < 32)\n"
    ));
    dbg_print(format_args!(
        "  Original and new file must be random access files.\n"
    ));
    dbg_print(format_args!(
        "  Output is sent to standard output if output file is missing.\n"
    ));
    dbg_print(format_args!("Hint:\n"));
    dbg_print(format_args!(
        "  Do not use jdiff directly on compressed files, such as zip, gzip, rar, ...\n"
    ));
    dbg_print(format_args!(
        "  Instead use uncompressed files, such as tar, cpio or zip-0, and then compress\n"
    ));
    dbg_print(format_args!("  the jdiff's output file afterwards.\n"));
    dbg_print(format_args!("\n"));
}

/// Output sink: a real file or locked stdout (`main.cpp:510-517`).
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
/// deviation 3: `EXI_WRI` is unreachable in the diff CLI and write failures
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
