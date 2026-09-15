# Accept bounded path-dependence in the transposition table

The table is keyed on board state and discards the move sequence that reached
it. Two consequences are known, present, and deliberately not fixed:

- **Graph history interaction.** The same position scores differently depending
  on the path taken to it, because whether a repetition is available is a
  property of the path, not the position.
- **The fifty-move clock is not in the key.** Two positions identical in pieces,
  side to move, castling rights and en passant square share an entry even when
  one is two plies from a fifty-move draw and the other is fifty moves away.

Both leak in the same place, and it is not the obvious one. The node being
searched is safe: `is_draw` runs before the table is probed, so a position that
is already drawn scores zero, can never take a cutoff, and is never stored. What
leaks is the **ancestors**, which store their score with no record of whether it
came from a subtree containing a draw.

Requiring a genuine threefold, rather than scoring the first repetition inside
the tree as many engines do, is a real mitigation: far fewer nodes carry a
path-dependent zero.

## Considered options

**Fixing it properly.** The published solutions (Kishimoto and Müller) exist and
are general. They also cost more than they return at this engine's strength: the
error shows up as half points in specific endgames, not as general weakness, and
the engine currently loses games to things measured in whole pawns. Sequencing
work by what it buys means this loses to almost everything.

**The cheap defensive half.** Suppressing the store when a subtree returned a
repetition draw, or carrying a flag that does, closes most of the ancestor
window for very little code. This is the closest call of the four. It was not
taken because it silently lowers how much the table retains, and there was no
instrumentation to price that.

That instrumentation now exists. `Tt::hashfull` reports occupancy and
`CutoffStats` reports cutoff rate and cause, both on every `info` line, so the
cost this option was deferred over is now measurable rather than guessed at.
The experiment it was waiting for is therefore available, and specified: apply
the suppression, then compare occupancy, first-move cutoff rate, and
fixed-depth node counts against the same positions without it. A suppression
that costs little on all three is worth taking; one that visibly empties the
table is the reason this was not taken in the first place, and would say so.

What it is *not* is free to land. It changes which entries exist, so it changes
the tree and needs the same self-play gate any search change does. The condition
this ADR set has been met, and the work it unblocks is queued rather than done.

**Clearing the table between `go` commands.** This eliminates both problems
outright, because the cross-`go` lifetime is what widens them: an entry stored
during move 20's search gets probed during move 24's, with four more positions
on the repetition stack and the halfmove clock eight plies further along. It is
also the simplest possible fix. It is rejected because that lifetime is the
entire point of ADR 0001, which exists because a table that does not outlive its
`go` call never sees the transpositions that matter most in real play. Trading a
measured benefit for an unmeasured correctness gain is the wrong direction.

## Consequences

The engine can, in principle, report a draw that is not available or miss one
that is. In practice both require a specific shape: a repetition or fifty-move
boundary reachable on one path and not another, with the relevant entry
surviving in the table between the two.

This decision has an expiry date, which is the main reason for writing it down.
The bound is invisible at current strength; it does not stay invisible. Null-move
pruning and late move reductions both increase how much gets stored, which widens
the window, and draw-related errors become proportionally more expensive as the
engine stops losing games for simpler reasons.

One widening arrived from an angle this list did not anticipate. Pin-set legal
move generation, and probing the table before generating moves, made the engine
search more nodes in the same budget. More nodes searched is more entries stored
and more churn, which is the same pressure null-move pruning and reductions were
named for, without any of them being implemented. The lesson for anyone reading
this list as a checklist: "the engine got faster" belongs on it.

Two tests document the current behaviour and are expected to fail:
`accepted_gap_shared_table_leaks_a_stale_score_across_a_fifty_move_boundary` and
`accepted_gap_shared_table_leaks_a_repetition_tainted_score_across_go_commands`.
They are the acceptance criteria rather than dead weight: when the fix lands
they flip to passing.

They are kept out of ordinary runs twice over, and both are load-bearing. The
`accepted_gap_` prefix is what the default nextest filter excludes, which is what
the weekly deep job needs, since it passes `--run-ignored all` and so defeats the
attribute. `#[ignore]` is what plain `cargo test` honours, which is what
`cargo llvm-cov` and `cargo mutants` shell out to. A weekly `accepted-gaps` job
runs exactly these and **inverts the result**, so it is green while the gap
persists and red when one *passes*. A permanently red job teaches everyone to
ignore it, and then it cannot report the one thing it exists to report.
