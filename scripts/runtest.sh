#!/usr/bin/env bash
#
# Round-trip smoke test with the Rust binaries — cross-platform port of the
# C++ Makefile's `runtest` target and run.sh:
#
#     jdiff -m 0 <original> <new> > <patch>
#     jptch <original> <patch> > patched_version
#     compare <new> patched_version        (md5sum in the original; cmp here)
#
# Usage: scripts/runtest.sh <original> <new> [patch-output]
#
# Binaries are taken from $JDIFF / $JPTCH if set, else target/release
# (build them first with `cargo build --release`).
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <original> <new> [patch-output]" >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JDIFF="${JDIFF:-$ROOT/target/release/jdiff}"
JPTCH="${JPTCH:-$ROOT/target/release/jptch}"

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
for exe in "$JDIFF" "$JPTCH"; do
    if [ ! -x "$exe" ]; then
        echo "runtest: binary not found: $exe (run cargo build --release)" >&2
        exit 1
    fi
done

echo "Diffing..."
time "$JDIFF" -m 0 "$TEST1" "$TEST2" > "$OUT_FILE"
echo
echo "Patching..."
"$JPTCH" "$TEST1" "$OUT_FILE" > "$PATCHED"
echo
echo "Verifying desired and resulted file:"
cmp "$TEST2" "$PATCHED" && echo "OK: files are identical"
rm -f "$PATCHED"
