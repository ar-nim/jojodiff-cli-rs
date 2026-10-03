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
//!    wraps the diff sink in `IgnoringWriter` (`cli::run`), keeping a
//!    failing output sink silent (oracle: `jdiff A B /dev/full` → exit 0, no
//!    message). The patch path (`JFileOut`) DOES check its writes and exits
//!    9, like the C++ (oracle: `-u <64k file> <EQL 64k patch> /dev/full` →
//!    exit 9).
//! 4. **`-c` + stdout patch ordering**: with `-c`, a stdout patch and the
//!    verbose stream share Rust's global stdout buffer (like the C++ single
//!    `FILE*`), so verbose lines and patch bytes keep write order; without
//!    `-c` the patch is `BufWriter`-buffered and flushed at scope end,
//!    mirroring the C++ exit-time flush.

use std::ffi::OsString;
use std::process::exit;

use jojodiff_cli_rs::cli::config::{self, Function};
use jojodiff_cli_rs::cli::diff_phase::diff_phase;
use jojodiff_cli_rs::cli::error::report;
use jojodiff_cli_rs::cli::patch_phase::patch_phase;
use jojodiff_cli_rs::cli::report::{print_greeting, print_notes, print_usage};
use jojodiff_cli_rs::cli::run::{Sink, open_inputs, open_output_file};
use jojodiff_cli_rs::error::JDiffError;
use jojodiff_cli_rs::jdebug::dbg_print;

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
    let mut opts = config::parse(&args);
    let li_fun = opts.fun;
    let verbose = opts.verbose;
    let mch_max = opts.mch_max; /* Maximum entries in matching table. */
    let mch_min = opts.mch_min; /* Minimum entries in matching table. */
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
    let nam_org = opts.operands[0].clone();
    let nam_new = opts.operands[1].clone();
    let nam_out: OsString = if opts.operands.len() >= 3 {
        opts.operands[2].clone()
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
    let buffers = config::size_buffers(&opts);

    /* Open files and create file handlers (`main.cpp:647-752`). The inputs
     * may be opened a second time for `-t`'s patch phase (deviation 2).
     * This open stays ahead of the output open — the C++ order (647-752
     * before 754-774) — because the pinned open-failure tests
     * (`unopenable_org_exit_3_message`, `u_open_error_exits_3_4_5`) require
     * exit 3/4 WITHOUT the output file being created. The diff phase
     * consumes the readers; for `-u` this open is the (behavior-pinning)
     * probe and the patch phase opens its own fresh readers. */
    let inputs = match open_inputs(
        &nam_org,
        &nam_new,
        buffers.ll_buf_org,
        buffers.ll_buf_new,
        buffers.blk_sze,
    ) {
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
        li_ret = diff_phase(&mut opts, &nam_out, &buffers, inputs, file_out);
    } /* liFun == Diff or Test */

    if li_fun == Function::Patch || li_fun == Function::Test {
        li_ret = patch_phase(
            &mut opts,
            &nam_org,
            &nam_new,
            &nam_out,
            &buffers,
            out_is_stdout,
            li_fun == Function::Test,
        );
    } /* liFun == Patch or Test */

    /* Cleanup: the readers and writers drop in the phase calls above. */

    /* Exit (`main.cpp:897-932`): the single boundary prints the pinned error
     * text (if any) and yields the positive process exit code. */
    report(li_ret, verbose)
}
