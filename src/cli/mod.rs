//! CLI support layer: the `getopt_long`-equivalent option scanner
//! (`src/cli/opts.rs`) and option parsing and buffer sizing
//! (`src/cli/config.rs`) used by the single `jdiff` binary (spec §18.D).

pub mod config;
pub mod opts;

pub use config::{Buffers, Function, Options, function_from_argv0, parse, size_buffers};
pub use opts::{Getopt, HasArg, OPT_LNG, OPT_SHT};
