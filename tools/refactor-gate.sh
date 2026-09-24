#!/bin/sh
#
# The evidence a refactor owes before it lands.
#
# A refactor claims two things: that the engine still plays the same moves,
# and that it still plays them at the same speed. If both hold, playing
# strength is unchanged by construction and a self-play match measures
# nothing. This script checks both and says which of the three outcomes
# applies.
#
# The move-identity half is exact and is the reason to run this. The
# throughput half is weak, and how weak was measured rather than assumed:
# comparing a source tree *against itself*, so every number reported is noise,
# criterion's estimate came back between nine and twenty percent "faster"
# depending on configuration, never near zero.
#
# That is a bias rather than scatter, and it runs one way: whichever side is
# measured second looks faster, because the first process pays for a cold
# machine. This script measures the baseline first and the candidate second, so
# the bias favours passing, the worst direction for a check meant to catch
# regressions. A discarded warm-up pass below roughly halves it, and does not
# remove it.
#
# Raising the sample count makes this worse rather than better, which is why
# the script no longer tries: more samples narrows the interval around the
# biased estimate, so criterion grows confident in a change that is not there.
# At ten samples three null comparisons in eight were called an improvement; at
# thirty it was seven in eight.
#
# So the throughput reading is advisory. A change smaller than the floor is not
# evidence either way, and this script says so rather than passing criterion's
# verdict through. Measuring a few percent honestly needs a quiet machine and
# an alternating base/candidate design, which is a different tool.
#
# See README.md and docs/agents/toolchain.md for when each tool applies.

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

# Under `target/`, which is already gitignored, and per-ref so repeated runs
# against the same baseline reuse a warm cargo cache.
work_dir="$repo_root/target/refactor-gate"

base_ref="main"
test_ref="worktree"
depth="7"
bench_filter=""
run_bench="true"
# Percent. Below this, a reported change is indistinguishable from the bias
# measured above, so the script refuses to call it either way.
throughput_floor="20"

usage() {
    cat <<'USAGE'
Usage: refactor-gate.sh [options]

Checks a refactor against the two questions it has to answer, and reports
whether a self-play match is still owed.

Sides (each accepts a git ref or the literal `worktree` for the current
checkout including uncommitted changes; a path to a prebuilt binary works
only with --no-bench, since a benchmark needs a source tree to run in):
  --base REF      the baseline being preserved  (default: main)
  --test REF      the refactor under test       (default: worktree)

Checks:
  --depth N       search depth for the move-identity check
                                                (default: 7)
  --bench NAME    run only this benchmark target rather than all of them;
                  `search` is the one that covers the search hot path
  --no-bench      skip the throughput check and report move-identity only,
                  for iterating on a change before it is ready to measure
  --floor PCT     treat a throughput change smaller than this as no evidence
                                                         (default: 20)

The floor is not caution: comparing a tree against itself on this hardware
reports differences of nine to twenty percent, so anything under it is
indistinguishable from measuring nothing. Each benchmark sets its own sample
count in code, which overrides any command line flag, so this script does not
pass one.

The machine should be otherwise idle: the throughput half is a timing
measurement, and anything else competing for CPU invalidates it.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --base) base_ref="$2"; shift 2 ;;
        --test) test_ref="$2"; shift 2 ;;
        --depth) depth="$2"; shift 2 ;;
        --bench) bench_filter="$2"; shift 2 ;;
        --no-bench) run_bench="false"; shift ;;
        --floor) throughput_floor="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'refactor-gate.sh: unknown option: %s\n\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

die() {
    printf 'refactor-gate.sh: %s\n' "$1" >&2
    exit 1
}

# Resolves one side to a source tree and a binary, building it if it names a
# git ref. Sets `resolved_src` and `resolved_bin` rather than echoing, since
# both halves are needed: the binary drives the move-identity check and the
# source tree is what `cargo bench` runs in.
#
# This shares its shape with `selfplay/sprt.sh`'s own engine resolution.
# Unifying the two waits on the standing decision about what may live under
# `tools/` and how it is shared, rather than being settled by whichever
# script was edited most recently.
resolve() {
    ref="$1"
    slot="$2"

    if [ "$ref" = "worktree" ]; then
        printf '=== building %s: current working tree ===\n' "$slot" >&2
        (cd "$repo_root" && cargo build --release --package turox-cli) >&2
        resolved_src="$repo_root"
        resolved_target="$repo_root/target"
        resolved_bin="$work_dir/bin/$slot-worktree"
        mkdir -p "$work_dir/bin"
        cp "$repo_root/target/release/turox-cli" "$resolved_bin"
        return
    fi

    if [ -f "$ref" ]; then
        if [ "$run_bench" = "true" ]; then
            die "a prebuilt binary has no source tree to benchmark: $ref
  Pass a git ref or 'worktree', or add --no-bench to check moves alone."
        fi
        resolved_src=""
        resolved_target=""
        resolved_bin=$(CDPATH= cd -- "$(dirname -- "$ref")" && printf '%s/%s' "$(pwd)" "$(basename -- "$ref")")
        return
    fi

    sha=$(cd "$repo_root" && git rev-parse --short --verify "$ref^{commit}" 2>/dev/null) \
        || die "not a git ref or 'worktree': $ref"
    resolved_src="$work_dir/src/$sha"
    resolved_target="$work_dir/target/$sha"
    printf '=== building %s: %s (%s) ===\n' "$slot" "$ref" "$sha" >&2

    # `git archive` rather than `git worktree add`: this only needs to read a
    # tree, and an archive extract leaves no worktree registration behind to
    # clean up if the run is interrupted.
    rm -rf "$resolved_src"
    mkdir -p "$resolved_src"
    (cd "$repo_root" && git archive "$sha") | tar -x -C "$resolved_src"

    # Separate target dir per ref, so the two builds do not evict each
    # other's artifacts on every run.
    (cd "$resolved_src" && CARGO_TARGET_DIR="$resolved_target" \
        cargo build --release --package turox-cli) >&2
    mkdir -p "$work_dir/bin"
    resolved_bin="$work_dir/bin/$slot-$sha"
    cp "$resolved_target/release/turox-cli" "$resolved_bin"
}

resolve "$base_ref" base
base_src="$resolved_src"
base_target="$resolved_target"
base_bin="$resolved_bin"

resolve "$test_ref" test
test_src="$resolved_src"
test_target="$resolved_target"
test_bin="$resolved_bin"

if cmp -s "$base_bin" "$test_bin"; then
    printf 'refactor-gate.sh: note: the two binaries are byte-identical; this is a null run.\n' >&2
fi

# Question one: does it play the same moves? Node counts alone would miss an
# inverted comparison, which walks the same tree and returns a different
# move, so the principal variation is compared too.
printf '\n=== move-identical check (depth %s) ===\n' "$depth"
moves_ok="true"
if python3 "$script_dir/treeshape/measure.py" \
    --bin "$base_bin" --bin "$test_bin" --depth "$depth" --require-identical; then
    :
else
    moves_ok="false"
fi

# Question two: does it run at the same speed? Criterion's own regression
# detection decides this, rather than a threshold invented here: it already
# does the sampling, outlier rejection and significance testing that turn a
# pair of timings into a verdict.
bench_ok="true"
bench_ran="false"
if [ "$run_bench" = "true" ]; then
    bench_ran="true"
    criterion_home="$work_dir/criterion"
    rm -rf "$criterion_home"
    mkdir -p "$criterion_home"

    # `--benches`, not a bare `cargo bench`: the latter also runs the lib's
    # unit-test harness as a benchmark target, and libtest does not understand
    # `--save-baseline`, so the run dies before measuring anything.
    if [ -n "$bench_filter" ]; then
        set -- bench --package turox-engine --bench "$bench_filter"
    else
        set -- bench --package turox-engine --benches
    fi

    # Discarded. The first process of a run pays for a cold machine, and
    # whichever side is measured after it looks faster for reasons that have
    # nothing to do with the code. Throwing one pass away roughly halves that.
    printf '\n=== warm-up (discarded) ===\n'
    (cd "$base_src" && CRITERION_HOME="$criterion_home" \
        CARGO_TARGET_DIR="$base_target" \
        cargo "$@" -- --save-baseline warmup) >&2

    printf '\n=== throughput baseline: %s ===\n' "$base_ref"
    (cd "$base_src" && CRITERION_HOME="$criterion_home" \
        CARGO_TARGET_DIR="$base_target" \
        cargo "$@" -- --save-baseline refactor-gate) >&2

    printf '\n=== throughput under test: %s ===\n' "$test_ref"
    bench_log="$work_dir/bench-test.log"
    (cd "$test_src" && CRITERION_HOME="$criterion_home" \
        CARGO_TARGET_DIR="$test_target" \
        cargo "$@" -- --baseline refactor-gate) 2>&1 | tee "$bench_log"

    # Criterion's own verdict is deliberately ignored. It answers whether a
    # difference is statistically real, and here it says yes to one that is
    # not. The question asked instead is whether a reported *slowdown* clears
    # the floor, which is the only claim this measurement supports.
    #
    # Slowdowns only, not the absolute change. The bias runs toward reporting
    # the candidate as faster, so flagging large apparent speedups would fail
    # nearly every run on the strength of the very artefact being worked
    # around. An apparent speedup here is not evidence of anything either; it
    # is simply not the failure this is looking for.
    #
    # `sed`, not `tr`: criterion prints U+2212, which is three bytes, and
    # `tr` substitutes byte by byte, turning each minus into three hyphens
    # that awk then reads as zero. That failure is silent and reports every
    # change as 0.0%, so the check passes everything.
    worst=$(sed 's/−/-/g' "$bench_log" | awk '
        /change:/ { getline
                    if (match($0, /\[[^]]*\]/)) {
                        split(substr($0, RSTART + 1, RLENGTH - 2), a, "%")
                        v = a[2] + 0
                        if (v > m) m = v
                    } }
        END { printf "%.1f", m + 0 }')
    printf '\nlargest slowdown reported: %s%%, floor %s%%\n' "$worst" "$throughput_floor"
    over=$(awk -v w="$worst" -v f="$throughput_floor" 'BEGIN { print (w > f) ? "yes" : "no" }')
    if [ "$over" = "yes" ]; then
        bench_ok="false"
    fi
fi

printf '\n=========================================\n'
if [ "$moves_ok" = "false" ]; then
    printf 'NOT move-identical.\n\n'
    printf 'This is a change in what the engine plays, not a refactor, so the\n'
    printf 'throughput reading above does not settle it. It owes a full\n'
    printf 'wall-clock SPRT:\n\n'
    printf '  tools/selfplay/sprt.sh --base %s --test %s\n' "$base_ref" "$test_ref"
    exit 1
fi

if [ "$bench_ran" = "false" ]; then
    printf 'Move-identical. Throughput not measured (--no-bench).\n\n'
    printf 'Re-run without --no-bench before calling this done.\n'
    exit 0
fi

if [ "$bench_ok" = "false" ]; then
    printf 'Move-identical, but a throughput change cleared the floor.\n\n'
    printf 'Large enough to be worth looking at, which is all this says: the\n'
    printf 'measurement is biased toward reporting the candidate as faster, so a\n'
    printf 'change big enough to show up anyway deserves attention.\n\n'
    printf 'Re-run first. If it holds, the engine plays the same moves at a\n'
    printf 'different speed, so the whole strength effect is that delta, and a\n'
    printf 'match is the only thing that prices it:\n\n'
    printf '  tools/selfplay/sprt.sh --base %s --test %s\n' "$base_ref" "$test_ref"
    exit 1
fi

printf 'Move-identical, no throughput change above the floor. No SPRT owed.\n\n'
printf 'Same moves at the same speed is the same engine, so a match would be\n'
printf 'measuring nothing. Note what this does not say: a regression smaller\n'
printf 'than the floor would not have been seen, and nothing available here\n'
printf 'could see it.\n'
