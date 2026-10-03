//! CLI support layer: the `getopt_long`-equivalent option scanner
//! (`src/cli/opts.rs`), option parsing and buffer sizing
//! (`src/cli/config.rs`), the byte-pinned text blocks (`src/cli/report.rs`)
//! and the single error→(code, stderr) boundary (`src/cli/error.rs`) used
//! by the one `jdiff` binary (spec §18.D).

pub mod config;
pub mod error;
pub mod opts;
pub mod report;

pub use config::{Buffers, Function, Options, function_from_argv0, parse, size_buffers};
pub use opts::{Getopt, HasArg, OPT_LNG, OPT_SHT};
