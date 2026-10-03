//! The diff (and `-t` first-half) phase (`main.cpp:776-869`): sequential
//! auto-detect warnings (mutating `opts` exactly as the C++ locals),
//! writer construction with the diff-path ignore policy, JDiff wiring,
//! parameter echo, execution, the EXI_EQL/EXI_DIF swap, statistics.

use std::ffi::OsStr;
use std::io::Write;

use crate::cli::config::{Buffers, Options};
use crate::cli::run::{IgnoringWriter, Inputs, Sink, wrap_buffered};
use crate::defs::{EXI_DIF, EXI_EQL};
use crate::error::JDiffError;
use crate::jdebug::dbg_print;
use crate::jdiff::JDiff;
use crate::jout::{JOut, JOutAsc, JOutBin, JOutRgn};

/// The diff (and `-t` first-half) phase (`main.cpp:776-869`). The inputs
/// arrive already opened by the caller (the shared `main.cpp:647-752` open
/// that must precede the output open); `-t`'s patch phase opens its own
/// fresh readers. The output sink arrives from the caller's
/// `main.cpp:754-774` open; `nam_out` re-derives the stdout decision for
/// the writer wrapping.
pub fn diff_phase(
    opts: &mut Options,
    nam_out: &OsStr,
    buffers: &Buffers,
    inputs: Inputs,
    file_out: Option<Sink>,
) -> Result<i32, JDiffError> {
    let out_is_stdout = *nam_out == *"-";
    let Buffers {
        ll_buf_org,
        ll_buf_new,
        blk_sze,
        ahd_max,
    } = *buffers;

    /* Perform JDiff */
    // Switch to sequential source file (`main.cpp:780-788`)
    if !opts.seq_org && inputs.org.is_sequential() {
        opts.seq_org = true;
        opts.cmp_all = false; // only compare data within the buffer
        opts.src_bkt = false; // only backtrack on source file in buffer
        opts.src_scn = 0; // no pre-scan indexing

        dbg_print(format_args!(
            "\n{}\n",
            "Warning: Source file is a sequential file, assuming -p."
        ));
    }

    // Switch to sequential destination file (`main.cpp:790-795`)
    if !opts.seq_new && inputs.new.is_sequential() {
        opts.seq_new = true;
        opts.mch_min = 0; // only search within the buffer
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
    let writer: Box<dyn Write> = wrap_buffered(IgnoringWriter { inner: out_sink }, out_is_stdout);
    let jout: Box<dyn JOut> = match opts.out_typ {
        1 => Box::new(JOutAsc::new(writer)),
        // --compat-081 (§21.16) selects the byte-exact 0.8.1 writer
        // policy; the default keeps the 0.8.5 implicit-MOD format.
        0 => Box::new(JOutBin::with_compat_081(writer, opts.compat_081)),
        _ => Box::new(JOutRgn::new(writer)),
    };

    /* Initialize JDiff object (`main.cpp:817-820`). */
    let mut lo_jdiff = JDiff::new(
        inputs.org,
        inputs.new,
        jout,
        opts.hsh_mbt,
        opts.verbose,
        opts.src_bkt,
        opts.src_scn != 0,
        opts.mch_max,
        opts.mch_min,
        i64::from(ahd_max),
        opts.cmp_all,
    );

    /* Show execution parameters (`main.cpp:822-836`), verbatim including
     * the stale `(-s)`/`(-b)` letters and the "disbale" typo. */
    if opts.verbose > 1 {
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
            opts.mch_min
        ));
        dbg_print(format_args!(
            "Max number of matches to search  (-x): {}\n",
            opts.mch_max
        ));
        dbg_print(format_args!(
            "Compare out-of-buffer (-f to disable): {}\n",
            if opts.cmp_all { "yes" } else { "no" }
        ));
        dbg_print(format_args!(
            "Full indexing scan   (-ff to disbale): {}\n",
            if opts.src_scn > 0 { "yes" } else { "no" }
        ));
        dbg_print(format_args!(
            "Backtrace allowed     (-p to disable): {}\n",
            if opts.src_bkt { "yes" } else { "no" }
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
    let li_ret = match engine {
        Ok(()) if stats.dta > 0 => Ok(EXI_DIF),
        Ok(()) => Ok(EXI_EQL),
        Err(e) => Err(e),
    };

    /* Write statistics (`main.cpp:847-869`). */
    if opts.verbose > 1 {
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
    if opts.verbose > 0 {
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

    // The C++ keeps the post-warning `lbSeqOrg`/`lbSeqNew` locals alive
    // until end of main without reading them; the mirror mutations on
    // `opts` above are dead for us too — one explicit read keeps the
    // intent visible without changing the 1:1 shape.
    let _ = (opts.seq_org, opts.seq_new);

    li_ret
}
