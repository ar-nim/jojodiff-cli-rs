#!/usr/bin/env bash
#
# Round-trip smoke test with the Rust binary — cross-platform port of the
# C++ Makefile's `runtest` target and run.sh:
#
#     jdiff -m 0 <original> <new> <patch>
#     jdiff -u <original> <patch> <patched_version>
#     compare <new> <patched_version>      (md5sum in the original; cmp here)
#
# One binary (0.8.5 shape): patching is `jdiff -u` — or a copy/link of the
# binary named `jpatch`/`jptch`, see README ("Patch modes: -u and argv[0]").
#
# Usage: scripts/runtest.sh <original> <new> [patch-output]
#
# The binary is taken from $JDIFF if set, else target/release
# (build it first with `cargo build --release`).
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <original> <new> [patch-output]" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JDIFF="${JDIFF:-$ROOT/target/release/jdiff}"

TEST1="$1"
TEST2="$2"
OUT_FILE="${3:-patch.jdf}"
PATCHED="patched_version"

for f in "$TEST1" "$TEST2"; do
    if [ ! -f "$f" ]; then
        echo "runtest: file not found: $f" >&2
        exit 1
    fi
done
if [ ! -x "$JDIFF" ]; then
    echo "runtest: binary not found: $JDIFF (run cargo build --release)" >&2
    exit 1
fi

echo "Diffing..."
time "$JDIFF" -m 0 "$TEST1" "$TEST2" "$OUT_FILE"
echo
echo "Patching..."
"$JDIFF" -u "$TEST1" "$OUT_FILE" "$PATCHED"
echo
echo "Verifying desired and resulted file:"
cmp "$TEST2" "$PATCHED" && echo "OK: files are identical"
rm -f "$PATCHED"
