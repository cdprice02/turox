# turox

A chess engine. This glossary holds only terms turox uses in a way the chess
programming literature doesn't; standard chess vocabulary (perft, quiescence,
pseudo-legal, en passant, magic bitboards, SPRT) belongs to that literature,
not here.

## Language

**Naive reference**:
A second, deliberately slow implementation of some logic, kept alongside a
fast one purely so a property test can assert the two agree. Built by
copy-make or direct recomputation rather than any of the fast path's
precomputed structure (pin sets, magic tables, incremental state), so it
stays obviously correct by inspection even as the fast path gets cleverer.
Never deleted once the fast path lands; deleting it would remove the fast
path's only independent check.
Shares no *reasoning* with the thing it checks: not the walk, not the order of
cases, not the dispatch. It may share *magnitudes*, and for a tuned weight it
should: a number self-play chose has no independent truth to check against, so
a second copy of it only ever tests that someone updated both, and turns every
retune into a two-place edit. What a weight is worth is pinned separately, by
structure rather than by transcription.
_Avoid_: Oracle (used for the reference itself elsewhere in chess literature,
but ambiguous here with "TT/eval oracle"), slow path, reference implementation
(too generic; "naive" is the load-bearing word).

**Session**:
The long-lived, stateful loop in `uci::session::run` that owns `Board`,
move history, and the transposition table for an entire game, across
every `go` command it receives. `Search` is deliberately rebuilt fresh
on every `go` and holds none of that state itself; the session creates
the transposition table and history once and threads them into each
`Search` call, so a transposition found during one `go` is still visible
during the next. See
`docs/adr/0001-transposition-table-owned-by-session-not-search.md`.
_Avoid_: engine loop, game loop (session is the codebase's own name for
this piece)

**Bounded path-dependence**:
The transposition table's accepted, tracked correctness gap: an entry is
keyed on board state alone, with no record of the path that reached it,
so the same position can score differently depending on whether a draw
(by repetition or the fifty-move clock) was reachable from an ancestor
along one path but not another. Wider than graph history interaction,
the term from the chess-programming literature that covers only the
repetition case; turox's term also covers the fifty-move clock sharing
an entry across two paths at different distances from the clock. Written
down as an ADR because the acceptance has an expiry date: the gap is
invisible at the engine's current strength and will stop being invisible
as search gets deeper and stronger. See
`docs/adr/0002-accept-bounded-path-dependence-in-the-transposition-table.md`.
_Avoid_: graph history interaction, GHI (narrower published term; covers
only the repetition half of what turox tracks under this name)
