#!/bin/sh
#
# Self-play A/B testing: runs an SPRT match between two turox builds and
# reports whether the candidate is stronger, weaker, or neither.
#
# Builds each side from a git ref (or takes a prebuilt binary), then hands
# both to `fastchess`. See README.md in this directory for installing
# fastchess, for the reasoning behind the defaults, and for how to read the
# result.

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

# Under `target/`, which is already gitignored, and per-ref so repeated runs
# against the same baseline reuse a warm cargo cache instead of rebuilding
# the whole workspace every time.
work_dir="$repo_root/target/selfplay"

base_ref="main"
test_ref="worktree"
time_control="10+0.1"
nodes=""
timemargin="200"
elo0="0"
elo1="10"
alpha="0.05"
beta="0.05"
rounds=""
concurrency=""
openings="$script_dir/openings.epd"
fastchess="${FASTCHESS:-fastchess}"
pgnout=""
seed="42"

usage() {
    cat <<'USAGE'
Usage: sprt.sh [options]

Engines (each accepts a git ref, a path to a prebuilt binary, or the literal
`worktree` for the current checkout including uncommitted changes):
  --base REF      baseline to beat            (default: main)
  --test REF      candidate under test        (default: worktree)

Match:
  --tc TC         time control, cutechess format, seconds+increment
                                              (default: 10+0.1)
  --nodes N       search a fixed N nodes per move instead of using a clock;
                  slower per game but free of timing noise, and time
                  management stops being part of what is measured
  --timemargin MS clock overrun tolerated before a loss on time
                                              (default: 200)
  --rounds N      max rounds, 2 games each; capped at the opening suite size
                                              (default: the whole suite)
  --concurrency N games in parallel           (default: physical core count)
  --openings FILE EPD opening suite           (default: ./openings.epd)
  --seed N        opening shuffle seed        (default: 42)
  --pgnout FILE   where to write the games    (default: under target/selfplay)

SPRT:
  --elo0 N        H0: the change is worth no more than this  (default: 0)
  --elo1 N        H1: the change is worth at least this      (default: 10)
  --alpha N       false-accept rate                          (default: 0.05)
  --beta N        false-reject rate                          (default: 0.05)

Environment:
  FASTCHESS       path to the fastchess binary (default: fastchess on PATH)
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --base) base_ref="$2"; shift 2 ;;
        --test) test_ref="$2"; shift 2 ;;
        --tc) time_control="$2"; shift 2 ;;
        --nodes) nodes="$2"; shift 2 ;;
        --timemargin) timemargin="$2"; shift 2 ;;
        --rounds) rounds="$2"; shift 2 ;;
        --concurrency) concurrency="$2"; shift 2 ;;
        --openings) openings="$2"; shift 2 ;;
        --seed) seed="$2"; shift 2 ;;
        --pgnout) pgnout="$2"; shift 2 ;;
        --elo0) elo0="$2"; shift 2 ;;
        --elo1) elo1="$2"; shift 2 ;;
        --alpha) alpha="$2"; shift 2 ;;
        --beta) beta="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'sprt.sh: unknown option: %s\n\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

die() {
    printf 'sprt.sh: %s\n' "$1" >&2
    exit 1
}

command -v "$fastchess" >/dev/null 2>&1 || die \
"fastchess not found (looked for '$fastchess').
  It has no Homebrew formula; build it from source and either put the binary
  on PATH or point FASTCHESS at it. See README.md in this directory."

[ -f "$openings" ] || die "opening suite not found: $openings"

# Physical cores, not logical: the engine is single-threaded and the match is
# timed, so oversubscribing hyperthreads distorts every game's clock.
if [ -z "$concurrency" ]; then
    if concurrency=$(sysctl -n hw.physicalcpu 2>/dev/null); then
        :
    elif concurrency=$(nproc 2>/dev/null); then
        :
    else
        concurrency=1
    fi
fi

# Every round uses one start position, and turox's search is deterministic
# with no opening book: reusing a position replays a game that was already
# played rather than sampling a new one, which would inflate the SPRT's game
# count without adding any information to it. fastchess wraps around the
# book (`idx % book_size`) once it runs out, so the cap has to be enforced
# here.
suite_size=$(grep -c '[^[:space:]]' "$openings")
if [ -z "$rounds" ]; then
    rounds="$suite_size"
elif [ "$rounds" -gt "$suite_size" ]; then
    printf 'sprt.sh: capping --rounds %s at the opening suite size (%s)\n' \
        "$rounds" "$suite_size" >&2
    rounds="$suite_size"
fi

# Resolves one engine argument to a binary path, building it if it names a
# git ref. Echoes the path; all progress output goes to stderr so the caller
# can capture the path with a command substitution.
resolve_engine() {
    ref="$1"
    slot="$2"

    if [ -f "$ref" ]; then
        (CDPATH= cd -- "$(dirname -- "$ref")" && printf '%s/%s\n' "$(pwd)" "$(basename -- "$ref")")
        return
    fi

    if [ "$ref" = "worktree" ]; then
        printf '=== building %s: current working tree ===\n' "$slot" >&2
        (cd "$repo_root" && cargo build --release --package turox-cli) >&2
        built="$repo_root/target/release/turox-cli"
        tag="worktree"
    else
        sha=$(cd "$repo_root" && git rev-parse --short --verify "$ref^{commit}" 2>/dev/null) \
            || die "not a git ref, an existing file, or 'worktree': $ref"
        tag="$sha"
        src="$work_dir/src/$tag"
        printf '=== building %s: %s (%s) ===\n' "$slot" "$ref" "$sha" >&2

        # `git archive` rather than `git worktree add`: this only needs to
        # read a tree, and an archive extract leaves no worktree registration
        # behind in the repo to clean up if the match is interrupted.
        rm -rf "$src"
        mkdir -p "$src"
        (cd "$repo_root" && git archive "$sha") | tar -x -C "$src"

        # Separate target dir per ref: sharing one with the working tree would
        # make the two builds evict each other's artifacts on every run.
        (cd "$src" && CARGO_TARGET_DIR="$work_dir/target/$tag" \
            cargo build --release --package turox-cli) >&2
        built="$work_dir/target/$tag/release/turox-cli"
    fi

    # Copied out rather than used in place, so a `cargo build` elsewhere
    # (including this script's own second build) cannot replace the binary
    # underneath a running match.
    mkdir -p "$work_dir/bin"
    binary="$work_dir/bin/$slot-$tag"
    cp "$built" "$binary"
    printf '%s\n' "$binary"
}

base_bin=$(resolve_engine "$base_ref" base)
test_bin=$(resolve_engine "$test_ref" test)

if cmp -s "$base_bin" "$test_bin"; then
    printf 'sprt.sh: note: base and test are byte-identical binaries; this is a null test.\n' >&2
fi

if [ -z "$pgnout" ]; then
    mkdir -p "$work_dir/results"
    pgnout="$work_dir/results/sprt-$(date +%Y%m%d-%H%M%S).pgn"
fi

# `--nodes` replaces the clock outright rather than bounding it: a search
# budget and a time control at once would let whichever runs out first decide
# the move, which is neither of the two things this is trying to measure.
if [ -n "$nodes" ]; then
    budget="nodes=$nodes"
    budget_label="$nodes nodes/move"
else
    budget="tc=$time_control"
    budget_label="$time_control"
fi

printf '\n=== match ===\n'
printf 'base:        %s (%s)\n' "$base_ref" "$base_bin"
printf 'test:        %s (%s)\n' "$test_ref" "$test_bin"
printf 'budget:      %s\n' "$budget_label"
printf 'openings:    %s (%s positions)\n' "$openings" "$suite_size"
printf 'rounds:      %s (up to %s games)\n' "$rounds" "$((rounds * 2))"
printf 'concurrency: %s\n' "$concurrency"
printf 'sprt:        elo0=%s elo1=%s alpha=%s beta=%s (logistic)\n' \
    "$elo0" "$elo1" "$alpha" "$beta"
printf 'games:       %s\n\n' "$pgnout"

# `-repeat` plays each opening twice with the colors swapped, so any
# imbalance in a start position hits both engines equally and cancels in the
# pair; that is also what makes the pentanomial (Ptnml) statistics in the
# output meaningful.
#
# `timemargin` absorbs process and pipe latency plus the engine's own
# overshoot past its deadline, so a game decided by scheduling jitter is not
# scored as a real loss. Adjudication cuts off dead-drawn and hopeless
# positions so the match spends its time on games that still carry
# information; `twosided=true` on resignation means both engines have to
# agree the position is lost, which matters when the whole point of the match
# is that their evaluations differ.
exec "$fastchess" \
    -engine "cmd=$base_bin" "name=base" \
    -engine "cmd=$test_bin" "name=test" \
    -each "$budget" "timemargin=$timemargin" proto=uci \
    -openings "file=$openings" format=epd order=random \
    -srand "$seed" \
    -config "outname=$work_dir/config.json" \
    -rounds "$rounds" -repeat -concurrency "$concurrency" \
    -sprt "elo0=$elo0" "elo1=$elo1" "alpha=$alpha" "beta=$beta" model=logistic \
    -maxmoves 200 \
    -draw movenumber=40 movecount=8 score=10 \
    -resign movecount=5 score=1000 twosided=true \
    -pgnout "file=$pgnout" notation=san
