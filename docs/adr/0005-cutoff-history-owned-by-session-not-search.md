# `CutoffHistory` is owned by `uci::session`, not `search::Search`

Unlike the killer table (ADR-0003), `CutoffHistory` is created once in
`session::run`, threaded into each `Search` call, and cleared on
`ucinewgame` alongside the transposition table, rather than rebuilt fresh
on every `go` command.

## Considered options

ADR-0003 just settled the opposite answer for a table that looks like this
one's sibling: both are move-ordering caches hanging off `Search`, and the
obvious move is to treat them the same way. But ADR-0003's reasoning was
specific to what a killer *is*: a fact about one search tree's sibling
structure, worthless the moment that tree is gone. History is a fact about
which moves tend to be good independent of any one tree, accumulated over
orders of magnitude more nodes than two killer slots, and the technique's
own aging step ("halve periodically so the table tracks the current
position rather than the whole game") presumes a table that survives past
a single `go` call; there is no "whole game" to age away from otherwise.
The reuse argument that justified hoisting the transposition table
(ADR-0001) holds here for the same reason it held there, even though it
didn't hold for killers.
