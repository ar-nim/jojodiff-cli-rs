//! `jdiff` — thin wrapper: argv in, exit code out (spec §8.1). All logic
//! lives in the library's `cli` module; the module-level documentation of
//! the four CLI deviations lives in `jojodiff_cli_rs::cli`.

use std::ffi::OsString;

fn main() -> anyhow::Result<()> {
    let args: Vec<OsString> = std::env::args_os().collect();
    // Pinned errors are printed and coded inside `cli::run`'s boundary;
    // anyhow context wraps only truly unexpected failures.
    let code = jojodiff_cli_rs::cli::run(&args).map_err(anyhow::Error::from)?;
    std::process::exit(code);
}
