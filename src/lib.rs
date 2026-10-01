//! `jojodiff-cli-rs` — an independent, byte-compatible Rust port of JojoDiff 0.8.1.
//!
//! Provides the `jdiff` and `jptch` binary diff/patch tools both as a reusable
//! library and as two standalone CLIs. Patches, listings, verbose output and exit
//! codes are held byte-identical to the original C++ implementation, which serves
//! as the verification oracle.
//!
//! The library is std-only and contains no `unsafe`. See the repository README
//! for project status and the docs in `docs/superpowers/` for the port plan and
//! functional specification.

pub mod defs;
pub mod jdebug;
pub mod jfile;
