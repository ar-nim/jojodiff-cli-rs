//! The patch (and `-t` second-half) phase (`main.cpp:871-876`): fresh
//! readers, append-vs-create output per `-t`, checked writes (EXI_WRI),
//! JPatcht wiring and execution.

use std::ffi::OsStr;
use std::io::Write;

use crate::cli::config::{Buffers, Options};
use crate::cli::run::{Sink, open_inputs, open_output_file, wrap_buffered};
use crate::defs::EXI_OK;
use crate::error::JDiffError;
use crate::jfileout::JFileOut;
use crate::jpatcht::JPatcht;

/// The patch (and `-t` second-half) phase (`main.cpp:871-876`). Opens its
/// own fresh readers (`nam_org`/`nam_new`) and — for the faithful-broken
/// `-t` (`is_test`, §21.3) — appends to the diff output; open failures
/// propagate to the caller's boundary, which prints the pinned text at the
/// same point in the output stream as the old inline `report` calls.
pub(crate) fn patch_phase(
    opts: &mut Options,
    nam_org: &OsStr,
    nam_new: &OsStr,
    nam_out: &OsStr,
    buffers: &Buffers,
    out_is_stdout: bool,
    is_test: bool,
) -> Result<i32, JDiffError> {
    let Buffers {
        ll_buf_org,
        ll_buf_new,
        blk_sze,
        ..
    } = *buffers;

    // The patch phase reads the source file and — for `-u` the patch,
    // for the faithful-broken `-t` (§21.3) the DESTINATION file — as the
    // patch (`main.cpp:871-876`). The readers are opened fresh here:
    // for `-u` this is their first and only open, for `-t` the C++
    // reuses its mid-cursor handles (the upstream bug — see module docs,
    // deviation 2). `-t`'s output appends to what the diff wrote.
    // (`?` propagates to the caller's boundary, which prints the pinned
    // text at the same point in the stream as the old inline `report`.)
    let mut inputs = open_inputs(nam_org, nam_new, ll_buf_org, ll_buf_new, blk_sze)?;
    let patch_sink = if out_is_stdout {
        Sink::Stdout(std::io::stdout().lock())
    } else if is_test {
        // Append to the diff output just flushed (C++: the same FILE*).
        Sink::File(open_output_file(nam_out, true)?)
    } else {
        Sink::File(open_output_file(nam_out, false)?)
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
        opts.verbose,
    );
    let li_ret = lo_jpatcht.jpatch().map(|()| EXI_OK);
    // Flush the patch writer at scope end, like the C++ exit-time flush.
    drop(lo_jpatcht.into_inner());

    li_ret
}
