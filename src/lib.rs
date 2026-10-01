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
pub mod jdiff;
pub mod jfile;
pub mod jhashpos;
pub mod jmatchtable;
pub mod jout;

/// Shared test utilities (test builds only).
#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

    /// Serializes tests that touch process-global state — the [`HSH_RPR`]
    /// repair counter (`jmatchtable`) and the debug `GB_DBG` flags
    /// (`jdebug`): cargo runs tests on parallel threads, and guarding such
    /// tests with per-module mutexes still leaves two modules racing on the
    /// same global (found in Task 11, where the added `jdebug` tests'
    /// scheduling perturbation made `jdiff::stats_and_hash_repairs` and the
    /// `jmatchtable` counter tests collide deterministically).
    pub fn hsh_rpr_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
