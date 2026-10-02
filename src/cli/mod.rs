//! CLI support layer: the `getopt_long`-equivalent option scanner
//! (`src/cli/opts.rs`) used by the single `jdiff` binary (spec §18.D).

pub mod opts;

pub use opts::{Getopt, HasArg, OPT_LNG, OPT_SHT};
