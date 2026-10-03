//! CLI support layer: the `getopt_long`-equivalent option scanner
//! (`src/cli/opts.rs`), option parsing and buffer sizing
//! (`src/cli/config.rs`), the byte-pinned text blocks (`src/cli/report.rs`),
//! the single error→(code, stderr) boundary (`src/cli/error.rs`), the
//! shared file plumbing and full execution entry point (`src/cli/run.rs`)
//! and the diff/patch phase bodies used by the one `jdiff` binary
//! (spec §18.D).
//!
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

pub mod config;
pub mod diff_phase;
pub mod error;
pub mod opts;
pub mod patch_phase;
pub mod report;
pub mod run;

pub use config::{Buffers, Function, Options, function_from_argv0, parse, size_buffers};
pub use opts::{Getopt, HasArg, OPT_LNG, OPT_SHT};
pub use run::run;
