//! Shared file plumbing of the `jdiff` CLI (`main.cpp:647-774`): the input
//! reader pair, the output sink, the diff-path error-ignoring writer and the
//! writer-buffering decision. The phases ([`crate::cli::diff_phase`] /
//! [`crate::cli::patch_phase`]) and the binary's `real_main` are built on
//! these.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use crate::error::JDiffError;
use crate::jdebug::DBG_TO_STDOUT;
use crate::jfile::{JFile, JFileAhead};

/// The two input readers (`lpJflOrg`/`lpJflNew`).
pub struct Inputs {
    pub org: Box<dyn JFile>,
    pub new: Box<dyn JFile>,
}

/// Opens both input files as buffered look-ahead readers
/// (`main.cpp:647-752`): `-` re-opens `/dev/stdin` (deviation 1), anything
/// else opens the named file; failures return `OpenFirst`/`OpenSecond` with
/// the name moved in — the boundary (`cli::error::report`) prints the
/// `main.cpp` messages and exits 3/4.
pub fn open_inputs(
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
pub fn open_output_file(nam_out: &OsStr, append: bool) -> Result<File, JDiffError> {
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
pub enum Sink {
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
