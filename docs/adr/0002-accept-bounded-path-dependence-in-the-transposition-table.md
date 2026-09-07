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
taken because it silently lowers how much the table retains, and there is
currently no instrumentation to price that. Once cutoff-rate and table
instrumentation exist, this becomes a cheap experiment rather than a guess.

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

Two `#[ignore]`d tests document the current behaviour, following the same pattern
the deep perft depths use. They are expected to fail. When the fix lands they
flip to passing and the attribute comes off, which makes them the acceptance
criteria rather than dead weight.
