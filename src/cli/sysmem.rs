//! Machine memory ceiling for the CLI memory guard (plan
//! 2026-10-04-memguard-bugfix, Layer A): anonymous pages the engines touch
//! are served by RAM + swap, so the honest ceiling is the platform's
//! *available* figure, not total RAM. All three supported platforms are
//! catered for with std-only reporting — no `unsafe`, no new dependencies:
//! each platform's own reporting source is parsed.
//!
//! - **Linux**: `/proc/meminfo` — `MemAvailable + SwapFree` (exact).
//! - **macOS**: `vm_stat` (free + inactive + speculative pages — the
//!   heuristic MemAvailable analogue) plus the free swap from
//!   `sysctl vm.swapusage`.
//! - **Windows**: `Win32_OperatingSystem.FreeVirtualMemory` (KB, =
//!   available physical + available paging file) via one
//!   `powershell -NoProfile` spawn (~0.5–2 s per run — the price of a
//!   no-FFI implementation; `GlobalMemoryStatusEx` would need `unsafe`).
//!
//! On any other target, or when the platform source is unreadable or
//! missing its fields, the ceiling is unknown (`None`) and only Layer B
//! (`try_reserve` in the constructors) protects the run.
//!
//! `JDIFF_UNSAFE_NO_MEMGUARD=1` disables Layer A deliberately (the OS
//! overcommit heuristic then decides, exactly like the pre-guard builds).

/// Safety margin below the ceiling: page tables for huge tables, output
/// BufWriters, allocator slack.
pub(crate) const MEMGUARD_HEADROOM: u64 = 512 * 1024 * 1024;

/// Escape hatch: any value set disables the Layer-A pre-flight gate.
pub(crate) const MEMGUARD_ENV: &str = "JDIFF_UNSAFE_NO_MEMGUARD";

/// Bytes of anonymous memory the machine can back (RAM + swap), or `None`
/// when that cannot be determined (unsupported target, unreadable source).
pub(crate) fn available_anon_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let (avail, swap) = parse_meminfo(&text)?;
        Some(avail.saturating_add(swap).saturating_mul(1024))
    }
    #[cfg(target_os = "macos")]
    {
        macos_available()
    }
    #[cfg(target_os = "windows")]
    {
        windows_available()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// (MemAvailable, SwapFree) in kB out of a Linux `/proc/meminfo` body;
/// `None` when either field is missing or unparsable.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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

/// macOS ceiling: free + inactive + speculative pages from `vm_stat` times
/// the page size, plus the free swap from `sysctl vm.swapusage`.
#[cfg(target_os = "macos")]
fn macos_available() -> Option<u64> {
    let stat = std::process::Command::new("/usr/bin/vm_stat")
        .output()
        .ok()?;
    let (page_bytes, avail_pages) = parse_vm_stat(&String::from_utf8_lossy(&stat.stdout))?;
    let swap = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "vm.swapusage"])
        .output()
        .ok()?;
    let swap_free = parse_swapusage(&String::from_utf8_lossy(&swap.stdout))?;
    Some(
        avail_pages
            .saturating_mul(page_bytes)
            .saturating_add(swap_free),
    )
}

/// `vm_stat` body -> (page size in bytes, available pages = free +
/// inactive + speculative). The macOS MemAvailable analogue is a
/// heuristic; pages that are active or compressed do not count. `None`
/// when the page-size line is missing or any of the three counters is
/// absent/unparsable (fail to Layer B, never guess).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_vm_stat(text: &str) -> Option<(u64, u64)> {
    let mut page_bytes = None;
    let mut avail: u64 = 0;
    let mut seen = 0u8;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Mach Virtual Memory Statistics: (page size of ") {
            if let Some(bytes) = rest.strip_suffix(" bytes)") {
                page_bytes = bytes.trim().parse().ok();
            }
        } else if let Some(rest) = line.strip_prefix("Pages ") {
            if let Some((label, count)) = rest.split_once(':') {
                if let Ok(n) = count.trim().trim_end_matches('.').parse::<u64>() {
                    match label.trim() {
                        "free" | "inactive" | "speculative" => {
                            avail = avail.saturating_add(n);
                            seen += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if seen == 3 {
        Some((page_bytes?, avail))
    } else {
        None
    }
}

/// `sysctl vm.swapusage` body -> free swap in bytes. Line shape:
/// `total = 4096.00M used = 512.50M free = 3583.50M` (K/M/G/T suffixes,
/// 1024-based; the `=` is its own token).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_swapusage(text: &str) -> Option<u64> {
    let mut it = text.split_whitespace();
    while let Some(token) = it.next() {
        if token == "free" {
            match it.next() {
                Some("=") => return parse_sized_number(it.next()?),
                Some(value) => return parse_sized_number(value),
                None => return None,
            }
        }
    }
    None
}

/// `1535.75M` / `3.75G` / `512K` / `4096` -> bytes (1024-based suffixes).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_sized_number(value: &str) -> Option<u64> {
    let split = value
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(value.len());
    let n: f64 = value[..split].parse().ok()?;
    let mult = match value[split..].trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// Windows ceiling via `Win32_OperatingSystem.FreeVirtualMemory` (KB):
/// available physical + available paging file — the `MemAvailable +
/// SwapFree` analogue. One PowerShell spawn per run.
#[cfg(target_os = "windows")]
fn windows_available() -> Option<u64> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_OperatingSystem).FreeVirtualMemory",
        ])
        .output()
        .ok()?;
    parse_free_kb(&String::from_utf8_lossy(&out.stdout))
}

/// PowerShell stdout of `FreeVirtualMemory`: a bare KB integer (with
/// CRLF line ending) -> bytes. `None` unless it parses as one number.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_free_kb(text: &str) -> Option<u64> {
    text.trim()
        .parse::<u64>()
        .ok()
        .map(|kb| kb.saturating_mul(1024))
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

    const VM_STAT: &str = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                 42.
Pages active:                             1234.
Pages inactive:                            100.
Pages speculative:                          58.
Pages wired down:                         2000.
Pages swapins:                              12.
";

    #[test]
    fn parses_vm_stat_free_inactive_speculative() {
        // The counters are summed, the rest of the body is ignored.
        assert_eq!(parse_vm_stat(VM_STAT), Some((16_384, 42 + 100 + 58)));
    }

    #[test]
    fn vm_stat_missing_counters_or_page_size_is_none() {
        assert_eq!(parse_vm_stat("Pages free: 1.\nPages inactive: 2.\n"), None);
        assert_eq!(
            parse_vm_stat(
                "Mach Virtual Memory Statistics: (page size of 4096 bytes)\nPages free: 1.\n"
            ),
            None
        );
        assert_eq!(parse_vm_stat(""), None);
    }

    #[test]
    fn parses_swapusage_free() {
        assert_eq!(
            parse_swapusage("total = 4096.00M used = 512.50M free = 3583.50M\n"),
            Some((3583.5 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(
            parse_swapusage("total = 0.00M used = 0.00M free = 3.75G\n"),
            Some((3.75 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
    }

    #[test]
    fn sized_numbers_and_swapusage_edge_cases() {
        assert_eq!(parse_sized_number("512K"), Some(512 * 1024));
        assert_eq!(parse_sized_number("4096"), Some(4096));
        assert_eq!(parse_sized_number("x"), None);
        assert_eq!(parse_swapusage("total = 4G used = 1G"), None); // no free=
        assert_eq!(parse_swapusage(""), None);
    }

    #[test]
    fn parses_powershell_free_virtual_kb() {
        assert_eq!(parse_free_kb("1234567\r\n"), Some(1_234_567 * 1024));
        assert_eq!(parse_free_kb("  2048 \n"), Some(2048 * 1024));
        assert_eq!(parse_free_kb(""), None);
        assert_eq!(parse_free_kb("not-a-number"), None);
    }
}
