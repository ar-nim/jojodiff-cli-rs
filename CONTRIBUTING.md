# Contributing

Thanks for your interest in contributing to `jojodiff-cli-rs`!

## AI coding agents

If you are an AI agent (Claude Code, Codex, Gemini CLI, GitHub Copilot,
Cursor, ZCode, ...): **[AGENTS.md](AGENTS.md) is the canonical, binding
instruction file** for this repository. `CLAUDE.md`, `GEMINI.md` and
`.github/copilot-instructions.md` are pointers that only resolve to it —
whatever harness you run under, read and follow `AGENTS.md` first.
Everything below is the human-oriented detail of the same workflow.

## Project goal

This project is a 1:1 port of JojoDiff 0.8.5. **Byte-exact compatibility with the
original C++ tools is the contract** — patch files, listings, verbose output, exit
codes, and even historical quirks (integer truncation semantics, typos in output
strings) must match the reference build. Do not "fix" behaviour that differs from the
C++ reference; deviations are allowed only where the specification lists them
explicitly (§20/§21 of `docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md`).

## Development setup

- Rust 1.85 or newer (`rustup update stable`); the project uses edition 2024.
- Runtime dependencies are `thiserror` (library error impls, derive-only),
  `anyhow` (the thin binary wrapper only) and `sysinfo` (the memory guard's
  platform ceiling, default-features off); no `unsafe`. Zero-dependency is
  not a dogma — new crates are fine when they replace hand-maintained
  platform code and get a §13 decision record. See the README's
  [Dependencies](README.md#dependencies) section and spec §13 for the record.
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

## Port anchors

`// port:<File>.cpp:<lines>` comments are verification metadata tying code
to the C++ source the goldens were generated from. They are optional —
new idiomatic code owes nothing — but when ported code moves, its anchors
travel with it (greppable via `rg '// port:'`).

## Do-not-fix markers

Behavioral quirks replicated on purpose are inventoried in the port spec
(`docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md` §21). If a
piece of code looks wrong, check §21 before "fixing" it — the goldens will
fail otherwise, by design.

## Refactoring rule

Green-to-green only: the full gate (`cargo fmt --all && cargo clippy
--all-targets --all-features --locked -- -D warnings && cargo test
--all-features`) passes before and after every transformation, one
transformation per commit. Goldens are never regenerated.

## Testing and the oracle

`cargo test --all-features` runs the oracle harness layers (see `tests/oracle.rs`):

1. Round-trip gate and francisdb cross-validation — always run.
2. Golden byte-compares against the committed 0.8.5-oracle outputs under
   `tests/fixtures/golden85/` — always run; the goldens are C++-oracle truth,
   never regenerate them from Rust output (`scripts/gen-golden.sh` rebuilds
   them from the oracle only).
3. Cross-version gate (`tests/crossver.rs`) — always run: the frozen 0.8.1
   goldens under `tests/fixtures/golden/` (never regenerated) must apply with
   `jdiff -u` and an argv[0]=`jptch` copy of the binary and restore byte-exact.
4. Live-oracle byte-compares against a compiled C++ 0.8.5 reference — skipped
   automatically unless `$JOJODIFF_ORACLE` points at a directory containing
   `jdiff`, or `target/oracle` exists. Build it locally with
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
