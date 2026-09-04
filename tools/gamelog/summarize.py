#!/usr/bin/env python3
"""Summarize turox's lichess games from lichess-bot's PGN archive.

Answers the questions the strength backlog actually asks: what depth is the
engine reaching at each time control, how is its rating moving in each
separate pool, and what shape are its losses. Written because reconstructing
those by hand from the PGNs is what the first evidence session cost.

Loss shape is the interesting one. A *collapse* is an eval that holds steady
and then falls off a cliff, which is the signature of a missing king-safety
term: the position looks fine right up until the mating net is inside the
search horizon. A *drift* is an eval that erodes gradually, which points at
positional terms instead. Telling those apart across many games is what
decides which evaluation work is worth doing first.

Usage: tools/gamelog/summarize.py [pgn-dir]
"""

import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

DEFAULT_DIR = Path.home() / "repos" / "lichess-bot" / "game_records"
BOT = "turox-bot"
# A jump this large in one move is a blunder or a collapse rather than drift.
COLLAPSE_CP = 300


def tag(text, name):
    m = re.search(rf'\[{name} "([^"]*)"\]', text)
    return m.group(1) if m else None


def classify_speed(tc):
    """Lichess buckets by estimated duration: initial + 40 * increment."""
    try:
        base, inc = (int(x) for x in tc.split("+"))
    except (ValueError, AttributeError):
        return "unknown"
    est = base + 40 * inc
    if est < 179:
        return "bullet"
    if est < 479:
        return "blitz"
    if est < 1499:
        return "rapid"
    return "classical"


def evals_white_pov(text):
    """(centipawns, depth, is_mate) per annotated move.

    Mate scores are flagged rather than folded into the centipawn scale. A mate
    is not a number on the same axis: treating `#1` as +10000 makes the step
    from any ordinary eval into a mate look like a ~9000 centipawn swing, which
    would make every checkmate register as an eval collapse and tell you
    nothing about whether the eval saw it coming.
    """
    out = []
    for raw, depth in re.findall(r"%eval (#?-?[0-9.]+),(\d+)", text):
        if "#" in raw:
            sign = -1 if raw.lstrip("#").startswith("-") or raw.startswith("-") else 1
            out.append((sign * 10000, int(depth), True))
        else:
            out.append((int(round(float(raw) * 100)), int(depth), False))
    return out


def loss_shape(ev, turox_is_white):
    """Largest single-move swing against turox between two ordinary evals.

    Pairs involving a mate score are skipped: the transition into a mate is a
    change of units, not a measurable swing, so including it would report a
    collapse for every game that ended in checkmate regardless of how the eval
    behaved beforehand.
    """
    worst, at = 0, 0
    for i in range(1, len(ev)):
        prev_cp, _, prev_mate = ev[i - 1]
        cp, _, mate = ev[i]
        if prev_mate or mate:
            continue
        delta = cp - prev_cp
        against = -delta if turox_is_white else delta
        if against > worst:
            worst, at = against, i
    return (worst, at, len(ev)) if worst else None


def main():
    d = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_DIR
    games = sorted(d.glob("*.pgn"), key=lambda p: p.stat().st_mtime)
    if not games:
        print(f"no PGNs in {d}")
        return

    pools = defaultdict(list)
    rows = []
    for path in games:
        text = path.read_text(errors="replace")
        white, result = tag(text, "White"), tag(text, "Result")
        if white is None:
            continue
        turox_is_white = white == BOT
        side = "White" if turox_is_white else "Black"
        tc = tag(text, "TimeControl") or "?"
        speed = classify_speed(tc)
        elo = tag(text, f"{side}Elo")
        diff = tag(text, f"{side}RatingDiff")
        opp_elo = tag(text, "BlackElo" if turox_is_white else "WhiteElo")

        if result == "1/2-1/2":
            outcome = "draw"
        elif (result == "1-0") == turox_is_white:
            outcome = "WIN"
        else:
            outcome = "loss"

        ev = evals_white_pov(text)
        depths = [d_ for _, d_, _ in ev]
        shape = loss_shape(ev, turox_is_white)
        mated = any(m for _, _, m in ev)

        if elo and diff:
            pools[speed].append((int(elo), int(diff)))

        rows.append(
            dict(
                name=path.stem[:34], speed=speed, tc=tc, side=side[0],
                outcome=outcome, elo=elo, diff=diff, opp=opp_elo,
                med=int(statistics.median(depths)) if depths else 0,
                mx=max(depths) if depths else 0, shape=shape, mated=mated,
            )
        )

    print(f"{len(rows)} games in {d}\n")
    hdr = f"{'game':36}{'speed':8}{'tc':8}{'s':3}{'result':8}{'elo':6}{'diff':6}{'opp':6}{'med':5}{'max':5}  loss shape"
    print(hdr)
    print("-" * len(hdr))
    for r in rows:
        sh = ""
        if r["outcome"] == "loss" and r["shape"]:
            worst, at, total = r["shape"]
            kind = "collapse" if worst >= COLLAPSE_CP else "drift"
            sh = f"{kind} {worst/100:+.1f} @ {at}/{total}"
            if r["mated"]:
                sh += ", mated"
        print(
            f"{r['name']:36}{r['speed']:8}{r['tc']:8}{r['side']:3}{r['outcome']:8}"
            f"{str(r['elo'] or '-'):6}{str(r['diff'] or '-'):6}{str(r['opp'] or '-'):6}"
            f"{r['med']:<5}{r['mx']:<5}  {sh}"
        )

    print("\nrating pools (each settles separately from its provisional seed)")
    for speed, entries in sorted(pools.items()):
        latest_elo, latest_diff = entries[-1]
        settled = latest_elo + latest_diff
        print(
            f"  {speed:10} {len(entries):2} rated games   latest {settled:5}"
            f"   last swing {latest_diff:+5}"
        )

    losses = [r for r in rows if r["outcome"] == "loss" and r["shape"]]
    if losses:
        collapses = sum(1 for r in losses if r["shape"][0] >= COLLAPSE_CP)
        worst = sorted((r["shape"][0] for r in losses), reverse=True)
        print(
            f"\nloss shape (mate transitions excluded): {collapses}/{len(losses)} losses"
            f" show a single-move swing >= {COLLAPSE_CP/100:.1f}"
        )
        print(
            "  worst swings: "
            + ", ".join(f"{w/100:+.1f}" for w in worst[:8])
        )


if __name__ == "__main__":
    main()
