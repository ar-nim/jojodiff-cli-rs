# Contributing

Thanks for your interest in contributing to `jojodiff-cli-rs`!

## Project goal

This project is a 1:1 port of JojoDiff 0.8.1. **Byte-exact compatibility with the
original C++ tools is the contract** — patch files, listings, verbose output, exit
codes, and even historical quirks (integer truncation semantics, typos in output
strings) must match the reference build. Do not "fix" behaviour that differs from the
C++ reference; deviations are allowed only where the specification lists them
explicitly (§15 of `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`).

## Development setup

- Rust 1.85 or newer (`rustup update stable`); the project uses edition 2024.
- The library is std-only: no new runtime dependencies, no `unsafe`.
- Before committing:
  ```
  cargo fmt --all
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  ```
  CI runs exactly these checks on Linux, Windows and macOS.

## Workflow

Porting work is organized as a test-driven task list in
`docs/superpowers/plans/2026-10-01-jojodiff-rs-port.md`, with the behavioural
specification in `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`.
Each task follows the same cycle: write failing tests from the spec, port the
C++ module, make the tests pass, then commit.

The conformance tests skip automatically unless `JOJODIFF_ORACLE` points at a
directory containing a compiled C++ reference build, so they run in CI's dedicated
oracle job but stay inert on developer machines without it.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) —
e.g. `feat: JHashPos sample hashtable (port of JHashPos.cpp)`, `fix: ...`,
`docs: ...`, `ci: ...`.
