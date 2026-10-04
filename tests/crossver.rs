//! Cross-version compatibility gate (spec §22.3; wire-format matrix §18.C):
//! every committed 0.8.1 golden patch under `tests/fixtures/golden/` —
//! generated once by the 0.8.1 oracle and never regenerated — must apply
//! with the 0.8.5 engine and restore the corresponding fixture "new" file
//! byte-exact (the one-way gate: 0.8.1 explicit-opcode patches are a subset
//! of the 0.8.5 grammar).
//!
//! Each patch is applied twice:
//! 1. via `jdiff -u` (the upstream 0.8.5 patch mode), and
//! 2. via a temp-dir copy of the binary named `jptch` (argv[0] dispatch —
//!    the port extension; upstream matches `jpatch` only, spec §21.2), which
//!    patches without any option.
//!
//! The reverse direction is deliberately NOT gated here: 0.8.1-era patchers
//! silently drop the 0.8.5 implicit-MOD data bytes (§18.C), so patches from
//! this port ≥0.8.5 are flagged unreadable for them in the README; no C++
//! 0.8.1 patcher takes part in any gate.

mod common;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The 0.8.1 golden corpus: per pair directory, the original and new fixture
/// files a patch must reproduce, and the number of committed `.jdf` patches
/// (bkocomu 10 + test2 12 — pins that the corpus stays complete; the two
/// `.rgn` and one `.asc` listings are diagnostics, not patches).
const PAIRS: &[(&str, &str, &str, usize)] = &[
    ("bkocomu", "bkocomu.0000.fil", "bkocomu.0009.fil", 10),
    ("test2", "test2.001.txt", "test2.002.txt", 12),
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn golden_dir() -> PathBuf {
    fixtures_dir().join("golden")
}

/// Copies the test binary into `dir` under the given argv[0] base name and
/// returns the copy's path. The `.exe` suffix is appended on Windows
/// (`std::env::consts::EXE_SUFFIX`): `CreateProcessW` would not find an
/// extensionless image, while the dispatch only inspects the basename
/// prefix (`jptch.exe` still starts with `jptch`, spec §21.2).
fn copy_binary_as(dir: &Path, name: &str) -> PathBuf {
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_jdiff"), &exe).expect("copy binary");
    exe
}

/// Runs a binary with `args`, retrying on `ETXTBSY`: a concurrently-running
/// test's child may inherit the copy's still-open write handle at clone time
/// (fd tables are copied at fork and `fs::copy` opens without `O_CLOEXEC`)
/// and keep it open through exec, failing `execve` with `Text file busy`.
/// The memguard opt-out keeps the byte-contract suite RAM-independent
/// (same rationale as tests/common/mod.rs and tests/oracle.rs).
fn run_copied(exe: &Path, args: &[&Path]) -> Output {
    for attempt in 0..100 {
        match Command::new(exe)
            .args(args)
            .env("JDIFF_UNSAFE_NO_MEMGUARD", "1")
            .output()
        {
            Ok(out) => return out,
            Err(err) if err.raw_os_error() == Some(26) && attempt < 99 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(err) => panic!("spawn {}: {err}", exe.display()),
        }
    }
    unreachable!("retry loop above either returns or panics")
}

/// The committed 0.8.1 patches: (pair, patch file), sorted for stable order.
fn committed_patches() -> Vec<(&'static str, PathBuf)> {
    let mut patches: Vec<(&'static str, PathBuf)> = Vec::new();
    for (pair, ..) in PAIRS {
        let dir = golden_dir().join(pair);
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read golden dir {}: {e}", dir.display()))
            .map(|e| e.expect("golden dir entry").path())
            .filter(|p| p.extension().is_some_and(|e| e == "jdf"))
            .collect();
        entries.sort();
        for p in entries {
            patches.push((pair, p));
        }
    }
    patches
}

/// Applies every 0.8.1 golden patch through `jdiff -u` (mode "u") and an
/// argv[0]=`jptch` copy of the binary (mode "argv0") and asserts the exact
/// restore of the pair's new fixture (spec §22.3 one-way gate).
#[test]
fn golden_081_patches_restore_exactly() {
    let patches = committed_patches();

    // The corpus must stay complete: 10 bkocomu + 12 test2 patches.
    for (pair, _, _, count) in PAIRS {
        let found = patches.iter().filter(|(p, _)| p == pair).count();
        assert_eq!(
            found, *count,
            "golden/{pair} must hold {count} committed .jdf patches (the cross-version corpus is never regenerated)"
        );
    }

    // The argv[0]=jptch copy of the single binary (spec §21.2).
    let argv0_dir = common::scratch("argv0");
    let jptch = copy_binary_as(argv0_dir.path(), "jptch");

    for (pair, patch) in &patches {
        let (_, org, new, _) = PAIRS.iter().find(|(p, ..)| p == pair).unwrap();
        let org = fixtures_dir().join(org);
        let new_bytes = fs::read(fixtures_dir().join(new)).expect("read new fixture");
        let patch_bytes = fs::read(patch).expect("read 0.8.1 golden patch");
        assert!(
            !patch_bytes.is_empty(),
            "0.8.1 golden patch {} must be non-empty",
            patch.display()
        );

        for (mode, exe, via_argv0) in [
            ("-u", Path::new(env!("CARGO_BIN_EXE_jdiff")), false),
            ("argv0", jptch.as_path(), true),
        ] {
            let dir = common::scratch(&format!("apply-{pair}-{mode}"));
            let restored = dir.path().join("restored.bin");
            let out = if via_argv0 {
                run_copied(exe, &[org.as_path(), patch.as_path(), restored.as_path()])
            } else {
                common::jdiff(&[
                    OsStr::new("-u"),
                    org.as_os_str(),
                    patch.as_os_str(),
                    restored.as_os_str(),
                ])
                .output()
                .expect("spawn jdiff -u")
            };
            assert_eq!(
                out.status.code(),
                Some(0),
                "{mode}: applying {} exited with {:?}\nstderr:\n{}",
                patch.display(),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.stdout.is_empty(),
                "{mode}: applying {} must not write to stdout (the restore lands in the file)",
                patch.display()
            );
            assert!(
                out.stderr.is_empty(),
                "{mode}: applying {} must be silent: {:?}",
                patch.display(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                fs::read(&restored).expect("restored bytes"),
                new_bytes,
                "{mode}: 0.8.1 patch {} did not restore {} byte-exact",
                patch.display(),
                new
            );
        }
    }
}

/// The argv[0] copies carry a `.exe` suffix on Windows (CreateProcessW only
/// finds `name.exe`, and the dispatch matches the basename prefix —
/// [`copy_binary_as`]) and the plain name elsewhere.
#[test]
fn argv0_copy_name_matches_platform() {
    let dir = common::scratch("suffix");
    let exe = copy_binary_as(dir.path(), "jptch");
    let name = exe.file_name().expect("copied file name").to_string_lossy();
    if cfg!(windows) {
        assert_eq!(name, "jptch.exe");
    } else {
        assert_eq!(name, "jptch");
    }
}
