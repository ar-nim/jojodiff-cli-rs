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
> including dropping implicit-MOD bytes. See [Version history](#version-history) for what
> changed between the 0.8.1 and 0.8.5 lines.

## What is JojoDiff?

JojoDiff is a binary diff/patch tool: `jdiff` compares two files and writes a small patch
file, and `jdiff -u` applies that patch to the original file to reproduce the new file
byte-for-byte. It works on any data (no line-oriented assumptions), finds shifted and
repeated blocks via backtracking, and has no external dependencies.

This project ports the 0.8.5 algorithms **exactly**: patch files, listings, verbose
output and exit codes are verified byte-for-byte against a fixed build of the original
C++ source, which serves as the acceptance oracle.

## Relationship to other projects

`jojodiff-cli-rs` is an independent, complete Rust port of JojoDiff 0.8.5 by Joris
Heirbaut (GPLv3, https://sourceforge.net/projects/jojodiff/), which superseded the
project's earlier 0.8.1 port (via the v0.8.1 C++ class rewrite).

It is **not affiliated with, endorsed by, or derived from** the `jojodiff` crate /
[francisdb/jojodiff-rs](https://github.com/francisdb/jojodiff-rs) — a separate
MIT-licensed library that only *applies* patches. That crate is used solely as an
optional cross-validation consumer in this project's test suite. The original author's
GPLv3 work is credited as the source of the algorithm, wire format, and test data.

## Usage

```
jdiff [options] <original file> <new file> [<output file>]     # make a patch
jdiff -u [options] <original file> <patch file> [<output file>] # apply a patch
```

A missing output file or `-` means stdin/stdout. Options may appear anywhere on the
command line (GNU permutation); `--` ends option processing.

### Patch modes: `-u` and argv[0]

Upstream 0.8.5 merged the old standalone patcher into `jdiff`:

- **`jdiff -u`** (long form `--undiff`) is the upstream patch mode.
- Copies, hard links or symlinks of the binary whose name starts with **`jpatch`**
  patch via argv[0] dispatch (upstream behaviour, `main.cpp:303-315`).
- Names starting with **`jptch`** also patch via argv[0] — a **port extension**
  (upstream matches `jpatch` only), kept for 0.8.1 script compatibility.

### jdiff options

Actual behaviour is listed below (the C++ help text contains stale numbers that the port
replicates verbatim — e.g. it still says "-i (default 64)", "(in KB)" and "0=no
buffering"; the table gives the real values):

| Option | Effect |
|---|---|
| `-j` / `-u` | Force function: diff / patch |
| `-v`, `-vv`, `-vvv` | Verbose: greeting + results / + progress + statistics / + help and details |
| `-h`, `-hh` | Help; `-hh` adds notes and explanations |
| `-l` | List byte by byte (ASCII output) |
| `-r` | List regions (grouped bytes; was `-lr` in 0.8.1) |
| `-c` | Write verbose and debug info to stdout instead of stderr (was `-do`) |
| `-b`, `-bb`, ... | Better: use more memory, search more (multiplicative preset) |
| `-f`, `-ff`, ... | Lazy: only compare buffered data (often slower); `-ff` drops the full index |
| `-p` | Sequential source (auto-assumed for piped input) |
| `-q` | Sequential destination (auto-assumed for piped input) |
| `-s` | Use stdio files instead of iostreams (for testing; no size argument anymore) |
| `-a <KB>` | Size (in KB) to search ahead (default = buffer size) |
| `-i <MB>` | Index table size in MB (default 32; was `-s <size>`) |
| `-k <B>` | Block size in bytes for reading (default 32768; was `-bs`) |
| `-m <MB>` | Search buffers, MB in total, split evenly (default 2; `-m 0` = defaults; was KB) |
| `-n <count>` | Minimum number of matches to search (default 2; was `-min`) |
| `-x <count>` | Maximum number of matches to search (default 128; was `-max`) |
| `-d <name>` | Debug flag by name (`hsh ahd cmp prg buf hsk ahh bkt red mch dst`); needs the `debug` feature |

The index table size is always lowered to the nearest lower prime. Presets (`-b`, `-f`,
...) should be used before other options (parse order is part of the contract).

### Example

```
jdiff archive0000.tar archive0001.tar archive0001.jdf
jdiff -u archive0000.tar archive0001.jdf archive0001b.tar   # identical to archive0001.tar
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
jdiff -u archive0000.zip archive0001.jdf archive0001b.zip
unzip archive0001b.zip                    # restore mydir
```

`tar` + `gzip` (or any other archiver/compressor) works the same way.

## Patch-format compatibility (breaking change in 0.8.5)

Upstream 0.8.5 changed the patch wire format: MOD data runs are emitted **without** the
leading `ESC MOD` control pair ("implicit MOD"), and equal runs become `ESC EQL len`
from 3 bytes on (0.8.1 needed 5). Consequences, all verified in the test suite:

- **Patches produced by this port (≥0.8.5) are NOT applicable by 0.8.1-era patchers** —
  a 0.8.1 `jptch` silently drops the implicit-MOD data bytes and produces corrupt
  output. Regenerate patches when upgrading, or keep a 0.8.1 `jdiff` for old pairs.
- **0.8.1 patches still apply**: explicit opcodes are a subset of the 0.8.5 grammar.
  This one-way gate is pinned by `tests/crossver.rs`, which applies the entire committed
  0.8.1 golden corpus with `jdiff -u` and with an argv[0]=`jptch` copy of the binary.
- Listings (`-l`/`-r`) are diagnostic formats, not applied by patchers; they are not
  affected by compatibility concerns.

## The `-t` option is broken (upstream behaviour, ported faithfully)

`jdiff -t` (`--test`) is parsed but never used by upstream 0.8.5, so after diffing the
program feeds the **destination file** to the patch phase, appending misparsed data to
the already-written patch output (exit 0, corrupt file). The port replicates this
exactly — including in `debug` builds: the debug-only invariant assert upstream fires on
is unreachable in the port (fresh input re-open plus the EOF gate below), so debug and
release behave identically. Do not use `-t`; it exists for fidelity only.

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

Four layers run in `tests/`:

1. **Golden byte-compares** (everywhere): `scripts/gen-golden.sh` runs the compiled C++
   0.8.5 reference over the bundled corpus (`bkocomu.0000/0009.fil`,
   `test2.001/002.txt`) and the option matrix of spec §22.1, storing patches, listings
   and `-vv` stderr captures under `tests/fixtures/golden85/`. The Rust tool must
   reproduce those files byte-for-byte on every machine (one run-dependent statistics
   line is masked — see the script header).
2. **Cross-version gate** (everywhere): every committed **0.8.1** golden patch under
   `tests/fixtures/golden/` applies through `jdiff -u` and an argv[0]=`jptch` copy and
   restores byte-exact (spec §22.3).
3. **Round-trip gate** (everywhere): `jdiff OPTS A B p && jdiff -u A p out` restores B
   byte-exact for every corpus pair × option set, plus the francisdb-crate
   cross-validation.
4. **Live-oracle compares** (run when the reference is built): Rust and C++ binaries are
   compared directly across the whole matrix — `jdiff` outputs, verbose stderr, corpus
   listings (including the ~80 MB ASCII listing of the binary pair, intentionally not
   committed) and cross-applied patches. Skips silently without the oracle:

```
scripts/build-oracle.sh                          # build the C++ 0.8.5 reference (g++, make)
JOJODIFF_ORACLE=target/oracle cargo test --all-features
```

See [Oracle verification](#oracle-verification) for details. `scripts/runtest.sh
<original> <new>` is the cross-platform equivalent of the C++ `make runtest`
(`jdiff -m 0 A B p; jdiff -u A p out; cmp`).

## Port fidelity

The port is a 1:1 translation of the 0.8.5 C++ sources; the contract is byte-exact
output — including historical quirks such as the `disbale` typo in the verbose echo and
the stale "-i (default 64)" help texts. The deviations are catalogued exhaustively in
[spec §20/§21](docs/superpowers/specs/2026-10-01-jojodiff-1to1-port-spec.md); the
user-visible ones:

- **EOF gate (the one deliberate non-replication, §21.17):** upstream 0.8.5 can read at
  a negative file position during patching, serves a stale buffer byte forever and
  **hangs at 100% CPU**; the port gates negative positions to EOF so affected patches
  terminate cleanly instead of hanging.
- **Deterministic counters (§21.5/§21.6):** the C++ leaves the index-hit/repair and
  inaccurate-solution counters uninitialized (its verbose output prints garbage that
  changes between runs); the port zero-initializes them and prints the real counts.
- **Index-table divisor (§18.E/§21.7):** the port keeps the stock-LP64 16-byte element
  size, so the same `-i <MB>` builds a smaller table than the 32-bit-`hkey` oracle
  build; the gates compare at element-equal sizes (`-i 8` port ≡ `-i 6` oracle).
- **Single binary (§21.2):** `jpatch`/`jptch` argv[0] routes replace the removed
  `jpatch.cpp`/`jptch` binaries; `jptch` matching is the port extension.
- **Not ported (compiled out upstream):** dedup (`-y`/`--dedup` exits 20 instead of
  crashing like upstream) and the `jdedup`/`jtst` argv[0] routes.

## Oracle verification

`tests/oracle.rs` implements the acceptance gates of spec §22 (see
[Testing](#testing) for the layer overview). To build the reference locally:

```
scripts/build-oracle.sh
```

copies the pristine vendored tree (`reference/jojodiff-0.8.5`) to `target/oracle-src85/`,
applies exactly the source patch that defines the canonical verification build (the
§21.7 32-bit `hkey` typedef) and compiles it with `-D_FILE_OFFSET_BITS=64`
(`JDIFF_LARGEFILE` live); the binary lands in `target/oracle/jdiff`. Tests find it via
`$JOJODIFF_ORACLE` or that default location; without it, layer 4 skips and everything
else still runs. CI runs the live compare in a dedicated ubuntu job on every push.

`tests/fixtures/golden85/` (0.8.5, regenerated by `scripts/gen-golden.sh`) and
`tests/fixtures/golden/` (0.8.1, frozen) are both oracle truth: never regenerate them
from Rust output.

## Version history

### 0.8.5 re-target (this port)

The port began as a byte-exact 0.8.1 port and was re-targeted to the 0.8.5 C++ sources,
adopting every upstream change from the 0.8.1 → 0.8.5 window (summarized from the
upstream changelog, `main.cpp:117-149`):

- **v0.8.2** — `jfopen`/`jfclose`/`jfread`/`jfseek` wrappers against LARGEFILE
  redefinition clashes; virtual destructors for `JFile`/`JOut`.
- **v0.8.3a-z** — `getopt_long` option processing (GNU permutation, long options, new
  option names); index table sized in MB and lowered to the nearest prime; improved
  progress feedback; equal-run counter mixed into the hash for quality; dynamic matching
  table (new/old lists replace the freelist); `-` standard-input support; `-s` stdio
  backend; `jpatch` integrated as `JFile.getbuf` client (one binary).
- **v0.8.4b-c** — hash re-initialization for incremental scanning.
- **v0.8.5a-ca** — rewritten sequential-file buffer logic; unbuffered istream
  implementation removed; experimental (and upstream-disabled) deduplication; fewer
  compares via cached negative results; improved gliding-match detection; incremental
  search; accuracy work for non-compared matches (`-f`/`-p`) and incremental scanning
  (`-ff`); `isOld` tuning.

Net user-visible effects (all ported and oracle-verified): a single `jdiff` binary with
`-u`/argv[0] patching, the new (breaking) patch wire format, the rewritten CLI surface,
`-` for any file argument including piped patches, and swapped exit codes for
differences (1) vs identical (0).

### 0.8.1 port (superseded)

The original scope of this project — a byte-exact port of the 0.8.1 C++ class rewrite,
complete with its own oracle and goldens (now frozen under `tests/fixtures/golden/` and
`tests/crossver.rs`).

## Roadmap

- [x] Repository scaffolding, port plan and functional specification
- [x] Library: readers, hash table, match table, output writers, diff engine
- [x] `jdiff` CLI (single binary; `-u` and argv[0] patch modes)
- [x] Debug feature (`-d <name>` flags, parity with the `_DEBUG` builds)
- [x] 0.8.5 re-target: engine, writers, CLI, oracle, goldens, cross-version gate

## License

GPL-3.0-or-later, matching the original JojoDiff from which this project is derived.
See [LICENSE](LICENSE).

Credits:

- **Joris Heirbaut** — author of the original JojoDiff (algorithm, wire format, test data).
- The v0.8.5 C++ sources (and the earlier v0.8.1 class rewrite), used as the porting
  reference and verification oracle.
