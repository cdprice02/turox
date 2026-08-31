#!/usr/bin/env python3
"""Regenerate `openings.epd` from the lichess-org/chess-openings data set.

The suite exists because turox's search is deterministic and the engine has
no opening book: two runs from the same start position play the identical
game move for move, so an SPRT match seeded from `startpos` would replay one
game thousands of times and report a confident verdict built on a single
sample. Every round of a match therefore needs its own start position.

Source data: https://github.com/lichess-org/chess-openings, released under
CC0 1.0 (public domain dedication). Pinned to one commit below so a
regeneration produces the same suite rather than drifting with upstream.

Requires `python-chess` (`pip install chess`) to replay the SAN move lists
into positions. That dependency is why the generated `openings.epd` is
checked in: running a match needs the suite, not this script.

Usage:
    python3 generate-openings.py [-o openings.epd]
"""

import argparse
import csv
import io
import pathlib
import urllib.request

import chess

# Pinned upstream revision, so regenerating reproduces the checked-in suite.
UPSTREAM_COMMIT = "4b8622759e7ae6f93f011cc6c83a3823401ab45e"
UPSTREAM_URL = "https://raw.githubusercontent.com/lichess-org/chess-openings/{}/{}.tsv"
ECO_VOLUMES = ("a", "b", "c", "d", "e")

# Ply bounds on a line to be usable as a start position. Below the lower
# bound the positions are too generic to spread games out (there are only
# twenty legal first moves, and a suite that keeps repeating "1. e4" against
# a deterministic engine repeats games). Above the upper bound the book is
# doing more of the playing than the engine is, and the deeper named lines
# skew toward sharp theory a sub-1500 engine cannot handle sensibly.
MIN_PLIES = 4
MAX_PLIES = 12

# Rough material values, used only to drop lines that start one side a piece
# down. Named opening theory includes plenty of lines that are objectively
# lost for someone; a pawn gambit is fine (matches are played with both
# colors from every position, so the imbalance cancels), a piece is not.
PIECE_VALUES = {
    chess.PAWN: 100,
    chess.KNIGHT: 320,
    chess.BISHOP: 330,
    chess.ROOK: 500,
    chess.QUEEN: 900,
}
MAX_MATERIAL_IMBALANCE = 100


def fetch_rows():
    """Yields (eco, name, pgn) for every opening in the upstream data set."""
    for volume in ECO_VOLUMES:
        url = UPSTREAM_URL.format(UPSTREAM_COMMIT, volume)
        with urllib.request.urlopen(url) as response:
            text = response.read().decode("utf-8")
        for row in csv.DictReader(io.StringIO(text), delimiter="\t"):
            yield row["eco"], row["name"], row["pgn"]


def material_imbalance(board):
    balance = 0
    for piece_type, value in PIECE_VALUES.items():
        balance += value * len(board.pieces(piece_type, chess.WHITE))
        balance -= value * len(board.pieces(piece_type, chess.BLACK))
    return abs(balance)


def position(pgn):
    """Replays a SAN move list into a `chess.Board`, or `None` if unusable."""
    board = chess.Board()
    for token in pgn.split():
        if token.endswith("."):
            continue
        try:
            board.push_san(token)
        except ValueError:
            return None
    return board


def build_suite():
    """Returns the deduplicated, sorted (fen, eco, name) suite."""
    seen = set()
    suite = []
    for eco, name, pgn in fetch_rows():
        board = position(pgn)
        if board is None:
            continue
        if not MIN_PLIES <= board.ply() <= MAX_PLIES:
            continue
        if board.is_game_over() or material_imbalance(board) > MAX_MATERIAL_IMBALANCE:
            continue
        # Transpositions: several named lines reach the same position, and a
        # duplicate start position is a duplicate game.
        key = board.board_fen() + " " + board.epd().split(" ", 1)[1]
        if key in seen:
            continue
        seen.add(key)
        suite.append((board.fen(), eco, name))
    # Sorted by ECO then name so the file's order is a property of the data,
    # not of dict iteration or network response order. Match ordering is
    # `fastchess -openings ... order=random`'s job, not the file's.
    suite.sort(key=lambda entry: (entry[1], entry[2]))
    return suite


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "-o",
        "--output",
        type=pathlib.Path,
        default=pathlib.Path(__file__).parent / "openings.epd",
        help="where to write the suite (default: openings.epd next to this script)",
    )
    args = parser.parse_args()

    suite = build_suite()
    with args.output.open("w") as out:
        for fen, eco, name in suite:
            # `fastchess -openings format=epd` hands the whole line to its FEN
            # parser, so the comment has to be a legal EPD operation rather
            # than a trailing bare string.
            out.write(f'{fen} c0 "{eco} {name}";\n')
    print(f"wrote {len(suite)} positions to {args.output}")


if __name__ == "__main__":
    main()
