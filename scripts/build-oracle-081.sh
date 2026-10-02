#!/usr/bin/env bash
#
# Build the LEGACY JojoDiff 0.8.1 C++ reference oracle (reference/jojodiff-cpp).
#
# Since the 0.8.5 re-target (plan Task 13) the active oracle is built by
# scripts/build-oracle.sh from reference/jojodiff-0.8.5; this script is kept
# for 0.8.1-era comparisons — notably the optional --compat-081 acceptance
# gate (tests/oracle.rs, $JOJODIFF_ORACLE_081). The committed 0.8.1 goldens
# (tests/fixtures/golden/) are frozen and never regenerated (there is no
# gen-golden equivalent for them anymore).
#
# Writes to target/oracle081/ — NOT target/oracle/, which holds the 0.8.5
# oracle the suite uses (gen-golden.sh, tests/oracle.rs).
#
# Copies the pristine vendored tree (reference/jojodiff-cpp) to target/oracle-src/,
# applies exactly the two patches that make the Linux build the canonical
# verification build of the port (spec §2, §15.1, §15.12), compiles it and places
# the binaries in target/oracle081/{jdiff,jptch}.
#
# The applied patches (and nothing else):
#   1. src/main.cpp: re-position both ifstreams after the pthread pre-read
#      (`liFilOrg->clear(); liFilOrg->seekg(0);` and the same for the new file)
#      — without it the stock Linux build serves EOF-state streams and emits
#      empty patches (spec §15.1).
#   2. headers/JDefs.h: `typedef unsigned long int hkey` → `typedef unsigned int
#      hkey` — on LP64 Linux `unsigned long` is 64-bit (SMPSZE=64) while the
#      port's target variant is the 32-bit hkey build (Windows/x86 semantics,
#      spec §2 and §15.12).
#
# Idempotent: the source tree is re-copied pristine on every run, so patches
# always apply to virgin files. The C++ tree under reference/ is never modified.
#
# Usage: scripts/build-oracle-081.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/reference/jojodiff-cpp"
DST="$ROOT/target/oracle-src"
OUT="$ROOT/target/oracle081"

for tool in g++ make python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "build-oracle: required tool '$tool' not found" >&2
        exit 1
    fi
done

if [ ! -f "$SRC/Makefile" ]; then
    echo "build-oracle: vendored C++ tree not found at $SRC" >&2
    exit 1
fi

echo "build-oracle: copying pristine tree $SRC -> $DST"
rm -rf "$DST"
mkdir -p "$DST"
cp -a "$SRC/." "$DST/"

echo "build-oracle: applying spec §15.1 + §15.12 patches"
python3 - "$DST" <<'PY'
import sys

dst = sys.argv[1]

def patch(path, old, new, what):
    with open(path, "r", encoding="utf-8", newline="") as f:
        text = f.read()
    if text.count(old) != 1:
        sys.exit(f"build-oracle: {what}: anchor found {text.count(old)} times (expected 1)")
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text.replace(old, new))
    print(f"build-oracle: patched {what}")

# 1. spec §15.1: re-position both ifstreams after the pthread pre-read.
main_cpp = dst + "/src/main.cpp"
anchor = (
    '  rc = pthread_join(threadOrg, NULL);\n'
    '  if(rc) {\n'
    '\t\tfprintf(stderr, "error joining threads\\n");\n'
    '\t\treturn -1;\n'
    '\t}\n'
)
fix = (
    anchor
    + '\n'
    + '  liFilOrg->clear(); liFilOrg->seekg(0);\n'
    + '  liFilNew->clear(); liFilNew->seekg(0);\n'
)
patch(main_cpp, anchor, fix, "main.cpp (§15.1 ifstream re-position)")

# 2. spec §2/§15.12: force 32-bit hkey (port's target variant, SMPSZE=32).
jdefs_h = dst + "/headers/JDefs.h"
patch(
    jdefs_h,
    "typedef unsigned long int hkey ;",
    "typedef unsigned int hkey ;",
    "JDefs.h (§15.12 32-bit hkey)",
)
PY

# The Makefile does not track main.cpp changes; the tree is fresh anyway,
# but remove stale products for belt and braces (plan Task 12 step 1).
rm -f "$DST/jdiff" "$DST/jptch" "$DST"/bin/*.o

echo "build-oracle: make"
make -C "$DST" all

mkdir -p "$OUT"
cp "$DST/jdiff" "$DST/jptch" "$OUT/"

# Sanity check (mirrors build-oracle.sh): the greeting must be the 0.8.1 one,
# and the §15.1 fix must yield a non-empty patch on the bundled text pair
# (the stock unpatched Linux build emits a 0-byte patch here).
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
printf 'hello world hello' > "$TMP/a.bin"
printf 'hello world byebye' > "$TMP/b.bin"
"$OUT/jdiff" -v "$TMP/a.bin" "$TMP/b.bin" "$TMP/sanity.jdf" \
    > "$TMP/sanity.out" 2>&1 || true
if ! grep -Fq "0.8.1 (beta) December 2011" "$TMP/sanity.out"; then
    echo "build-oracle: sanity check failed - '0.8.1 (beta) December 2011' missing from jdiff -v output:" >&2
    cat "$TMP/sanity.out" >&2
    exit 1
fi
if [ ! -s "$TMP/sanity.jdf" ]; then
    echo "build-oracle: sanity check failed - empty patch on text pair (§15.1 fix not effective?)" >&2
    exit 1
fi

echo "build-oracle: 0.8.1 oracle ready at $OUT/jdiff and $OUT/jptch (0.8.5 oracle stays at target/oracle)"
