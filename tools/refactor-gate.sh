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
# What this catches, and what it does not. The throughput half is a timing
# measurement on a machine that has other things to do, and its resolution was
# measured rather than assumed: comparing a source tree against itself, the
# confidence interval spans roughly ten to fifteen percent, on the search
# benchmark and on the micro-benchmarks alike. So this reliably catches a
# gross regression, which is what a refactor produces when it goes wrong (a
# closure that stops being inlined, an allocation that creeps into a hot
# loop), and it cannot see a few percent.
#
# Nothing available here can. An SPRT with the usual bounds is built to detect
# a gain of about ten Elo, and a few percent of speed is worth a few Elo, so a
# match would spend hours and report no change. The choice is not between a
# precise instrument and a cheap one; it is between a cheap instrument that
# catches gross regressions in minutes and an expensive one that catches the
# same ones in hours. Anything smaller is invisible either way, which is worth
# stating rather than papering over.
#
# The sample settings below are deliberately above criterion's defaults. At
# the default ten samples this comparison reported "Performance has improved",
# p = 0.00, between two builds of identical source; more samples is what stops
# the gate firing at random.
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
sample_size="20"
measurement_time="30"

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
  --sample-size N criterion samples per benchmark        (default: 20)
  --measurement-time S
                  criterion seconds per benchmark        (default: 30)

Raising either costs run time and buys resolution. These defaults sit above
criterion's own, which take too few samples for this comparison and report
significant-looking differences between identical builds.

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
        --sample-size) sample_size="$2"; shift 2 ;;
        --measurement-time) measurement_time="$2"; shift 2 ;;
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

    if [ -n "$bench_filter" ]; then
        set -- bench --package turox-engine --bench "$bench_filter"
    else
        set -- bench --package turox-engine
    fi

    printf '\n=== throughput baseline: %s ===\n' "$base_ref"
    (cd "$base_src" && CRITERION_HOME="$criterion_home" \
        CARGO_TARGET_DIR="$base_target" \
        cargo "$@" -- --sample-size "$sample_size" \
        --measurement-time "$measurement_time" \
        --save-baseline refactor-gate) >&2

    printf '\n=== throughput under test: %s ===\n' "$test_ref"
    bench_log="$work_dir/bench-test.log"
    (cd "$test_src" && CRITERION_HOME="$criterion_home" \
        CARGO_TARGET_DIR="$test_target" \
        cargo "$@" -- --sample-size "$sample_size" \
        --measurement-time "$measurement_time" \
        --baseline refactor-gate) 2>&1 | tee "$bench_log"

    if grep -q 'Performance has regressed' "$bench_log"; then
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
    printf 'Move-identical, but criterion reports a regression.\n\n'
    printf 'Re-run before believing it. One verdict from this comparison is not\n'
    printf 'reliable at a few percent, so confirm the regression reproduces and\n'
    printf 'read its size rather than just its presence.\n\n'
    printf 'If it holds: the engine plays the same moves more slowly, so the whole\n'
    printf 'strength effect is that speed delta. A large regression is worth\n'
    printf 'pricing in Elo; a marginal one is below what a match can resolve:\n\n'
    printf '  tools/selfplay/sprt.sh --base %s --test %s\n' "$base_ref" "$test_ref"
    exit 1
fi

printf 'Move-identical at equal throughput. No SPRT owed.\n\n'
printf 'Same moves at the same speed is the same engine, so a match would be\n'
printf 'measuring nothing.\n'
