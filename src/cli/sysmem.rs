//! Machine memory ceiling for the CLI memory guard (plan
//! 2026-10-04-memguard-bugfix, Layer A): anonymous pages the engines touch
//! are served by RAM + swap, so the honest ceiling is `MemAvailable +
//! SwapFree` from Linux `/proc/meminfo` (kB fields × 1024). Off Linux, or
//! when the file/fields are missing, the ceiling is unknown (`None`) and
//! only Layer B (`try_reserve` in the constructors) protects the run.
//!
//! `JDIFF_UNSAFE_NO_MEMGUARD=1` disables Layer A deliberately (the OS
//! overcommit heuristic then decides, exactly like the pre-guard builds).

/// Safety margin below the ceiling: page tables for huge tables, output
/// BufWriters, allocator slack.
pub(crate) const MEMGUARD_HEADROOM: u64 = 512 * 1024 * 1024;

/// Escape hatch: any value set disables the Layer-A pre-flight gate.
pub(crate) const MEMGUARD_ENV: &str = "JDIFF_UNSAFE_NO_MEMGUARD";

/// (MemAvailable, SwapFree) in kB out of a `/proc/meminfo` body; `None`
/// when either field is missing or unparsable.
pub(crate) fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut avail = None;
    let mut swap = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("MemAvailable:") => avail = it.next().and_then(|v| v.parse().ok()),
            Some("SwapFree:") => swap = it.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Some((avail?, swap?))
}

/// Bytes of anonymous memory the machine can back (RAM + swap), or `None`
/// when that cannot be determined (non-Linux, unreadable file).
pub(crate) fn available_anon_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let (avail, swap) = parse_meminfo(&text)?;
    Some(avail.saturating_add(swap).saturating_mul(1024))
}

/// Layer A active? Disabled by setting `JDIFF_UNSAFE_NO_MEMGUARD`.
pub(crate) fn memguard_enabled() -> bool {
    std::env::var_os(MEMGUARD_ENV).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
MemTotal:       16000000 kB
MemFree:          500000 kB
MemAvailable:   12000000 kB
SwapTotal:       4000000 kB
SwapFree:        3000000 kB
";

    #[test]
    fn parses_avail_and_swap() {
        assert_eq!(parse_meminfo(FIXTURE), Some((12_000_000, 3_000_000)));
    }

    #[test]
    fn missing_fields_are_none() {
        assert_eq!(parse_meminfo("MemTotal: 100 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
        // Present but unparsable values are None too.
        assert_eq!(parse_meminfo("MemAvailable: x kB\nSwapFree: 1 kB\n"), None);
    }

    #[test]
    fn memguard_env_disables() {
        // read-only check: unset means enabled
        assert!(std::env::var_os(MEMGUARD_ENV).is_none() || !memguard_enabled());
    }
}
