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
"""

import argparse
import re
import subprocess
import statistics
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BUILD_ROOT = REPO / "target" / "treeshape"

# Deliberately small and varied: the start position for comparability with
# published figures, Kiwipete because it is the standard wide-open middlegame
# and already the perft suite's second position, plus a quiet middlegame and a
# pawn endgame so the number is not dominated by one tree shape.
POSITIONS = [
    ("startpos", None),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ),
    (
        "midgame",
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
    ),
    ("endgame", "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1"),
]

INFO = re.compile(r"^info depth (\d+).*?\bnodes (\d+)(?:.*?\btime (\d+))?", re.M)


def build(ref: str) -> Path:
    """Build `ref` into its own target dir and return the binary."""
    src = BUILD_ROOT / "src" / ref.replace("/", "_")
    src.mkdir(parents=True, exist_ok=True)
    sha = subprocess.run(
        ["git", "rev-parse", "--short", ref], cwd=REPO,
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    subprocess.run(
        ["git", "--work-tree", str(src), "checkout", ref, "--", "."],
        cwd=REPO, check=True, capture_output=True,
    )
    subprocess.run(
        ["cargo", "build", "--release", "-p", "turox-cli"],
        cwd=src, check=True, capture_output=True,
    )
    out = BUILD_ROOT / "bin" / f"{ref.replace('/', '_')}-{sha}"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes((src / "target" / "release" / "turox-cli").read_bytes())
    out.chmod(0o755)
    return out


def run_position(binary: Path, fen, depth: int):
    """{depth: (nodes, ms)} for one position, from a single search.

    One `go depth N` reports every completed iteration, so the whole curve
    comes from one search rather than N of them. That also makes the numbers
    honest about iterative deepening: each depth's node count includes the
    shallower iterations that preceded it, which is what a real search pays.
    """
    pos = "position startpos" if fen is None else f"position fen {fen}"
    # Randomization off, or the node counts this exists to compare vary by half
    # between runs of the same binary. Builds predating the option ignore the
    # line, per UCI's own convention for an unrecognized option name, so the
    # same script drives old and new builds alike.
    script = (
        "uci\nsetoption name Randomize value false\n"
        f"ucinewgame\n{pos}\ngo depth {depth}\nquit\n"
    )
    proc = subprocess.run(
        [str(binary)], input=script, capture_output=True, text=True, timeout=1800
    )
    out = {}
    for d, nodes, ms in INFO.findall(proc.stdout):
        out[int(d)] = (int(nodes), int(ms) if ms else 0)
    return out


def report(label: str, results: dict, depth: int):
    print(f"\n=== {label} ===")
    print(f"{'position':10} {'depth':>5} {'nodes':>12} {'time_ms':>8} {'nps':>9} {'EBF':>6}")
    ebfs = []
    for name, per_depth in results.items():
        prev = None
        for d in range(1, depth + 1):
            if d not in per_depth:
                continue
            nodes, ms = per_depth[d]
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", action="append", default=[], help="git ref to build and measure")
    ap.add_argument("--bin", action="append", default=[], help="prebuilt binary to measure")
    ap.add_argument("--depth", type=int, default=7)
    args = ap.parse_args()

    targets = []
    for r in args.ref:
        print(f"building {r} ...", file=sys.stderr)
        targets.append((r, build(r)))
    for b in args.bin:
        targets.append((b, Path(b)))
    if not targets:
        targets = [("worktree", REPO / "target" / "release" / "turox-cli")]

    summary = {}
    for label, binary in targets:
        if not binary.exists():
            sys.exit(f"missing binary: {binary}")
        results = {}
        for name, fen in POSITIONS:
            results[name] = run_position(binary, fen, args.depth)
        summary[label] = report(f"{label}  ({binary.name})", results, args.depth)

    if len(summary) > 1:
        print("\n=== comparison ===")
        for label, geo in summary.items():
            print(f"{label:40} EBF {geo:.2f}" if geo else f"{label}: no data")


if __name__ == "__main__":
    main()
