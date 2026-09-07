# turox

turox is a chess engine. This glossary covers vocabulary specific to how
this codebase names its own concepts, not the broader chess-programming
literature (perft, quiescence, zobrist hashing, magic bitboards, SPRT,
and the like), which is used here in its standard, published sense and
is documented at chessprogramming.org rather than repeated here.

## Language

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
