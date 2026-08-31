# turox

[![Rust CI](https://github.com/cdprice02/turox/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/cdprice02/turox/actions/workflows/rust-ci.yml)

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
pseudolegal/legal move generation all work. `eval` (material and
piece-square tables), `search` (negamax with alpha-beta, iterative
deepening, and quiescence), and `uci` (parsing commands, emitting
responses, and a real stdin/stdout session loop) all work too: `turox-cli`
speaks UCI end to end and can be driven by any UCI-speaking GUI. What's left
is connecting it to lichess via `lichess-bot` for a real rating (see
"Playing on lichess" below).

## Architecture

```
types  ->  board  ->  move_gen  ->  search / eval / uci
```

- **`types`**: core value types (`Bitboard`, `Square`, `Color`, `Piece`,
  `Move`, ...) with no dependency on `Board`. Sits at the crate root rather
  than nested under `board/` because move generation, search, and evaluation
  all need these types without depending on `Board` itself; re-exported at
  the crate root too, so callers write `turox_engine::Bitboard` rather than
  reaching into the module.
- **`board`**: `Board` (piece placement plus game state) and FEN
  parsing/formatting, built on `types`.
- **`move_gen`**: attack tables, magic bitboards, pseudolegal and legal move
  generation, and `perft`.
- **`eval`**: static position evaluation (material and piece-square tables).
- **`search`**: negamax with alpha-beta over iterative deepening and
  quiescence, driven by a depth or node budget (a transposition table is a
  later addition).
- **`uci`**: the UCI protocol: parsing commands, emitting responses, and the
  stateful session loop that drives the engine from `turox-cli`.

`turox-engine` takes zero runtime dependencies, deliberately; see the
"Dependency policy" comment in `turox-engine/Cargo.toml`.

## Building and running

Requires Rust 1.87 or later (`Bitboard::shl`/`shr` use `u64::unbounded_shl`/
`unbounded_shr`, stabilized in 1.87; see `rust-version` in each crate's
`Cargo.toml`).

```sh
cargo build --workspace
cargo run -p turox-cli
```

## Playing on lichess

The concrete goal stated at the top: get `turox-cli` running as a bot on
[lichess](https://lichess.org) via
[`lichess-bot`](https://github.com/lichess-bot-devs/lichess-bot), so
performance and correctness work has a real rating to point at instead of
just "the tests pass."

`lichess-bot` is a separate Python project. It isn't cloned into this repo
or added as a dependency; it drives `turox-cli` as an external UCI process
over stdin/stdout, the same way any UCI-speaking GUI would.

1. Build a release binary; the default `dev` profile is far too slow for
   real time controls:

   ```sh
   cargo build --release -p turox-cli
   ```

2. Clone `lichess-bot` elsewhere and follow its own setup instructions
   (Python environment, dependencies, API token).
3. Upgrading a Lichess account to a BOT account is irreversible and only
   works on an account with zero rated games; pick or create one
   deliberately, not the account you play on yourself.
4. Point `lichess-bot`'s `config.yml` at the release binary's directory,
   with `protocol: uci`. `turox-cli` needs no engine-specific UCI options:
   it reads `go`'s `wtime`/`btime`/`winc`/`binc`/`movestogo`/`movetime`/
   `depth`/`nodes` fields directly, so `lichess-bot`'s default time-control
   handling works unmodified.
5. Run `lichess-bot` and challenge it (or have it challenge another bot) at
   bullet or blitz. Whether the rating stabilizes across a session, rather
   than trending down, is the actual signal worth watching, more than any
   single game's result.

## Testing

```sh
cargo nextest run --workspace
```

The suite is almost entirely property tests (`proptest`) checked against
independently-built reference implementations, plus concrete FEN-based
scenarios for rule-heavy or stateful logic (castling, en passant, promotion,
check/pin detection).

A handful of tests, the deepest `perft` depths, which push into the
millions of nodes, are `#[ignore]`d by default so the default suite stays
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
on every push; it does not run them there, since GitHub's shared runners
aren't consistent enough run-to-run for the numbers to mean much on a PR. A
weekly scheduled job runs them for real, informationally (see below).

## Self-play A/B testing

```sh
tools/selfplay/sprt.sh --base main --test my-branch
```

Benchmarks say whether a change made the engine faster; they say nothing
about whether it made it play better. `tools/selfplay/sprt.sh` builds two
turox binaries and plays them against each other under a sequential
probability ratio test, so an evaluation or search change gets a pass/fail
answer from games instead of from inspection. It drives
[`fastchess`](https://github.com/Disservin/fastchess), a separate program
rather than a crate dependency, over a 2494-position opening suite.

See `tools/selfplay/README.md` for installing fastchess, for how to read an
SPRT verdict, and for the reasoning behind the time control and the test
bounds.

## Fuzzing

```sh
cd turox-fuzz
cargo +nightly fuzz run fen
```

Coverage-guided fuzzing of `Board::try_from_fen`, via
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (`cargo install
cargo-fuzz`; requires a nightly toolchain, so this isn't part of the stable
CI job, and runs on demand instead). `try_from_fen` is the one place the
engine takes untrusted input directly off the wire, since UCI's `position
fen <...>` command resolves through it: `Err` is a correct outcome for a
malformed string, a panic is not. `tests/fen_props.rs` already checks the
same property over proptest-generated inputs; this is the coverage-guided
version of it, for inputs a random regex won't reliably hit.

## Development loop

[`bacon`](https://dystroy.org/bacon/) drives the local edit/check loop (see
`bacon.toml`): `bacon` alone runs clippy on save; `bacon nextest` runs the
test suite; `bacon nextest -- -- <name>` runs one test; `bacon nextest --
--run-ignored ignored-only` runs the deep perft depths on demand; `bacon run`
runs `turox-cli` in the background.

CI (`.github/workflows/rust-ci.yml`) runs on every push: build, `cargo
nextest run --workspace --profile ci`, doctests, `cargo fmt --check`, `cargo
clippy -D warnings`, and a rustdoc build with warnings denied. A weekly
schedule (also runnable on demand via `workflow_dispatch`) additionally runs
what's too slow to gate every PR, informationally: the deep perft depths and
the full magic-bitboard re-search (`--release --run-ignored all`), `cargo
mutants -p turox-engine`, `cargo llvm-cov --workspace`, and a real `cargo
bench` run.
