# Testing

Where a test goes, which technique it should use, and what the mutation and
coverage jobs are for.

## Where a test goes

**In-src `#[cfg(test)] mod tests` for private items.** This is forced, not
chosen: `tests/*.rs` compiles as a separate crate and sees only `pub` items, so
a private helper has nowhere else to be tested. `eval::king_safety`'s
`zone_files`, `shelter_penalty` and `storm_penalty` are private `const fn`s;
`move_gen::magic`'s `Magic`, `magic_index` and the whole search in `regen.rs`
are private too.

**`tests/<source_file>.rs` for public items.** The file is named after the
source file it covers, so finding the test for a given line of code is a
derivation rather than a search. A survivor reported as
`eval/pawn_structure.rs:NN` has exactly two places to look: that module's own
in-src tests, and `tests/eval_pawn_structure.rs`.

Name the file after the source, not after the technique. Proptest, golden,
snapshot and metamorphic tests for one source file share one test file, and say
which is which in the module doc and in test names. A technique suffix stops
scaling as soon as a second technique applies to the same code, and it answers a
question nobody asks: what triage needs is which source file a test covers.

A test that genuinely spans modules is named after the thing it exercises
(`tests/perft.rs`, `tests/uci_session.rs`), not after one of the files it
touches.

## Which technique

| Logic | Technique | Why |
| ------ | ---------- | ---- |
| Rules with an independent definition | Naive reference | Attacks, move generation, legality, `between`/`line`, bitboard ops: a second implementation built from the definition rather than from the fast path's precomputed structure. See `CONTEXT.md` for what does and does not qualify. |
| Published ground truth | Replay against it | Perft counts, and real game movetext for SAN and PGN. The strongest evidence available, and it requires no cleverness. |
| Search selectivity | Golden tree-shape | A pruned and an unpruned search return the same score by construction, so an oracle cannot see a reduction or re-search condition change. What changes is which nodes were visited, so node counts and the principal variation are the observable. |
| Symmetries | Metamorphic | Colour mirroring, file mirroring, move-order permutation. Holds for any weights, so it survives retuning. |
| Rendered output | Snapshot | UCI responses, diagnostic summaries, PGN and SAN rendering. |
| Tuned weights | Concrete positions | A number self-play chose has no independent truth to check against, so a naive reference can only assert someone updated both copies. Pin what a weight is worth with hand-picked positions instead. |

The naive reference is the default for the rules layer and nothing else. It has
nothing to offer for selectivity, where both sides agree by construction, and
nothing for formatting.

## Two rules the mutation data produced

**A property is only as good as its generator.** `is_pseudo_legal` holds the
right property, in both directions, and leaked 19 mutants anyway. Its candidate
move was drawn as three independent uniform values (`any_square`, `any_square`,
`any_move_flags`), which is 57,344 combinations sampled 256 times, so a
near-miss essentially never appeared and flipping `||` to `&&` deep in the
branch logic still rejected the obviously-absurd move.

When the input has a grammar, generate by **perturbing a valid value**, not by
sampling the space: take a real pseudo-legal move and change one square, or one
flag, or add the blocker that should disqualify it. Mix that with the uniform
draw rather than replacing it. Neither line coverage nor a passing property test
can see this, which is why it went unnoticed at 82.6% coverage.

**Assert the whole error value, not its discriminant.** `matches!(err,
WrongRankCount { .. })` cannot see a payload, so the rank and file numbers
inside a FEN error were free to be wrong: eight surviving mutants lived in those
payloads. Prefer `assert_eq!` against the constructed error. And check the
variant is reached at all; the eight survived partly because nothing anywhere
constructed `TooManyFilesInRank` or `NotEnoughFilesInRank`.

## What runs where

`docs/agents/toolchain.md` has the commands. The division:

- **The pull-request gate** runs the whole suite. A test belongs here unless it
  cannot be made to finish quickly, which in practice means deep perft and the
  full magic search.
- **The weekly job** runs what the gate cannot afford, plus the informational
  measures: mutation, coverage, benchmarks. None of it gates a merge. A
  surviving mutant is a prompt to add a test.
- **`accepted_gap_`-prefixed tests** assert the behaviour the engine *owes* a
  position rather than the behaviour it has, pinning a gap an ADR has
  consciously accepted. The weekly job that runs them errors when one
  **passes**, so a gap that gets closed announces itself.

Mutation results are read from the `mutants-summary` job's own output, not by
downloading artifacts. It reports a score and which files owe a test.

Two things it says that are easy to misread. A **timeout** is neither a pass nor
a fail, so it is counted beside survivors: both mean the file owes a test. And
the score is **per crate**, since each mutant is tested against only its own
crate's tests: a `turox-chess` survivor means `turox-chess`'s tests miss it,
whatever `turox-engine`'s tests might happen to catch.

Coverage is close to saturated and has little left to say on its own. Treat a
line it reports as uncovered as a real gap, and treat a covered line as
unproven.

## Unkillable mutants

Some mutants are semantically equivalent and no test can kill them.
`knight_attacks`'s `|` and `^` agree because the input is always a single bit, so
the terms being combined are disjoint.

Mark those with `#[mutants::skip]` at the smallest scope that works, and give the
reason in a doc comment. The reason is the point: it is usually an invariant the
code depends on without stating, which is worth writing down for its own sake.
Structural exclusions, for code whose behaviour nothing should depend on, go in
`.cargo/mutants.toml` instead.

Do not skip a mutant that survives because nothing covers it. That is a test
gap, and it belongs in the report.
