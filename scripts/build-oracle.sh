#!/usr/bin/env bash
#
# Build the JojoDiff 0.8.5 C++ reference oracle (spec §21.7/§21.8).
#
# Copies the pristine vendored tree (reference/jojodiff-0.8.5) to
# target/oracle-src85/, applies exactly the one patch that aligns it with the
# port's target variant, compiles it and places the binary in
# target/oracle/jdiff. 0.8.5 ships a single binary — there is NO jptch
# (patching is `jdiff -u` / argv[0] dispatch, spec §18.B).
#
# The applied patch (and nothing else):
#   1. src/JDefs.h: `typedef unsigned long int hkey` → `typedef unsigned int
#      hkey` — on LP64 Linux `unsigned long` is 64-bit (SMPSZE=64) while the
#      port's target variant is the 32-bit hkey build (Windows/x86 semantics;
#      spec §21.7 with §15.12).
#
# Build flags: `-D_FILE_OFFSET_BITS=64` is added to CFLAGS so JDIFF_LARGEFILE
# is live (the shipped Makefile omits the define; spec §21.7) — CFLAGS are
# overridden on the make command line. `make clean` runs first because the
# Makefile does NOT rebuild its objects when $(DBG)/CFLAGS change (spec §21.8).
#
# Idempotent: the source tree is re-copied pristine on every run, so the
# patch always applies to a virgin file. The C++ tree under reference/ is
# never modified. (The 0.8.5 Makefile's stale `jpatch`/`jpatcd` targets
# reference the deleted jpatch.cpp; they are not invoked — spec §21.8.)
#
# Usage: scripts/build-oracle.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/reference/jojodiff-0.8.5"
DST="$ROOT/target/oracle-src85"
OUT="$ROOT/target/oracle"

for tool in g++ make python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "build-oracle: required tool '$tool' not found" >&2
        exit 1
    fi
done

if [ ! -f "$SRC/src/Makefile" ]; then
    echo "build-oracle: vendored C++ tree not found at $SRC" >&2
    exit 1
fi

echo "build-oracle: copying pristine tree $SRC -> $DST"
rm -rf "$DST"
mkdir -p "$DST"
cp -a "$SRC/." "$DST/"

echo "build-oracle: applying spec §21.7/§15.12 hkey patch"
python3 - "$DST" <<'PY'
import sys

dst = sys.argv[1]
path = dst + "/src/JDefs.h"
with open(path, "r", encoding="utf-8", newline="") as f:
    text = f.read()
old = "typedef unsigned long int hkey ;"
if text.count(old) != 1:
    sys.exit(f"build-oracle: hkey anchor found {text.count(old)} times (expected 1)")
with open(path, "w", encoding="utf-8", newline="") as f:
    f.write(text.replace(old, "typedef unsigned int hkey ;"))
print("build-oracle: patched JDefs.h (§21.7/§15.12 32-bit hkey)")
PY

echo "build-oracle: make clean (Makefile does not track DBG/CFLAGS changes, §21.8)"
make -C "$DST/src" clean

echo "build-oracle: make with -D_FILE_OFFSET_BITS=64 (JDIFF_LARGEFILE live, §21.7)"
make -C "$DST/src" all CFLAGS="-m64 -O2 -Wall -D_FILE_OFFSET_BITS=64"

mkdir -p "$OUT"
# Remove stale products (e.g. a jptch left by an earlier 0.8.1
# build-oracle-081.sh run): a 0.8.5 jdiff next to a 0.8.1 jptch would hand
# gen-golden.sh / the oracle harness a silently mixed oracle.
rm -f "$OUT/jdiff" "$OUT/jptch"
cp "$DST/src/jdiff" "$OUT/jdiff"

# Sanity check (spec §21.7): the greeting must be the 0.8.5 one and the hkey
# patch must be effective — SMPSZE = sizeof(hkey) * 8 is 32 only when the
# typedef was replaced (an unpatched LP64 build prints "samples are 64 bytes").
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
printf 'hello world hello' > "$TMP/a.bin"
printf 'hello world byebye' > "$TMP/b.bin"
"$OUT/jdiff" -v "$TMP/a.bin" "$TMP/b.bin" "$TMP/sanity.jdf" \
    > "$TMP/sanity.out" 2>&1 || true
for want in "0.8.5 (beta) 2020" "samples are 32 bytes"; do
    if ! grep -Fq "$want" "$TMP/sanity.out"; then
        echo "build-oracle: sanity check failed - '$want' missing from jdiff -v output:" >&2
        cat "$TMP/sanity.out" >&2
        exit 1
    fi
done

echo "build-oracle: oracle ready at $OUT/jdiff (0.8.5 has no jptch binary)"
