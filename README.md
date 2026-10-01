# jojodiff-cli-rs

[![ci](https://github.com/ar-nim/jojodiff-cli-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ar-nim/jojodiff-cli-rs/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

An independent Rust port of [JojoDiff](https://sourceforge.net/projects/jojodiff/) 0.8.1 by
Joris Heirbaut — byte-compatible `jdiff` and `jptch` binary diff/patch command-line tools
for Windows, Linux and macOS, plus a reusable library.

> **Status: early development.** The port follows a detailed 1:1 translation plan
> ([docs/superpowers/plans/2026-10-01-jojodiff-rs-port.md](docs/superpowers/plans/2026-10-01-jojodiff-rs-port.md))
> driven by a [functional specification](docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md)
> of the original C++ sources. Until the first release, the interfaces below are the
> compatibility targets, not shipping features.

## What is JojoDiff?

JojoDiff is a binary diff/patch pair: `jdiff` compares two files and writes a small
patch file; `jptch` applies that patch to the original file to reproduce the new file
byte-for-byte. It works on any data (no line-oriented assumptions), finds shifted and
repeated blocks via backtracking, and has no external dependencies.

This project reimplements the 0.8.1 algorithms **exactly**: patch files, listings,
verbose output and exit codes are verified byte-for-byte against a fixed build of the
original C++ source, which serves as the acceptance oracle.

## Relationship to other projects

`jojodiff-cli-rs` is an independent, complete Rust port of JojoDiff 0.8.1 by Joris
Heirbaut (GPLv3, https://sourceforge.net/projects/jojodiff/), via the v0.8.1 C++ class
rewrite.

It is **not affiliated with, endorsed by, or derived from** the `jojodiff` crate /
[francisdb/jojodiff-rs](https://github.com/francisdb/jojodiff-rs) — a separate
MIT-licensed library that only *applies* patches. That crate is used solely as an
optional cross-validation consumer in this project's test suite. The original author's
GPLv3 work is credited as the source of the algorithm, wire format, and test data.

## Usage (planned interface)

```
jdiff [options] <original file> <new file> [<output file>]
jptch [options] <original file> <patch file> [<output file>]
```

A missing output file or `-` means stdin/stdout. Common options:

| Option | Effect |
|---|---|
| `-b` / `-f` / `-ff` | Presets: best / fast / fastest processing behaviour |
| `-l` / `-lr` | Write an ASCII listing / region listing instead of a patch |
| `-m size` | Look-ahead buffer size in kB (0 = whole-file in-memory mode) |
| `-v`, `-vv`, `-vvv` | Verbose output |
| `-h` | Help |

The full option set and byte-exact help text mirror JojoDiff 0.8.1; see the
[specification](docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md) for
details.

## Building

Rust 1.85 or newer (edition 2024); no runtime dependencies beyond `std`:

```
cargo build --release
```

produces `target/release/jdiff` and `target/release/jptch`.

## Testing

```
cargo test
```

The conformance harness (added with the port's test tasks) additionally compares this
implementation against the original C++ build when `JOJODIFF_ORACLE` points at a
directory containing compiled reference `jdiff`/`jptch` binaries.

## Roadmap

- [x] Repository scaffolding, port plan and functional specification
- [ ] Library: readers, hash table, match table, output writers, diff engine
- [ ] `jdiff` and `jptch` CLIs
- [ ] Debug feature (`-d*` flags, parity with the `_DEBUG` builds)
- [ ] Oracle conformance harness, golden fixtures, full documentation

## License

GPL-3.0-or-later, matching the original JojoDiff from which this project is derived.
See [LICENSE](LICENSE).

Credits:

- **Joris Heirbaut** — author of the original JojoDiff (algorithm, wire format, test data).
- The v0.8.1 C++ class rewrite of JojoDiff, used as the porting reference and
  verification oracle.
