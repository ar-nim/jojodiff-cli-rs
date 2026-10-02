#!/usr/bin/env bash
#
# Generate the committed golden fixtures for tests/oracle.rs by running the
# C++ reference oracle (spec §13 test corpus + option matrix, §16.2).
#
# Requires the LEGACY 0.8.1 oracle built by scripts/build-oracle-081.sh
# (either point JOJODIFF_ORACLE at its directory or leave target/oracle in
# place); since the 0.8.5 re-target (plan Task 13) the active oracle built by
# scripts/build-oracle.sh has no jptch binary and produces 0.8.5 bytes.
#
# Goldens are written to tests/fixtures/golden/<pair>/<optset>.<ext>:
#   <optset>.jdf         binary patch          (default,f,ff,b,s1,s32,bs512,
#                                              m64,min1max1,a16 on both pairs;
#                                              m0,m1 on the text pair)
#   <optset>.asc         ASCII listing         (-l)
#   <optset>.rgn         region listing        (-lr)
#   <optset>.v.stderr    jdiff -v  stderr      (text pair, patch sets)
#   <optset>.vv.stderr   jdiff -vv stderr      (text pair, patch sets)
#
# Deliberate restrictions (controller rulings; see tests/oracle.rs):
#   * m0/m1 (in-memory mode) are generated for the NUL-free text pair only:
#     the C++ in-memory reader truncates at NUL bytes (spec §15.3), so on the
#     NUL pair it would report the files identical (3-byte patch, exit 1).
#   * bkocomu/l.asc (the ASCII listing of the binary pair) is ~80 MB and is
#     NOT written: committing it would bloat every clone. That single matrix
#     entry is verified by the live-oracle tests instead (Rust vs C++
#     directly), which the CI oracle job runs on every push.
#
# The goldens are oracle truth, committed to the repository; never regenerate
# them from Rust output.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOLDEN="$ROOT/tests/fixtures/golden"

# Locate the oracle (env override, else the build-oracle-081.sh output location).
ORACLE_DIR="${JOJODIFF_ORACLE:-$ROOT/target/oracle}"
JDIFF="$ORACLE_DIR/jdiff"
if [ ! -x "$JDIFF" ] || [ ! -x "$ORACLE_DIR/jptch" ]; then
    echo "gen-golden: 0.8.1 oracle (jdiff + jptch) not found at $ORACLE_DIR - run scripts/build-oracle-081.sh first" >&2
    exit 1
fi

# Option matrix of spec §13 run on BOTH pairs: name | jdiff options | output
# extension. (In-memory mode and the oversized listing are handled below.)
OPTSET_NAMES=(default f ff b s1 s32 bs512 m64 min1max1 a16 l lr)
OPTSET_ARGS=("" "-f" "-ff" "-b" "-s 1" "-s 32" "-bs 512" "-m 64" \
             "-min 1 -max 1" "-a 16" "-l" "-lr")
OPTSET_EXTS=(jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf asc rgn)

# In-memory mode: text pair only (NUL-free; C++ -m 0/1 is broken on NUL data).
TEXT_ONLY_NAMES=(m0 m1)
TEXT_ONLY_ARGS=("-m 0" "-m 1")

# Corpus pairs: name | original | new.
PAIR_NAMES=(bkocomu test2)
PAIR_ORGS=(bkocomu.0000.fil test2.001.txt)
PAIR_NEW=(bkocomu.0009.fil test2.002.txt)

gen_one() { # <org> <new> <outdir> <optset-name> <opts> <ext>
    local org="$1" new="$2" outdir="$3" name="$4" opts="$5" ext="$6"
    mkdir -p "$outdir"
    # shellcheck disable=SC2086  # word splitting of the option list is intended
    "$JDIFF" $opts "$ROOT/tests/fixtures/$org" "$ROOT/tests/fixtures/$new" \
        "$outdir/$name.$ext"
    [ -s "$outdir/$name.$ext" ] || {
        echo "gen-golden: empty output for $outdir/$name (differing pairs must produce output)" >&2
        exit 1
    }
    echo "  ${outdir#$GOLDEN/}/$name.$ext"
}

echo "gen-golden: generating golden outputs with oracle $JDIFF"
for i in "${!PAIR_NAMES[@]}"; do
    pair="${PAIR_NAMES[$i]}"
    for j in "${!OPTSET_NAMES[@]}"; do
        # Skip the ~80 MB ASCII listing of the binary pair (live-oracle
        # coverage only, see header comment).
        if [ "$pair" = "bkocomu" ] && [ "${OPTSET_NAMES[$j]}" = "l" ]; then
            echo "  bkocomu/l.asc skipped (~80 MB; verified live against the oracle)"
            continue
        fi
        gen_one "${PAIR_ORGS[$i]}" "${PAIR_NEW[$i]}" "$GOLDEN/$pair" \
            "${OPTSET_NAMES[$j]}" "${OPTSET_ARGS[$j]}" "${OPTSET_EXTS[$j]}"
    done
    if [ "$pair" = "test2" ]; then
        for j in "${!TEXT_ONLY_NAMES[@]}"; do
            gen_one "${PAIR_ORGS[$i]}" "${PAIR_NEW[$i]}" "$GOLDEN/$pair" \
                "${TEXT_ONLY_NAMES[$j]}" "${TEXT_ONLY_ARGS[$j]}" jdf
        done
    fi
done

# Verbose stderr captures: text pair, every patch-producing option set
# (including the text-pair-only in-memory sets).
V_NAMES=("${OPTSET_NAMES[@]}" "${TEXT_ONLY_NAMES[@]}")
V_ARGS=("${OPTSET_ARGS[@]}" "${TEXT_ONLY_ARGS[@]}")
V_EXTS=("${OPTSET_EXTS[@]}" jdf jdf)
for j in "${!V_NAMES[@]}"; do
    [ "${V_EXTS[$j]}" = "jdf" ] || continue # skip listings (-l/-lr)
    name="${V_NAMES[$j]}"
    opts="${V_ARGS[$j]}"
    # The patch itself goes to /dev/null; only stderr is captured (the byte
    # counts in the statistics do not depend on the output stream).
    # shellcheck disable=SC2086
    "$JDIFF" -v  $opts "$ROOT/tests/fixtures/test2.001.txt" \
        "$ROOT/tests/fixtures/test2.002.txt" /dev/null 2> "$GOLDEN/test2/$name.v.stderr"
    # shellcheck disable=SC2086
    "$JDIFF" -vv $opts "$ROOT/tests/fixtures/test2.001.txt" \
        "$ROOT/tests/fixtures/test2.002.txt" /dev/null 2> "$GOLDEN/test2/$name.vv.stderr"
    [ -s "$GOLDEN/test2/$name.v.stderr" ] || {
        echo "gen-golden: empty verbose capture for test2/$name.v" >&2
        exit 1
    }
    echo "  test2/$name.v.stderr + .vv.stderr"
done

echo "gen-golden: done ($(find "$GOLDEN" -type f | wc -l) files)"
