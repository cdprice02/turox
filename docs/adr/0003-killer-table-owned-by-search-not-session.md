# Killer-move table is owned by `search::Search`, not `uci::session`

The killer table lives inside `Search`, rebuilt fresh on every `go` command
the same way `history`, `deadline`, and `stop` already do, rather than
hoisted to `session::run` the way ADR-0001 hoists the transposition table.

## Considered options

ADR-0001 sets a precedent that could read as "search state that matters
lives at the session level," which would point at hoisting the killer table
too. But the reasoning there was specific to the TT: a transposition is
keyed by position, and the same position recurs across *separate* `go`
calls within a game, so a table that doesn't survive past one `go` call
never sees the transpositions that matter most. A killer is keyed by ply
within *one* search tree's sibling structure; the tree is different every
`go`, so a killer surviving into the next `go` call is exactly the stale
information the killer-heuristic literature says to discard ("cleared per
search, not per node"). The two tables look alike (both ply/position-scoped
caches attached to search) but the reuse argument that justifies hoisting
one doesn't hold for the other.
