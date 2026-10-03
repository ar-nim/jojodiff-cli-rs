//! CLI support layer: the `getopt_long`-equivalent option scanner
//! (`src/cli/opts.rs`), option parsing and buffer sizing
//! (`src/cli/config.rs`), the byte-pinned text blocks (`src/cli/report.rs`),
//! the single error→(code, stderr) boundary (`src/cli/error.rs`), the
//! shared file plumbing (`src/cli/run.rs`) and the diff/patch phase bodies
//! used by the one `jdiff` binary (spec §18.D).

pub mod config;
pub mod diff_phase;
pub mod error;
pub mod opts;
pub mod patch_phase;
pub mod report;
pub mod run;

pub use config::{Buffers, Function, Options, function_from_argv0, parse, size_buffers};
pub use opts::{Getopt, HasArg, OPT_LNG, OPT_SHT};
