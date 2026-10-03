//! `jojodiff-cli-rs` — an independent Rust port of JojoDiff 0.8.5.
//!
//! Provides the `jdiff` tool as a reusable library and a single CLI: patching
//! is `jdiff -u` or an argv[0] dispatch (copies/links named `jpatch*`/`jptch*`
//! patch; spec §21.2). Patches, listings, verbose output and exit codes are
//! held byte-identical to the original C++ implementation, which serves as
//! the verification oracle.
//!
//! The library is std-only apart from one compile-time dependency,
//! [`thiserror`](https://docs.rs/thiserror) (spec §13): it derives the
//! `Display`/`std::error::Error` impls of the library-wide
//! [`error::JDiffError`] — no runtime code beyond the generated impls. There
//! is no `unsafe`. See the repository README for project status and the docs
//! in `docs/superpowers/` for the port plan and functional specification.

pub mod cli;
pub mod defs;
pub mod error;
pub mod jdebug;
pub mod jdiff;
pub mod jfile;
pub mod jfileout;
pub mod jhashpos;
pub mod jmatchtable;
pub mod jout;
pub mod jpatcht;

/// Shared test utilities (test builds only).
#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

    /// Serializes all tests that touch the shared process-global test state —
    /// since Task 16 (0.8.5 re-target) only the debug `GB_DBG` flags
    /// (`jdebug`): the 0.8.1 `HSH_RPR` repair counter this lock used to
    /// guard retired into a `JMatchTable` instance counter (`miHshRpr`,
    /// spec §18.E). Cargo runs tests on parallel threads, and the `-d*`
    /// flag tests mutate the process-global flag array that engine tests
    /// read behind `dbg(..)`; see the Task 11 finding this lock originated
    /// from.
    /// Only the `-d*` flag tests call it (a `debug`-feature test module), so
    /// non-debug test builds would see it as dead.
    #[cfg_attr(not(feature = "debug"), allow(dead_code))]
    pub fn gb_dbg_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
