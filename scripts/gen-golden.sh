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
# Index-table mapping (spec §18.E/§21.7; Task 17 verification): the port's
# fixed 16-byte element divisor vs the 32-bit-hkey oracle's 12 means the SAME
# table needs 3/4 the MB on the oracle (`-i 6` oracle ≡ `-i 8` port ≡ prime
# 524287). Every matrix set is therefore captured with an appended `-i`
# override that makes the oracle's table element-equal to the port's table
# for that set, so the committed golden is exactly what the port must print
# for the set's own option values:
#     default/-p/-q/-p -q/-s/-k*/-n 1 -x 2/-x 5/-m*/-a*   -> oracle -i 24
#                                                            (port default 32)
#     -b -> -i 96    -bb -> -i 384    -f -> -i 12    -ff -> -i 6
#     -i 8 -> -i 6   -i 512 -> -i 384
# At element-equal tables the engines agree byte-for-byte on both corpus
# pairs (patches, listings and the whole -vv stream except one line, see
# below) — verified per set when this script runs (it diffs nothing; the
# byte-gate lives in tests/oracle.rs).
#
# The one exception is `-i 1`: the port's 1 MB table is 65536 elements
# (prime 65521), which no oracle `-i` can reproduce (0.75 MB is below the
# 1 MB floor). test2/-i 1 is captured at identical option values — test2 is
# index-insensitive in this range, so the patch bytes still match the port
# (pinned by the committed golden); its -vv stream can NOT match (the echo
# and stats lines print each side's own table: 65521 vs 87359 samples), so
# no verbose capture is committed for -i 1. bkocomu/-i 1 is not committed at
# all: port and oracle legitimately diverge there (different tables);
# byte-equality for that cell is covered live on the tiny pair and by
# round-trip/exit-parity on the corpus (tests/oracle.rs layer 2).
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

# The §22.1 matrix minus the pipe-only variants (name | jdiff options |
# output extension | element-equal oracle -i override). -l/-r produce
# listings; the rest produce binary patches. --compat-081 (§21.16) is a
# Task 23 addition and not part of this matrix.
OPTSET_NAMES=(default b bb f ff p q pq s i1 i8 i512 k0 k1 k65565 n1x2 x5 m0 m7 m2048 a0 a1 l r)
OPTSET_ARGS=("" "-b" "-bb" "-f" "-ff" "-p" "-q" "-p -q" "-s" \
             "-i 1" "-i 8" "-i 512" "-k 0" "-k 1" "-k 65565" \
             "-n 1 -x 2" "-x 5" "-m 0" "-m 7" "-m 2048" "-a 0" "-a 1" \
             "-l" "-r")
OPTSET_EXTS=(jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf jdf asc rgn)
# Element-equal table sizes: port MB * 3/4 (spec §18.E divisor 16 vs
# oracle divisor 12). Empty = capture at identical values (-i 1 case).
OPTSET_MAP=("-i 24" "-i 96" "-i 384" "-i 12" "-i 6" "-i 24" "-i 24" "-i 24" "-i 24" \
            "" "-i 6" "-i 384" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" "-i 24" \
            "-i 24" "-i 24")

# Corpus pairs: name | original | new.
PAIR_NAMES=(bkocomu test2)
PAIR_ORGS=(bkocomu.0000.fil test2.001.txt)
PAIR_NEW=(bkocomu.0009.fil test2.002.txt)

# The one stats line that can never byte-match (masked in goldens and by
# tests/oracle.rs, see header): the C++ prints an uninitialized counter.
mask_inaccurate() { # <infile> <outfile>
    sed 's/^Inaccurate  solutions   = .*/Inaccurate  solutions   = (masked)/' "$1" > "$2"
}

gen_one() { # <org> <new> <outdir> <optset-name> <opts> <ext> <map>
    local org="$1" new="$2" outdir="$3" name="$4" opts="$5" ext="$6" map="$7"
    mkdir -p "$outdir"
    # jdiff exits 1 on differing pairs (0.8.5 exit swap) — tolerated; an
    # empty output is not.
    # shellcheck disable=SC2086  # word splitting of the option list is intended
    "$JDIFF" $opts $map "$ROOT/tests/fixtures/$org" "$ROOT/tests/fixtures/$new" \
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
        # -i 1 has no element-equal oracle table (see header): capture the
        # text pair at identical values, skip the binary pair entirely.
        if [ "$name" = "i1" ] && [ "$pair" = "bkocomu" ]; then
            echo "  bkocomu/i1.jdf skipped (no oracle table equivalent; live coverage)"
            continue
        fi
        # Skip the ~80 MB ASCII listing of the binary pair (live-oracle
        # coverage only, see header comment).
        if [ "$pair" = "bkocomu" ] && [ "${OPTSET_EXTS[$j]}" = "asc" ]; then
            echo "  bkocomu/l.asc skipped (~80 MB; verified live against the oracle)"
            continue
        fi
        gen_one "${PAIR_ORGS[$i]}" "${PAIR_NEW[$i]}" "$GOLDEN/$pair" \
            "$name" "${OPTSET_ARGS[$j]}" "${OPTSET_EXTS[$j]}" "${OPTSET_MAP[$j]}"
    done
done

# Verbose stderr captures: both pairs, every patch-producing option set.
# Only stderr is captured (the byte counts in the statistics do not depend
# on the output stream); the "Inaccurate  solutions" line is masked (header).
# -i 1 has no verbose capture for either pair (header: each side's echo and
# stats lines print its own index table).
for i in "${!PAIR_NAMES[@]}"; do
    pair="${PAIR_NAMES[$i]}"
    for j in "${!OPTSET_NAMES[@]}"; do
        [ "${OPTSET_EXTS[$j]}" = "jdf" ] || continue # skip listings (-l/-r)
        name="${OPTSET_NAMES[$j]}"
        if [ "$name" = "i1" ]; then
            echo "  $pair/i1.vv.stderr skipped (echo/stats print each side's own table; header)"
            continue
        fi
        # shellcheck disable=SC2086
        "$JDIFF" -vv ${OPTSET_ARGS[$j]} ${OPTSET_MAP[$j]} \
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
