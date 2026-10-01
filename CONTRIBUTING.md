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

## Testing and the oracle

`cargo test --all-features` runs the oracle harness layers (see `tests/oracle.rs`):

1. Round-trip gate and francisdb cross-validation — always run.
2. Golden byte-compares against the committed oracle outputs under
   `tests/fixtures/golden/` — always run; the goldens are C++-oracle truth,
   never regenerate them from Rust output (`scripts/gen-golden.sh`).
3. Live-oracle byte-compares against a compiled C++ reference — skipped
   automatically unless `$JOJODIFF_ORACLE` points at a directory containing
   `jdiff`/`jptch`, or `target/oracle` exists. Build it locally with
   `scripts/build-oracle.sh` (needs g++ and make; Linux/WSL).

```
scripts/build-oracle.sh
JOJODIFF_ORACLE=target/oracle cargo test --all-features
```

CI runs the live compare in a dedicated ubuntu job; the other layers run on
every OS. Byte-exact compatibility is the contract: if a live comparison
fails, that is a port bug or an oracle/golden mismatch — investigate, never
adjust goldens to fit Rust output.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) —
e.g. `feat: JHashPos sample hashtable (port of JHashPos.cpp)`, `fix: ...`,
`docs: ...`, `ci: ...`.
