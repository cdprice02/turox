# Transposition table is owned by `uci::session`, not `search::Search`

`Search` is rebuilt fresh on every `go` command (`build_search` in
`uci/session.rs`), the same way `history` lives in `session::run`'s loop and
gets passed in rather than owned across calls. The transposition table
follows that same pattern instead of living inside `Search`: it's created
once in `session::run`, threaded into each `Search` call, and cleared on
`ucinewgame`.

## Considered options

Owning the table inside `Search` was the more obvious choice, since every
other piece of search state (`history`, `deadline`, `max_nodes`, `stop`)
already lives there. But a table that doesn't survive past the `go` call
that created it never sees the transpositions that matter most in real
play: different move orders reaching the same position across *separate*
`go` calls in the same game, not just within one search tree. Rebuilding it
per call would compile and pass its own tests while quietly providing none
of the benefit the issue that motivated it was written for.
