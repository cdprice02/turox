# Chess primitives split into their own crate, and FEN stays with them

`turox-chess` holds the rules of chess (`types`, `board`, `move_gen`, `book`)
and `turox-engine` holds the part that plays (`search`, `eval`, `uci`), with the
dependency running one way and the compiler keeping it that way.
`turox-notation` (`pgn`, `san`) depends on `turox-chess` alone, so a tool that
parses notation or builds an opening book never compiles a search. FEN is the
exception that stays in `turox-chess`, alongside the positions it encodes.

## Considered options

**Leave `turox-engine` as one crate.** The measured case for splitting is thin:
a clean debug build of the engine is 22 seconds and an incremental rebuild is
under two, so the only beneficiaries (`turox-fuzz`, which needs `Board` and
nothing else, and `tools/bookgen`, where fifteen of seventeen engine imports are
primitives) save about eleven seconds on a build nobody waits on. The
primitives-never-depend-on-search rule was already true and had never been
violated in 26k lines, so a crate boundary would have been a mechanism guarding
a rule nobody breaks. Rejected on legibility rather than on the numbers: a crate
graph states the layering in a way a module tree cannot, and the numbers were
never the reason.

**Move FEN into `turox-notation` with PGN and SAN.** The obvious grouping, since
all three are ways of writing chess down, and the one that was asked for first.
Rejected because of where it puts the arrows. `uci/command.rs` parses
`position fen <fen>` in production code, so `turox-engine` would have to depend
on `turox-notation`, which stops being a leaf for tools and lands in the engine's
own dependency chain. That edge is mitigable with a feature, but the second
consequence is not: `turox-chess`'s own unit tests use `try_from_fen` at 113
sites, so it would dev-depend on `turox-notation`, which depends on it. Cargo
permits that cycle and it builds, but a cycle is the least legible thing a crate
graph can contain, and legibility is the whole reason for the split.

The line drawn instead: a **position** encoding is a rule-level fact the engine
needs, and a **game** encoding is notation only tools need. FEN is the former.

**A `turox-book` crate.** The book format has two consumers that must agree on
it, the engine reading and the generator writing, so it cannot live in
`turox-engine` without dragging the engine back into `bookgen`. Its own crate
would be the most honest home, and it was rejected as 340 lines and one more
node on the graph for a format whose two consumers will never disagree. It sits
in `turox-chess`, already downstream of `board::zobrist`, which keys it.

**Keeping `rng` inside `turox-chess`.** Rejected because the split forces it
public either way: `board::zobrist` and `move_gen::magic` need it on one side
and `search` needs it on the other, so its own module doc's claim to be
crate-private stops being true the moment the crate divides. `turox-rng` is
where "public within the workspace, `publish = false`" is honest rather than a
contradiction left in a comment.

## Consequences

`Tt::Entry.mv` changed from a `pub u16` to a `Move`. The boundary made the old
shape impossible: `Move::bits` is crate-private to `turox-chess` on the stated
grounds that nothing outside should see a move's bit layout, and an entry
holding raw bits would have handed every caller a representation it could not
decode. `Move` is a `u16` newtype, so an entry costs the same.

The `strategies` module is public API of `turox-chess` behind a feature, which
is test support living in a shipping crate. The alternative was two copies of
two hundred lines of proptest generator, or a sixth crate existing only for
tests.

Reopening conditions, so this is revisited on evidence rather than on taste:
a second consumer of `turox-notation` appearing (one consumer is a hypothetical
seam, two is a real one); the book format growing enough that its two consumers
can disagree; or `turox-chess` acquiring a reason to depend on something that
depends on it, which would mean the line was drawn in the wrong place.
