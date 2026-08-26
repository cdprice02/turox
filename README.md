# turox

Forged in Rust. Inspired by Turing.

A chess engine written from scratch as a hands-on exercise in bit-manipulation
(bitboards, magic numbers, Kogge-Stone parallel-prefix tricks) and
benchmark-driven Rust performance work. Chess is the vehicle, not the point:
the goal is real practice with profiling, `target-feature` tuning, and
iterating against actual measurements rather than guesses.

The concrete target that gives the perf work something to point at: get the
engine talking UCI well enough to run on [lichess](https://lichess.org) via
[`lichess-bot`](https://github.com/lichess-bot-devs/lichess-bot) and produce a
real rating.

## Status

Move generation is complete and verified end-to-end against perft (see
below): `Board`, FEN parsing/formatting, attack tables, magic bitboards, and
pseudolegal/legal move generation all work. `search`, `eval`, and `uci` are
not yet implemented — the engine can enumerate every legal move from a
position, but nothing yet decides which one to play or speaks UCI to report
it.

## Architecture

```
types  ->  board  ->  move_gen  ->  search / eval / uci
```

- **`types`** — core value types (`Bitboard`, `Square`, `Color`, `Piece`,
  `Move`, ...) with no dependency on `Board`. Sits at the crate root rather
  than nested under `board/` because move generation, search, and evaluation
  all need these types without depending on `Board` itself; re-exported at
  the crate root too, so callers write `turox_engine::Bitboard` rather than
  reaching into the module.
- **`board`** — `Board` (piece placement plus game state) and FEN
  parsing/formatting, built on `types`.
- **`move_gen`** — attack tables, magic bitboards, pseudolegal and legal move
  generation, and `perft`.
- **`search`**, **`eval`**, **`uci`** — planned. Iterative deepening over a
  transposition table, static position evaluation, and the UCI protocol that
  will drive the engine from `turox-cli`.

`turox-engine` takes zero runtime dependencies, deliberately — see the
"Dependency policy" comment in `turox-engine/Cargo.toml`.

## Building and running

```sh
cargo build --workspace
cargo run -p turox-cli
```

## Testing

```sh
cargo nextest run --workspace
```

The suite is almost entirely property tests (`proptest`) checked against
independently-built reference implementations, plus concrete FEN-based
scenarios for rule-heavy or stateful logic (castling, en passant, promotion,
check/pin detection). See `CLAUDE.md` for the collaboration pattern this
follows.

A handful of tests — the deepest `perft` depths, which push into the
millions of nodes — are `#[ignore]`d by default so the default suite stays
fast. Run them deliberately, in `--release` (the default `dev` profile is far
too slow for perft at depth):

```sh
cargo nextest run --workspace --release --run-ignored all
```

`perft` (performance test) is the project's end-to-end correctness gate: a
standard recursive node count over a legal-move search tree, checked against
the published results for six standard test positions
(`turox-engine/tests/perft.rs`) from
[chessprogramming.org](https://www.chessprogramming.org/Perft_Results). A
wrong count at low depth on any of them localizes to a specific rule; matching
all six, including the deep depths, is the bar for "move generation is
actually correct."

## Benchmarking

```sh
cargo bench -p turox-engine
```

Every benchmark drives over a precomputed corpus of varied inputs and
`black_box`es both the inputs and the returned value, so LLVM can't
const-fold a fixed input away and report a meaningless ~0ns. Compare against
a saved baseline before/after a change:

```sh
cargo bench -p turox-engine -- --save-baseline before
# ...change...
cargo bench -p turox-engine -- --baseline before
```

CI only checks that benches still compile (`cargo bench --workspace --no-run`)
on every push; it does not run them, since GitHub's shared runners aren't
consistent enough run-to-run for the numbers to mean much there.

## Development loop

[`bacon`](https://dystroy.org/bacon/) drives the local edit/check loop (see
`bacon.toml`): `bacon` alone runs clippy on save; `bacon nextest` runs the
test suite; `bacon nextest -- -- <name>` runs one test; `bacon nextest --
--run-ignored ignored-only` runs the deep perft depths on demand; `bacon run`
runs `turox-cli` in the background.

CI (`.github/workflows/rust-ci.yml`) runs the same checks on every push:
build, `cargo nextest run --workspace --profile ci`, `cargo fmt --check`,
`cargo clippy -D warnings`, and a rustdoc build with warnings denied.
