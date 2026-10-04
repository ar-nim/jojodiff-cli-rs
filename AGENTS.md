# AGENTS.md

Instructions for AI coding agents working in this repository. This is the
**canonical file** — `CLAUDE.md`, `GEMINI.md` and
`.github/copilot-instructions.md` are pointers here and carry no content of
their own; edit this file, never the pointers. Human contributors: start with
[CONTRIBUTING.md](CONTRIBUTING.md) — everything below applies to you too
unless marked agent-specific.

## What this project is

`jojodiff-cli-rs` is a 1:1 Rust port of JojoDiff 0.8.5 (C++). The **byte
contract** governs everything: patches, listings, verbose output and exit
codes must stay byte-identical to the C++ oracle for every input that runs
today. Before changing any behavior, read spec §21
(`docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`) — quirks
that look like bugs are often replicated on purpose. Implementation plans
and release notes live under `docs/superpowers/`.

## Non-negotiables

- **Byte contract**: no change to any stdout/stderr byte or exit code for
  configurations that run today. New output is allowed only on paths that
  previously crashed (e.g. the memory guard's exit-10 refusal text).
- **MSRV 1.85** (edition 2024 — no let-chains), **no `unsafe`**, **no new
  dependencies** without a spec decision record (§13).
- **Conventional Commits** (`feat:`, `fix:`, `docs:`, `test:`,
  `refactor:`, `ci:`), one logical change per commit.
- The full gate must pass before you claim done:
  `cargo fmt --all --check && cargo clippy --all-targets --all-features
  -- -D warnings && cargo test --all-features`. Report failures verbatim;
  never claim green without fresh output in front of you.

## Shell discipline

- If the `rtk` token-optimizing proxy is on PATH, prefix shell commands
  with `rtk`; use `rtk proxy <cmd>` when you need raw or piped output.
  Without `rtk`, run commands normally.
- **Never gate a follow-up action on a piped verification command.**
  `cargo test | tail -5 && git commit …` commits on tail's exit status,
  not the suite's. Run verification bare and judge its own exit code and
  summary; pipe output only for display, never for control flow.
- Wrap spawned `jdiff` invocations in tests and probes with a `timeout` —
  a runaway engine can fill the disk.

## Testing rules

- TDD: failing test first, watch it fail for the right reason, minimal
  fix, commit. Red-green for every regression fix.
- Tests must be **machine-independent**: never depend on host RAM, disk
  size or load. Refusal-path vectors are petabyte-scale (exceed any
  conceivable machine); never GB-scale.
- Byte-contract/golden harnesses set the documented escape hatch
  (`JDIFF_UNSAFE_NO_MEMGUARD=1`) so golden comparisons never depend on
  the host's memory state; the feature's own tests exercise the guard
  separately (`tests/memguard.rs`).
- Compute pinned test vectors with the actual code/formatter, never by
  hand.

## Platform scope

**Windows, macOS and Linux are all first-class** — CI runs the suite on
all three. Platform-specific code must compile on every target (watch for
dead code on targets that don't call it — clippy `-D warnings` runs
everywhere); keep platform parsers pure functions so they are
unit-testable everywhere. The memory-guard ceiling sources per platform
are specified in §21.19.

## Workflow

Porting and feature work follow the plans in `docs/superpowers/plans/`
against the behavioral spec in
`docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`. Build the
C++ oracle for live byte-compares with `scripts/build-oracle.sh` and set
`JOJODIFF_ORACLE=target/oracle`. Full details — setup, oracle layers,
golden policy, port anchors — in [CONTRIBUTING.md](CONTRIBUTING.md).
