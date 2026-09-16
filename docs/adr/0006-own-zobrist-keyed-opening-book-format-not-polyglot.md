# Own Zobrist-keyed format for the opening book, not Polyglot

The opening book is keyed on `board::zobrist`'s existing hash and stored in a
format built for this engine, rather than the widely-used Polyglot `.bin`
format. Polyglot needs zero engine-side code under `lichess-bot`, which
already reads it directly, but it works only under that one host: not
self-play, not any other UCI GUI. It also carries its own 781-number Zobrist
scheme, entirely separate from this engine's, and its own castling-move
encoding convention, storing a castling move as the king capturing its own
rook (e1h1 rather than e1g1 for kingside castling): a fresh instance of the
`{Color}x{side}` shape that has repeatedly produced scrambled bugs in this
crate. The own-format path reuses hashing that already exists and is already
tested, and serves every UCI host turox runs under from one file.

## Considered options

**Polyglot `.bin`.** The path of least engine-side effort: a config change
and a book file, no code. Rejected because the savings are narrow, limited to
one host, and the cost of ever reading Polyglot data directly, rather than
only relying on lichess-bot's own reader, is a second Zobrist scheme plus a
castling encoding that has burned this codebase before in a different guise.

## Consequences

A book built in this format has no meaning outside turox: it cannot be read
by another engine, and the large public ecosystem of prebuilt Polyglot books
cannot be reused directly. Populating the book is entirely this project's own
responsibility, through its own generator.
