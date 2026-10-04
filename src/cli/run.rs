//! Shared file plumbing and the full CLI execution of the `jdiff` binary
//! (`main.cpp:647-932`): the input reader pair, the output sink, the
//! diff-path error-ignoring writer, the writer-buffering decision and
//! [`run`], the parse → gate → open → dispatch → report orchestration. The
//! phases ([`crate::cli::diff_phase`] / [`crate::cli::patch_phase`]) are
//! built on the plumbing.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use crate::cli::config::{self, Function};
use crate::cli::error::report;
use crate::cli::report::{print_greeting, print_notes, print_usage};
use crate::error::JDiffError;
use crate::jdebug::{DBG_TO_STDOUT, dbg_print};
use crate::jfile::{JFile, JFileAhead};

/// Full CLI execution (`main.cpp`): returns the process exit code —
/// never exits itself. `main.cpp`'s structure: `argv[0]` dispatch, parse,
/// greeting/usage gate, operand extraction, buffer sizing, input/output
/// open, function dispatch, boundary report.
///
/// # Contract
///
/// `args` must include `argv[0]` (the process name), exactly as `main`
/// receives `argv`: its basename selects the default function (`jpatch*`/
/// `jptch*` patch, everything else diffs) and the option scanner consumes
/// the slot. An empty slice underflows the `optind` arithmetic
/// ([`crate::cli::Getopt::optind`] reports 0, `config::parse` subtracts 1):
/// debug builds panic on the subtraction, release builds wrap and land in
/// the usage path (exit 2).
///
/// # Errors
///
/// All pinned paths print and code themselves at the single
/// [`crate::cli::error::report`] boundary and return `Ok(process exit
/// code)`; the `Err` half is reserved for truly unexpected failures (none
/// today) so the thin binary can wrap `run` in `anyhow` without touching
/// the pinned stderr bytes or exit codes.
pub fn run(args: &[OsString]) -> Result<i32, JDiffError> {
    let ai_arg_cnt = args.len(); /* aiArgCnt */

    /* Parse option-switches (`main.cpp:276-476`): argv[0] dispatch, default
     * settings and the getopt_long loop with GNU permutation moved to
     * `cli::config::parse` (unit-tested there); `?` (unknown option or
     * argument error) sets liHlp=1 and parsing CONTINUES. */
    let mut opts = config::parse(args);
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
            return Ok(report(Err(JDiffError::Args), verbose));
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
        return Ok(report(Err(JDiffError::Args), verbose));
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
        Err(e) => return Ok(report(Err(e), verbose)),
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
            Err(e) => return Ok(report(Err(e), verbose)),
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
        li_ret =
            crate::cli::diff_phase::diff_phase(&mut opts, &nam_out, &buffers, inputs, file_out);
    } /* liFun == Diff or Test */

    if li_fun == Function::Patch || li_fun == Function::Test {
        li_ret = crate::cli::patch_phase::patch_phase(
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
    Ok(report(li_ret, verbose))
}

/// The two input readers (`lpJflOrg`/`lpJflNew`).
pub(crate) struct Inputs {
    pub(crate) org: Box<dyn JFile>,
    pub(crate) new: Box<dyn JFile>,
}

/// Opens both input files as buffered look-ahead readers
/// (`main.cpp:647-752`): `-` re-opens `/dev/stdin` (deviation 1), anything
/// else opens the named file; failures return `OpenFirst`/`OpenSecond` with
/// the name moved in — the boundary (`cli::error::report`) prints the
/// `main.cpp` messages and exits 3/4.
pub(crate) fn open_inputs(
    nam_org: &OsStr,
    nam_new: &OsStr,
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
                    name: nam_org.to_os_string(),
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
                    name: nam_new.to_os_string(),
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
pub(crate) fn open_output_file(nam_out: &OsStr, append: bool) -> Result<File, JDiffError> {
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
pub(crate) fn wrap_buffered<W: Write + 'static>(sink: W, out_is_stdout: bool) -> Box<dyn Write> {
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
pub(crate) enum Sink {
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
pub(crate) struct IgnoringWriter<W: Write> {
    pub(crate) inner: W,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// No operands: the `liHlp`-gated usage path exits 2 via the boundary
    /// (`main.cpp:594-599`; pinned byte-exact by the integration suite —
    /// these unit tests assert codes only). All pinned paths return
    /// `Ok(process code)`, so `unwrap` asserts that contract too.
    #[test]
    fn run_with_no_args_is_arg_error_exit_2() {
        assert_eq!(run(&[OsString::from("jdiff")]).unwrap(), 2);
    }

    /// Both inputs from stdin: the operand-gated error site exits 2
    /// (`main.cpp:610-616` shape; three operands so the nargs gate passes).
    #[test]
    fn run_with_both_inputs_dash_is_arg_error_exit_2() {
        let args: Vec<OsString> = ["jdiff", "-", "-", "-"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(run(&args).unwrap(), 2);
    }

    /// An unknown option sets liHlp=1 and parsing continues; with no
    /// operands the help path prints usage (no "Not enough arguments" line,
    /// `liHlp != 0`) and exits 2.
    #[test]
    fn run_unknown_option_no_operands_is_usage_exit_2() {
        let args: Vec<OsString> = ["jdiff", "-Z"].iter().map(OsString::from).collect();
        assert_eq!(run(&args).unwrap(), 2);
    }

    /// Dedup is accepted and exits 20 (§21.4, the EXI_ERR replacement):
    /// the inputs open (scratch files), no output is opened, the crash-path
    /// substitute `NotPorted` crosses the boundary. Scratch via the
    /// `tempfile` dev-dependency's RAII `TempDir`, like the integration
    /// harness.
    #[test]
    fn run_dedup_is_not_ported_exit_20() {
        let dir = tempfile::TempDir::new().expect("create scratch dir");
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"hello world hello").expect("write scratch a");
        std::fs::write(&b, b"hello world byebye").expect("write scratch b");
        let args: Vec<OsString> = [
            OsString::from("jdiff"),
            OsString::from("-y"),
            a.into_os_string(),
            b.into_os_string(),
            dir.path().join("out.bin").into_os_string(),
        ]
        .to_vec();
        let code = run(&args);
        assert_eq!(code.unwrap(), -crate::defs::EXI_ERR);
    }
}
