#!/usr/bin/env python3
"""Measure turox's tree shape and throughput at fixed depth.

The effective branching factor is the number that orders the strength
backlog: it says whether the engine is limited by how fast it searches a node
or by how many nodes it searches at all. Getting it by hand meant building a
binary and running positions one at a time, which is both tedious and
impossible to compare against later.

EBF here is the ratio of node counts between consecutive depths, not the
textbook average branching factor. That is the useful quantity: it answers
"what does one more ply cost", which is what decides whether a change bought
depth. Alpha-beta with good ordering approaches the square root of the true
branching factor, so roughly 6 for chess, and modern engines with reductions
reach 2 to 3.

Node counts at fixed depth are deterministic, so they compare cleanly across
builds even on a busy machine. Times and nps are not: run those on an idle
machine or treat them as indicative.

Usage:
  tools/treeshape/measure.py --ref main --ref HEAD --depth 7
  tools/treeshape/measure.py --bin ./target/release/turox-cli
  tools/treeshape/measure.py --ref main --ref HEAD --require-identical
"""

import argparse
import re
import subprocess
import statistics
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BUILD_ROOT = REPO / "target" / "treeshape"

# Chosen for variety of tree shape, since the number this tool reports is a
# growth rate and one dominant shape would decide it: an open middlegame is
# capture-rich and spends its budget in quiescence, a closed one has few
# captures and leans on move ordering instead, and an endgame is cheap per node
# with a high branching factor for the pieces left.
#
# Two kinds of position are excluded, both because they measure
# re-verification rather than search growth. One is a forced mate found at
# shallow depth: iterative deepening stops learning from further iterations and
# the ratios report how fast the engine re-confirms an answer it already had.
# The other is a position whose score is flat from depth 1, which is the same
# defect arrived at by material rather than by mate. Check a candidate by
# reading its score per depth before adding it; a working position's score
# moves.
#
# Watch for an immediate free capture when composing one by hand: two rooks
# sharing a file with nothing between them hands the side to move a piece, and
# the resulting score looks like a positional judgement rather than the
# one-move tactic it is.
#
# Both colours are to move somewhere in this set. A tree-shape measurement
# crossed with a colour is the shape that has produced scrambled bugs in this
# engine before, and a suite that only ever moves White cannot see one.
#
# Labels name the shape rather than the position, since reading EBF by shape is
# the point of the table. Two of them have names worth knowing anyway:
# `open-mid` is Kiwipete, the perft suite's second position, and `pawn-end` is
# its third, so both are already legality-checked by the perft tests and appear
# under those names elsewhere in the repo.
POSITIONS = [
    ("startpos", None),
    (
        "open-mid",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ),
    (
        "quiet-mid",
        "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    ),
    (
        "closed-mid",
        "r1bq1rk1/ppp1npbp/3p1np1/3Pp3/2P1P3/2N2N1P/PP2BPP1/R1BQ1RK1 b - - 0 10",
    ),
    (
        "sharp-mid",
        "r1b1k2r/2qnbppp/p2ppn2/1p4B1/3NPP2/2N2Q2/PPP3PP/2KR1B1R w kq - 0 11",
    ),
    ("pawn-end", "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1"),
    ("rook-end", "8/5pk1/6p1/8/8/1R4P1/5PK1/3r4 w - - 0 1"),
]

INFO = re.compile(r"^info depth (\d+)\b(.*)$", re.M)
NODES = re.compile(r"\bnodes (\d+)")
TIME = re.compile(r"\btime (\d+)")
# `pv` runs to the end of a UCI info line, so it takes the rest of it. The
# principal variation is the half of a comparison that catches a changed
# conclusion: two searches can walk the same number of nodes and still
# disagree on the move, which is what an inverted comparison in an extracted
# loop produces.
PV = re.compile(r"\bpv (.+)$")


def build(ref: str) -> Path:
    """Build `ref` into its own target dir and return the binary."""
    src = BUILD_ROOT / "src" / ref.replace("/", "_")
    src.mkdir(parents=True, exist_ok=True)
    sha = subprocess.run(
        ["git", "rev-parse", "--short", ref],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    subprocess.run(
        ["git", "--work-tree", str(src), "checkout", ref, "--", "."],
        cwd=REPO,
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["cargo", "build", "--release", "-p", "turox-cli"],
        cwd=src,
        check=True,
        capture_output=True,
    )
    out = BUILD_ROOT / "bin" / f"{ref.replace('/', '_')}-{sha}"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes((src / "target" / "release" / "turox-cli").read_bytes())
    out.chmod(0o755)
    return out


def run_position(binary: Path, fen, depth: int):
    """{depth: (nodes, ms, pv)} for one position, from a single search.

    One `go depth N` reports every completed iteration, so the whole curve
    comes from one search rather than N of them. That also makes the numbers
    honest about iterative deepening: each depth's node count includes the
    shallower iterations that preceded it, which is what a real search pays.
    """
    pos = "position startpos" if fen is None else f"position fen {fen}"
    # Randomization and the book both off, or a run varies by half between
    # runs of the same binary (randomization), or `startpos` skips search
    # entirely and reports no `info depth` lines at all (the book). Builds
    # predating either option ignore the corresponding line, per UCI's own
    # convention for an unrecognized option name, so the same script drives
    # old and new builds alike.
    script = (
        "uci\nsetoption name Randomize value false\nsetoption name Book value false\n"
        f"ucinewgame\n{pos}\ngo depth {depth}\nquit\n"
    )
    proc = subprocess.run(
        [str(binary)], input=script, capture_output=True, text=True, timeout=1800
    )
    out = {}
    for d, rest in INFO.findall(proc.stdout):
        nodes = NODES.search(rest)
        ms = TIME.search(rest)
        pv = PV.search(rest)
        out[int(d)] = (
            int(nodes.group(1)) if nodes else 0,
            int(ms.group(1)) if ms else 0,
            pv.group(1).strip() if pv else "",
        )
    return out


def report(label: str, results: dict, depth: int):
    print(f"\n=== {label} ===")
    print(
        f"{'position':10} {'depth':>5} {'nodes':>12} {'time_ms':>8} {'nps':>9} {'EBF':>6}"
    )
    ebfs = []
    for name, per_depth in results.items():
        prev = None
        for d in range(1, depth + 1):
            if d not in per_depth:
                continue
            nodes, ms, _pv = per_depth[d]
            nps = int(nodes * 1000 / ms) if ms else 0
            ebf = nodes / prev if prev else None
            if ebf is not None and d >= 4:
                # Below depth 4 the ratios are dominated by fixed overhead
                # rather than by tree growth, so they would flatter the mean.
                ebfs.append(ebf)
            print(
                f"{name:10} {d:5} {nodes:12,} {ms:8} {nps:9,} "
                f"{('%.2f' % ebf) if ebf else '':>6}"
            )
            prev = nodes
    if ebfs:
        geo = statistics.geometric_mean(ebfs)
        print(f"\ngeometric-mean EBF (depth >= 4): {geo:.2f}  over {len(ebfs)} ratios")
    return statistics.geometric_mean(ebfs) if ebfs else None


def compare(a, b, depth):
    """Node-count and PV mismatches between two measured runs.

    Node counts on their own would miss the failure a refactor is likeliest to
    cause: an inverted or reordered comparison walks the same tree, counts the
    same nodes, and returns a different move. Comparing the principal variation
    as well is what makes that visible, and it costs nothing, since the engine
    already reports it on every info line.
    """
    diffs = []
    for name, _ in POSITIONS:
        pa, pb = a.get(name, {}), b.get(name, {})
        for d in range(1, depth + 1):
            if d not in pa or d not in pb:
                if (d in pa) != (d in pb):
                    diffs.append(f"{name:10} depth {d}: reported by one side only")
                continue
            nodes_a, _, pv_a = pa[d]
            nodes_b, _, pv_b = pb[d]
            if nodes_a != nodes_b:
                diffs.append(f"{name:10} depth {d}: nodes {nodes_a:,} vs {nodes_b:,}")
            if pv_a != pv_b:
                diffs.append(f"{name:10} depth {d}: pv {pv_a!r} vs {pv_b!r}")
    return diffs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--ref", action="append", default=[], help="git ref to build and measure"
    )
    ap.add_argument(
        "--bin", action="append", default=[], help="prebuilt binary to measure"
    )
    ap.add_argument("--depth", type=int, default=7)
    ap.add_argument(
        "--require-identical",
        action="store_true",
        help="exit nonzero unless the two targets agree on node counts and PV",
    )
    args = ap.parse_args()

    targets = []
    for r in args.ref:
        print(f"building {r} ...", file=sys.stderr)
        targets.append((r, build(r)))
    for b in args.bin:
        # Labelled by file name rather than by the path given: a caller that
        # passes absolute paths (the refactor gate does) otherwise gets a
        # comparison table too wide to read.
        targets.append((Path(b).name, Path(b)))
    if not targets:
        targets = [("worktree", REPO / "target" / "release" / "turox-cli")]

    summary = {}
    measured = {}
    for label, binary in targets:
        if not binary.exists():
            sys.exit(f"missing binary: {binary}")
        results = {}
        for name, fen in POSITIONS:
            results[name] = run_position(binary, fen, args.depth)
        measured[label] = results
        summary[label] = report(f"{label}  ({binary.name})", results, args.depth)

    if len(summary) > 1:
        print("\n=== comparison ===")
        for label, geo in summary.items():
            print(f"{label:40} EBF {geo:.2f}" if geo else f"{label}: no data")

    if args.require_identical:
        if len(targets) != 2:
            sys.exit("--require-identical needs exactly two targets")
        left, right = targets[0][0], targets[1][0]
        print(f"\n=== move-identical: {left} vs {right} ===")
        diffs = compare(measured[left], measured[right], args.depth)
        for line in diffs:
            print(line)
        if diffs:
            sys.exit(f"not move-identical: {len(diffs)} mismatch(es)")
        print("identical node counts and PV at every measured depth")


if __name__ == "__main__":
    main()
