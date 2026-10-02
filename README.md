# jojodiff-cli-rs

[![ci](https://github.com/ar-nim/jojodiff-cli-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ar-nim/jojodiff-cli-rs/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

An independent Rust port of [JojoDiff](https://sourceforge.net/projects/jojodiff/) by
Joris Heirbaut — byte-compatible binary diff/patch command-line tools for Windows, Linux
and macOS, plus a reusable library. The port started from 0.8.1 and is re-targeted to
**0.8.5**, which ships a **single `jdiff` binary**: patching is `jdiff -u`, and copies,
links or aliases of the binary named `jpatch`/`jptch` patch via argv[0] dispatch.

> **Status: beta** — the port is complete and matches JojoDiff 0.8.5. Every patch byte,
> listing, verbose line and exit code produced by this implementation is verified against
> the original C++ source compiled as a fixed oracle build (see
> [Oracle verification](#oracle-verification)). Bugs are fidelity bugs; report them as such.
>
> **Migrating from the 0.8.1 package:** after `cargo install` only `jdiff` exists —
> existing `jptch` scripts migrate with a one-time `ln -s jdiff jptch` (or a shell alias,
> or calling `jdiff -u`). Remove or replace any stale `jptch` binary left in `~/.cargo/bin`
> by the 0.8.1 install: a leftover 0.8.1 `jptch` would silently keep 0.8.1 semantics,
> including dropping implicit-MOD bytes.

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

## Usage

```
jdiff [options] <original file> <new file> [<output file>]
jptch [options] <original file> <patch file> [<output file>]
```

A missing output file or `-` means stdin/stdout.

### jdiff options

| Option | Effect |
|---|---|
| `-v` | Verbose (greeting, results and tips) |
| `-vv` | Verbose (debug info) |
| `-h` | Help (this text) |
| `-l` | List byte by byte (ascii output) |
| `-lr` | List groups of bytes (ascii output) |
| `-b` | Try to be better (using more memory) |
| `-f` | Try to be faster: using less memory, no out of buffer compares |
| `-ff` | Try to be faster: no out of buffer compares, no prescanning |
| `-m size` | Size (in kB) for look-ahead buffer (default 256, 0 = no buffers / in-memory) |
| `-bs size` | Block size (in bytes) for reading from files (default 4096) |
| `-s size` | Number of samples in mega (default 8 mega samples) |
| `-a size` | Number of kB to look ahead (default = same as buffer-size) |
| `-min count` | Minimum number of solutions to find |
| `-max count` | Maximum number of solutions to find |
| `-do` | Write verbose and debug info to stdout instead of stderr |

`-min`/`-max` defaults are 8/32; the presets assign their own values. Options `-b`,
`-f` or `-ff` should be used before other options (parse order is part of the
contract). Sample size is always lowered to the largest n-bit prime (n < 32).

### jptch options

| Option | Effect |
|---|---|
| `-v` | Verbose: version and licence |
| `-vv` | Verbose: debug info |
| `-vvv` | Verbose: more debug info |
| `-t` | Test: no output file |
| `-d` | Write debug info to stdout |
| `-h` | Help (this text) |

### Example

```
jdiff archive0000.tar archive0001.tar archive0001.jdf
jptch archive0000.tar archive0001.jdf archive0001b.tar   # identical to archive0001.tar
```

Typical applications are incremental backups and synchronising files over slow
networks. `jdiff` does not compress the patch file; compress it yourself. Do **not**
diff compressed files — diff uncompressed containers and compress afterwards:

```
zip -0 archive0000.zip mydir/*            # put mydir in an archive
zip -0 archive0001.zip mydir/*            # some time later
jdiff archive0000.zip archive0001.zip archive0001.jdf
zip -9 archive0001.jdf.zip archive0001.jdf  # compress the patch for transfer
# ... later:
unzip archive0001.jdf.zip
jptch archive0000.zip archive0001.jdf archive0001b.zip
unzip archive0001b.zip                    # restore mydir
```

`tar` + `gzip` (or any other archiver/compressor) works the same way.

## Building

Rust 1.85 or newer (edition 2024); no runtime dependencies beyond `std`:

```
cargo build --release
```

produces `target/release/jdiff` (0.8.5 ships one binary; `jpatch`/`jptch` are
argv[0] aliases — see the migration note at the top).

For regular use, install the binary onto your `PATH` (default `~/.cargo/bin`):

```
cargo install --path .
```

The library crate (`jojodiff_cli_rs`) exposes the same building blocks as the C++
classes (readers, hash table, match table, output writers, diff engine). With the
non-default `debug` feature, the `-d*` diagnostic flags of the `_DEBUG` builds are
available too.

## Testing

```
cargo test --all-features
```

Three layers run everywhere: the round-trip gate over the bundled test corpus, golden
byte-compares against oracle outputs committed under `tests/fixtures/golden/`, and
cross-validation with the third-party `jojodiff` crate. A fourth layer compares the
Rust tools directly against a compiled C++ reference whenever one is present, and
skips silently otherwise:

```
scripts/build-oracle.sh                          # build the C++ reference (g++, make)
JOJODIFF_ORACLE=target/oracle cargo test --all-features
```

See [Oracle verification](#oracle-verification) for details. `scripts/runtest.sh
<original> <new>` is the cross-platform equivalent of the C++ `make runtest`
(`jdiff -m 0 A B > p; jptch A p > patched; cmp`).

## Port fidelity

The port is a 1:1 translation of the 0.8.1 C++ sources; the contract is byte-exact
output — including historical quirks such as the `Hastable` typo in verbose output.
The only behavioural deltas are the documented deviations in
[spec §15](docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md):

- **§15.1** — the Linux ifstream pre-read defect is fixed as in the spec'd 2-line
  patch (or the MinGW build); the port opens files directly.
- **§15.2** — the never-used OpenMP `make parallel` target (a data race) is not
  ported; the port is serial and deterministic.
- **§15.3** — the in-memory reader (`-m 0`/`-m 1`) serves full content; the C++
  `istringstream` construction truncates at NUL bytes (and overflows on long input).
- **§15.4** — `jptch -` reads stdin fully instead of seeking (strictly more
  permissive; under verbose, the out-position prints −1 on non-seekable stdout).
- **§15.5** — MinGW ifdefs are collapsed into one uniform implementation.
- **§15.6** — `JFileAhead` and `JFileIStreamAhead` (byte-identical twins in C++) are
  one implementation.
- **§15.7** — known francisdb/jojodiff-rs decoder edge cases are not copied; the port
  follows C++ everywhere.
- **§15.8** — the dead scroll-back mode 2 (collapsed by a `bool` seek flag) is
  replicated as dead-code parity.
- **§15.9** — the dead `lbFnd < 0` error check (`bool` forcing) is replicated as
  control flow; errors surface via the final EOB min-check.
- **§15.10** — the stale-failbit reader defect of C++ `JFileIStream` is not ported.
- **§15.11** — unopenable `jdiff` inputs produce the documented open-check messages
  and exit codes (the stock Linux oracle aborts there; `jptch` open checks are live
  in C++ and are oracle-faithful).
- **§15.12** — the port is the 32-bit `hkey` variant (SMPSZE = 32, Windows/x86
  semantics); the Linux oracle build forces the same width.

## Oracle verification

`tests/oracle.rs` implements the acceptance gates of spec §16:

1. **Golden byte-compares** (always run): `scripts/gen-golden.sh` runs the compiled
   C++ reference over the bundled corpus (`bkocomu.0000/0009.fil`,
   `test2.001/002.txt`) and the option matrix of spec §13, storing patches, listings
   and verbose stderr captures under `tests/fixtures/golden/`. The Rust tools must
   reproduce those files byte-for-byte on every machine.
2. **Round-trip gate** (always run): `jdiff A B p && jptch A p out` restores B
   byte-exact for every corpus pair × option set.
3. **Cross-validation** (always run): the `jojodiff` crate (francisdb) applies the
   Rust-produced patches of the text pair.
4. **Live-oracle compares** (run when the reference is built): Rust and C++ binaries
   are compared directly across the whole matrix — `jdiff` outputs, verbose stderr,
   and `jptch` outputs for every golden patch — including the ~80 MB ASCII listing of
   the binary pair, which is intentionally not committed as a golden.

To build the reference locally:

```
scripts/build-oracle.sh
```

copies the pristine vendored tree (`reference/jojodiff-cpp`) to `target/oracle-src/`,
applies exactly the two source patches that define the canonical verification build
(spec §15.1 ifstream fix, §15.12 32-bit `hkey`), compiles it and places the binaries
in `target/oracle/`. Tests find them via `$JOJODIFF_ORACLE` or that default location;
without them, layer 4 skips and everything else still runs. CI runs the live compare
in a dedicated ubuntu job on every push.

## Roadmap

- [x] Repository scaffolding, port plan and functional specification
- [x] Library: readers, hash table, match table, output writers, diff engine
- [x] `jdiff` and `jptch` CLIs
- [x] Debug feature (`-d*` flags, parity with the `_DEBUG` builds)
- [x] Oracle conformance harness, golden fixtures, full documentation

## License

GPL-3.0-or-later, matching the original JojoDiff from which this project is derived.
See [LICENSE](LICENSE).

Credits:

- **Joris Heirbaut** — author of the original JojoDiff (algorithm, wire format, test data).
- The v0.8.1 C++ class rewrite of JojoDiff, used as the porting reference and
  verification oracle.
