#!/usr/bin/env bash
#
# Generate the committed golden85 fixtures for tests/oracle.rs by running the
# C++ 0.8.5 reference oracle (spec §22.2: "Regenerate all goldens from the
# 0.8.5 oracle — regenerate, don't mix").
#
# Requires the 0.8.5 oracle built by scripts/build-oracle.sh (either point
# JOJODIFF_ORACLE at its directory or leave target/oracle in place). The
# legacy 0.8.1 goldens under tests/fixtures/golden/ are NEVER touched: they
# re-pin the cross-version compatibility gate (spec §22.3, tests/crossver.rs).
#
# Goldens are written to tests/fixtures/golden85/<pair>/<optset>.<ext>:
#   <optset>.jdf         binary patch        (22 patch sets of the §22.1
#                                            matrix minus the pipe-only
#                                            variants and --compat-081;
#                                            --compat-081 arrives in Task 23)
#   <optset>.asc         -l listing
#   <optset>.rgn         -r region listing
#   <optset>.vv.stderr   jdiff -vv stderr    (patch sets)
#
# Index tables (spec §21.18): port and oracle build IDENTICAL tables at the
# same `-i` (12 bytes/element on both sides — the u32-hkey variant), so every
# matrix set is captured at un-overridden option values and the committed
# golden is exactly what the port must print for that set. (An earlier
# revision carried a port-/16-vs-oracle-/12 mismatch and an element-equal
# `-i` remap here; retired with §21.18.)
#
# Deliberate restrictions:
#   * bkocomu/l.asc (the ASCII listing of the binary pair, ~80 MB) is NOT
#     written: committing it would bloat every clone. That single matrix
#     cell is verified by the live-oracle layer instead (Rust vs C++
#     directly), which the CI oracle job runs on every push.
#   * The "Inaccurate  solutions" line of -vv captures is replaced by a
#     masked placeholder before writing. The C++ never initializes that
#     counter (spec §21.5/§21.6): the oracle prints different garbage on
#     every run while the port prints its deterministic real count — the
#     line can never byte-match, so both the goldens and the test mask it
#     (tests/oracle.rs). Every other stats line is deterministic and gates
#     byte-exact.
#
# Goldens are oracle truth, committed to the repository; never regenerate
# them from Rust output.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOLDEN="$ROOT/tests/fixtures/golden85"

# Locate the 0.8.5 oracle (env override, else the build-oracle.sh output
# location; it has no jptch — 0.8.5 ships a single binary).
ORACLE_DIR="${JOJODIFF_ORACLE:-$ROOT/target/oracle}"
JDIFF="$ORACLE_DIR/jdiff"
if [ ! -x "$JDIFF" ]; then
    echo "gen-golden: 0.8.5 oracle (jdiff) not found at $ORACLE_DIR - run scripts/build-oracle.sh first" >&2
    exit 1
fi

# Sanity: the binary must be the 0.8.5 oracle (mirrors build-oracle.sh's
# check — gen-golden must never capture from the wrong binary).
SANITY_DIR="$(mktemp -d)"
trap 'rm -rf "$SANITY_DIR"' EXIT
printf 'hello world hello' > "$SANITY_DIR/a.bin"
printf 'hello world byebye' > "$SANITY_DIR/b.bin"
"$JDIFF" -v "$SANITY_DIR/a.bin" "$SANITY_DIR/b.bin" "$SANITY_DIR/s.jdf" \
    > "$SANITY_DIR/sanity.out" 2>&1 || true
for want in "0.8.5 (beta) 2020" "samples are 32 bytes"; do
    if ! grep -Fq "$want" "$SANITY_DIR/sanity.out"; then
        echo "gen-golden: sanity check failed - '$want' missing from $JDIFF -v output:" >&2
        cat "$SANITY_DIR/sanity.out" >&2
        exit 1
    fi
done
rm -rf "$SANITY_DIR"
trap - EXIT

# The §22.1 matrix minus the pipe-only variants (name | jdiff options |
# output extension). -l/-r produce listings; the rest produce binary patches.
# --compat-081 (§21.16) is a Task 23 addition and not part of this matrix.
OPTSET_NAMES=(default b bb f ff p q pq s i1 i8 i512 k0 k1 k65565 n1x2 x5 m0 m7 m2048 a0 a1 l r)
OPTSET_ARGS=("" "-b" "-bb" "-f" "-ff" "-p" "-q" "-p -q" "-s" \
             "-i 1" "-i 8" "-i 512" "-k 0" "-k 1" "-k 65565" \
             "-n 1 -x 2" "-x 5" "-m 0" "-m 7" "-m 2048" "-a 0" "-a 1" \
             "-l" "-r")
OPTSET_EXTS=(jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf asc rgn)

# Corpus pairs: name | original | new.
PAIR_NAMES=(bkocomu test2)
PAIR_ORGS=(bkocomu.0000.fil test2.001.txt)
PAIR_NEW=(bkocomu.0009.fil test2.002.txt)

# The one stats line that can never byte-match (masked in goldens and by
# tests/oracle.rs, see header): the C++ prints an uninitialized counter.
mask_inaccurate() { # <infile> <outfile>
    sed 's/^Inaccurate  solutions   = .*/Inaccurate  solutions   = (masked)/' "$1" > "$2"
}

gen_one() { # <org> <new> <outdir> <optset-name> <opts> <ext>
    local org="$1" new="$2" outdir="$3" name="$4" opts="$5" ext="$6"
    mkdir -p "$outdir"
    # jdiff exits 1 on differing pairs (0.8.5 exit swap) — tolerated; an
    # empty output is not.
    # shellcheck disable=SC2086  # word splitting of the option list is intended
    "$JDIFF" $opts "$ROOT/tests/fixtures/$org" "$ROOT/tests/fixtures/$new" \
        "$outdir/$name.$ext" || [ $? -eq 1 ]
    if [ ! -s "$outdir/$name.$ext" ]; then
        echo "gen-golden: empty output for $outdir/$name (differing pairs must produce output)" >&2
        exit 1
    fi
    echo "  ${outdir#$GOLDEN/}/$name.$ext"
}

echo "gen-golden: generating golden85 outputs with oracle $JDIFF"
rm -rf "$GOLDEN"
for i in "${!PAIR_NAMES[@]}"; do
    pair="${PAIR_NAMES[$i]}"
    for j in "${!OPTSET_NAMES[@]}"; do
        name="${OPTSET_NAMES[$j]}"
        # Skip the ~80 MB ASCII listing of the binary pair (live-oracle
        # coverage only, see header comment).
        if [ "$pair" = "bkocomu" ] && [ "${OPTSET_EXTS[$j]}" = "asc" ]; then
            echo "  bkocomu/l.asc skipped (~80 MB; verified live against the oracle)"
            continue
        fi
        gen_one "${PAIR_ORGS[$i]}" "${PAIR_NEW[$i]}" "$GOLDEN/$pair" \
            "$name" "${OPTSET_ARGS[$j]}" "${OPTSET_EXTS[$j]}"
    done
done

# Verbose stderr captures: both pairs, every patch-producing option set.
# Only stderr is captured (the byte counts in the statistics do not depend
# on the output stream); the "Inaccurate  solutions" line is masked (header).
for i in "${!PAIR_NAMES[@]}"; do
    pair="${PAIR_NAMES[$i]}"
    for j in "${!OPTSET_NAMES[@]}"; do
        [ "${OPTSET_EXTS[$j]}" = "jdf" ] || continue # skip listings (-l/-r)
        name="${OPTSET_NAMES[$j]}"
        # shellcheck disable=SC2086
        "$JDIFF" -vv ${OPTSET_ARGS[$j]} \
            "$ROOT/tests/fixtures/${PAIR_ORGS[$i]}" \
            "$ROOT/tests/fixtures/${PAIR_NEW[$i]}" /dev/null \
            2> "$GOLDEN/$pair/$name.vv.raw" || [ $? -eq 1 ]
        mask_inaccurate "$GOLDEN/$pair/$name.vv.raw" "$GOLDEN/$pair/$name.vv.stderr"
        rm "$GOLDEN/$pair/$name.vv.raw"
        [ -s "$GOLDEN/$pair/$name.vv.stderr" ] || {
            echo "gen-golden: empty verbose capture for $pair/$name.vv" >&2
            exit 1
        }
        echo "  $pair/$name.vv.stderr"
    done
done

echo "gen-golden: done ($(find "$GOLDEN" -type f | wc -l) files)"
