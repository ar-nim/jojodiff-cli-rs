//! `jojodiff-cli-rs` — an independent Rust port of JojoDiff 0.8.5.
//!
//! Provides the `jdiff` tool as a reusable library and a single CLI: patching
//! is `jdiff -u` or an `argv[0]` dispatch (copies/links named `jpatch*`/`jptch*`
//! patch; spec §21.2). Patches, listings, verbose output and exit codes are
//! held byte-identical to the original C++ implementation, which serves as
//! the verification oracle.
//!
//! Module map:
//!
//! - **Engine** (the ported C++ classes): [`defs`], [`jdebug`], [`jdiff`],
//!   [`jfile`], [`jfileout`], [`jhashpos`], [`jmatchtable`], [`jout`],
//!   [`jpatcht`].
//! - **CLI** ([`cli`]): option parsing and buffer sizing ([`cli::config`],
//!   [`cli::opts`]), the phase functions and orchestrator
//!   ([`cli::diff_phase`], [`cli::patch_phase`], [`cli::run()`]), the
//!   byte-pinned greeting/usage/report texts ([`cli::report`]) and the error
//!   boundary ([`cli::error`]).
//! - **Errors** ([`error`]): the library-wide [`error::JDiffError`].
//!
//! Error model: the engine's status APIs return `Result<T, JDiffError>` (the
//! legacy exit-code vocabulary survives as [`error::JDiffError::exit_code`]
//! and the `Raw` variant). [`cli::error::report`] is the single
//! text/exit-code boundary — the one place an error becomes pinned stderr
//! bytes and a process exit code; the library never exits the process.
//!
//! Dependencies (spec §13): [`thiserror`](https://docs.rs/thiserror) derives
//! the `Display`/`std::error::Error` impls of [`error::JDiffError`] (compile-
//! time only, no runtime code beyond the generated impls),
//! [`anyhow`](https://docs.rs/anyhow) wraps `cli::run` in the thin `jdiff`
//! binary (no `anyhow` types cross into the library), and
//! [`sysinfo`](https://docs.rs/sysinfo) provides the memory guard's
//! platform ceiling (default features off, `system` only). There is no
//! `unsafe` in this crate.
//! See the repository README for project status and the docs in
//! `docs/superpowers/` for the port plan and functional specification.

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

/// Allocation-guarded zeroed Vec (memory guard Layer B, plan
/// 2026-10-04-memguard-bugfix): `try_reserve_exact` instead of the
/// aborting `vec![fill; len]`, mapping an OS refusal (allocation failure,
/// overcommit limit) to [`error::JDiffError::Memory`] — exit 10 at the CLI
/// boundary instead of a Rust allocation abort.
pub(crate) fn try_zeroed_vec<T: Clone>(len: usize, fill: T) -> Result<Vec<T>, error::JDiffError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| error::JDiffError::Memory)?;
    v.resize(len, fill);
    Ok(v)
}

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
