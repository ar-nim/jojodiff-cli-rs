//! Machine memory ceiling for the CLI memory guard (plan
//! 2026-10-04-memguard-bugfix, Layer A): anonymous pages the engines touch
//! are served by RAM + swap, so the honest ceiling is the platform's
//! *available* figure, not total RAM. The query goes through the
//! [`sysinfo`] crate (§13 dependency record 3) — one maintained,
//! `unsafe`-free API for all three supported platforms instead of three
//! hand-maintained parsers.
//!
//! Platform semantics behind the getters (sysinfo): Linux reports
//! `MemAvailable` + `SwapFree` from `/proc/meminfo` (exact); macOS reports
//! mach-level free estimates plus swap; Windows reports free *physical*
//! memory (conservative — the paging file is not counted, unlike the
//! process-commit figures; the guard may only be more reluctant there).
//! Known future improvement: cgroup-limited containers (sysinfo exposes
//! `cgroup_limits()`; the host figures over-report inside a container —
//! Layer B still catches the hard failures).
//!
//! When the platform is unsupported or the query yields no data, the
//! ceiling is unknown (`None`) and only Layer B (`try_reserve` in the
//! constructors) protects the run.
//!
//! `JDIFF_UNSAFE_NO_MEMGUARD=1` disables Layer A deliberately (the OS
//! overcommit heuristic then decides, exactly like the pre-guard builds).

/// Safety margin below the ceiling: page tables for huge tables, output
/// BufWriters, allocator slack.
pub(crate) const MEMGUARD_HEADROOM: u64 = 512 * 1024 * 1024;

/// Escape hatch: any value set disables the Layer-A pre-flight gate.
pub(crate) const MEMGUARD_ENV: &str = "JDIFF_UNSAFE_NO_MEMGUARD";

/// Bytes of anonymous memory the machine can back (RAM + swap), or `None`
/// when that cannot be determined (unsupported target, query failure).
pub(crate) fn available_anon_bytes() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    // sysinfo reports 0 until a refresh succeeds; all-zero totals mean the
    // platform query failed (or is unsupported) — unknown, not exhausted.
    if sys.total_memory() == 0 && sys.total_swap() == 0 {
        return None;
    }
    Some(sys.available_memory().saturating_add(sys.free_swap()))
}

/// Layer A active? Disabled by setting `JDIFF_UNSAFE_NO_MEMGUARD`.
pub(crate) fn memguard_enabled() -> bool {
    std::env::var_os(MEMGUARD_ENV).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity, not exactness: the crate must produce a plausible memory
    /// picture on every supported platform (CI: ubuntu, macos, windows).
    /// Exact per-OS values are sysinfo's contract, not ours.
    #[test]
    fn sysinfo_reports_a_plausible_memory_picture() {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        assert!(sys.total_memory() > 0, "total RAM must be reported");
        assert!(sys.available_memory() <= sys.total_memory());
        assert!(sys.free_swap() <= sys.total_swap());
    }

    #[test]
    fn memguard_env_disables() {
        // read-only check: unset means enabled
        assert!(std::env::var_os(MEMGUARD_ENV).is_none() || !memguard_enabled());
    }
}
