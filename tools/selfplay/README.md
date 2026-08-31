# Self-play A/B testing

`sprt.sh` plays two turox builds against each other and reports whether the
candidate is actually stronger, using a sequential probability ratio test
(SPRT) over real games rather than a read-through of the diff.

Anything that changes how the engine chooses moves (evaluation terms, search
pruning, move ordering, time management) should go through this before it is
called an improvement. Reading a search change and deciding it looks better
has no error bars on it; a few hundred games does.

This is external tooling, deliberately: `fastchess` is a separate program
invoked from a script, not a crate the workspace depends on. `turox-engine`
takes zero runtime dependencies, and a test harness is not a reason to make
an exception.

## Requirements

`fastchess`, on `PATH` or pointed at by `$FASTCHESS`. There is no Homebrew
formula for it, so build it from source (it needs only a C++17 compiler and
`make`, and produces a single self-contained binary):

```sh
git clone https://github.com/Disservin/fastchess.git
cd fastchess
make -j
```

Regenerating the opening suite additionally needs `python-chess`
(`pip install chess`). Running a match does not: `openings.epd` is checked in.

## Running a match

Defaults compare the current working tree, uncommitted changes included,
against `main`:

```sh
FASTCHESS=/path/to/fastchess tools/selfplay/sprt.sh
```

Both `--base` and `--test` take a git ref, a path to an already-built binary,
or the literal `worktree`:

```sh
# a branch against the commit it forked from
tools/selfplay/sprt.sh --base main --test pawn-structure-eval

# two specific commits, capped so it finishes inside an afternoon
tools/selfplay/sprt.sh --base 51109b2 --test HEAD --rounds 600

# binaries that already exist, no building
tools/selfplay/sprt.sh --base ./old-turox --test ./target/release/turox-cli
```

A git ref is exported with `git archive` into `target/selfplay/src/<sha>` and
built there with its own `CARGO_TARGET_DIR`, so the working tree is never
touched and repeated runs against the same baseline reuse a warm cache. Both
binaries are then copied to `target/selfplay/bin/` before the match starts, so
a `cargo build` in another terminal cannot swap one out mid-match. Games are
written to `target/selfplay/results/`.

`sprt.sh --help` lists every option.

## Reading the result

fastchess reprints a summary block after every game:

```
Results of base vs test (10+0.1, NULL, NULL, openings.epd):
Elo: -21.74 +/- 96.80, nElo: -39.34 +/- 170.24
LOS: 32.53 %, DrawRatio: 37.50 %, PairsRatio: 0.67
Games: 16, Wins: 3, Losses: 4, Draws: 9, Points: 7.5 (46.88 %)
Ptnml(0-2): [0, 3, 3, 2, 0], WL/DD Ratio: 0.50
LLR: -0.11 (-3.8%) (-2.94, 2.94) [0.00, 10.00]
```

The `LLR` line is the verdict. It is the log-likelihood ratio, its two
bounds, and the elo0/elo1 being tested. The match stops on its own when the
LLR crosses a bound:

- **above the upper bound**: H1 accepted, the change is worth at least `elo1`.
- **below the lower bound**: H0 accepted, the change is not worth `elo1`.
  This is a rejection, not a proof that the change is harmful.
- **neither, and the rounds ran out**: inconclusive. Raise `--rounds`, or
  accept that the change is too small to resolve at this time control.

Everything above the LLR line describes the games rather than deciding
anything. `Elo` with its error bar is the point estimate. `Ptnml(0-2)` counts
the paired results (both games lost, one loss and a draw, level, and so on),
which is the statistic the pairing exists to make available: pairing the two
colors from each opening removes most of the variance that opening choice
would otherwise contribute.

Watch for a `Timeouts` or `Crashed` count in the per-player block at the end,
and for games terminated by an illegal move. Any of the three means the match
measured a bug rather than a strength difference, and the Elo number is not
worth reading until it is fixed.

An `Illegal move 0000` in particular is the engine answering `go` with no
move at all. It happens when the search budget runs out before the first
iterative-deepening iteration finishes, since the result of an interrupted
iteration is correctly discarded and there is nothing behind it to fall back
on. A too-small `--nodes` value provokes it directly (1000 nodes per move is
enough to hit it from ordinary opening positions; 100000 is not), so keep the
budget somewhere the engine can finish a first iteration in.

## Why the defaults are what they are

### fastchess, not cutechess-cli

Both run SPRT matches and take the same broad option vocabulary. fastchess
builds from source with `make` and a C++17 compiler and nothing else, where
cutechess-cli wants a Qt toolchain; neither has a Homebrew formula, so ease of
building from source decides it. fastchess is also what current engine
development mostly uses, so its output format is the one other people's advice
is written against.

### An opening suite, not repeated games from the start position

`openings.epd` holds 2494 distinct start positions. Every round of a match
plays one of them twice, once with each engine as White.

This is not a nicety. turox has no opening book and its search is
deterministic given a fixed budget: from the same position with the same node
budget it plays the same game every time, move for move. A match seeded from
`startpos` would replay one game for as long as it was left running and report
a confident-looking verdict resting on a single sample. Under a wall-clock
time control the games are not bit-identical, since the deadline lands at
different points, but they are still heavily correlated for the same reason.

fastchess wraps around the book (`index % book_size`) once the openings run
out, so `sprt.sh` caps `--rounds` at the suite size and says so when it does.
That ceiling is 2494 rounds, or 4988 games.

The positions come from
[lichess-org/chess-openings](https://github.com/lichess-org/chess-openings),
released under CC0 1.0 (public domain dedication), pinned to a single upstream
commit in `generate-openings.py`. That data set is a list of named opening
lines as move text; the generator replays each one, keeps the lines that are 4
to 12 plies long, drops transposed duplicates and anything that starts one
side more than a pawn down, and writes the resulting positions as EPD.

Four plies is the lower bound because there are only twenty legal first moves,
and a suite that keeps handing out `1. e4` is barely more varied than
`startpos`. Twelve is the upper bound because past that the book is doing more
of the playing than the engine is, and the deeper named lines skew toward
sharp theory that a sub-1500 engine cannot handle sensibly. Nothing filters
for objective balance beyond the material check; playing every position from
both sides is what handles that, and it handles it exactly rather than
approximately.

To regenerate:

```sh
pip install chess
python3 tools/selfplay/generate-openings.py
```

### 10+0.1 by default

Ten seconds per side plus a tenth of a second per move is the standard fast
time control in engine testing, so it is a number other people's intuitions
are already calibrated against. On turox it buys roughly depth 4 to 5 in the
opening: measured from the start position, 400ms reaches depth 4 and one
second reaches depth 5, at around 130k nodes per second.

Faster than this runs into the engine's own time management rather than its
playing strength. At 1+0.01 a game in testing was lost on time to an 87ms
overrun, because `search::time::allocate_time` budgets `time_left / 30 +
increment` with nothing held back for per-move overhead. `--timemargin`
(default 200ms) is what keeps that from turning ordinary scheduling jitter
into scored losses; if a run reports timeouts anyway, the time control is too
fast for this engine, not the other way around.

A game at 10+0.1 takes something like 25 to 40 seconds of wall clock, so two
cores get through roughly 200 to 300 games an hour. Budget accordingly: an
SPRT that needs 1000 games is most of an afternoon.

Two alternatives worth knowing about:

- `--tc 60+0.6` to confirm at a slower control something that already passed
  at 10+0.1. Changes that help at shallow depth sometimes stop helping at
  deeper ones, and only a slower match will say so.
- `--nodes 100000` to give both engines a fixed node budget per move instead
  of a clock. That makes the games fully deterministic and immune to whatever
  else the machine is doing, which is the lower-variance way to A/B an
  evaluation change. The cost is that it measures nothing about time
  management, and a change that only makes the search faster scores exactly
  zero.

### elo0=0, elo1=10, alpha=beta=0.05

H0 is "worth nothing", H1 is "worth at least 10 Elo", with a 5% chance of
wrongly accepting either.

The bound of 10 is the judgment call. Stockfish tests at [0, 2] because it is
hunting gains of a couple of Elo and can spend tens of thousands of games
finding them. turox is nowhere near that: the changes actually queued up
(pawn structure terms, tapered evaluation, null-move pruning, late move
reductions, king safety) are each plausibly worth tens of Elo, and a [0, 2]
test would need on the order of 30k games to resolve, which is weeks on two
cores. [0, 10] resolves a genuine 25 Elo gain in a few hundred pairs, which is
an afternoon, and that is the difference between a harness that gets used
before every tuning change and one that does not.

The 5% error rates are the conventional choice. Tightening them costs games
this machine does not have; loosening them lets noise through on exactly the
decisions this harness exists to make.

`model=logistic` rather than fastchess's default normalized nElo, so `--elo0`
and `--elo1` mean the plain Elo the rest of the project talks in.

Two variations to reach for:

- **As gains get smaller**, tighten to `--elo1 5`. Expect the game count to
  roughly quadruple.
- **For a refactor that is supposed to be strength-neutral**, invert it:
  `--elo0 -10 --elo1 0`. Accepting H1 there means the change did not cost 10
  Elo, which is the actual claim being made.

### Adjudication

`-maxmoves 200`, a draw call after 8 consecutive moves under 10cp from move
40, and resignation after 5 consecutive moves past 1000cp. Adjudication is
about not spending the clock on games whose result is already settled.

Resignation is two-sided, so both engines have to agree the position is lost
before the game is called. That matters here specifically because the two
engines under test differ in how they evaluate, which is the whole point of
the match: a one-sided resignation rule would let the build with the more
optimistic evaluation talk the match into results neither engine actually
played out.

## Known limitations

- turox exposes no UCI options, so there is no `option.Hash` or `option.Threads`
  to set. Any test that needs to vary engine behavior at runtime needs a real
  `setoption` implementation in the engine first.
- Match results live under `target/selfplay/`, which `cargo clean` removes.
  Pass `--pgnout` somewhere else for a run worth keeping.
- The suite size caps a single match at 4988 games. That is more than this is
  likely to be run for, but it is a hard ceiling, not a default.
