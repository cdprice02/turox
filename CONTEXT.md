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
_Avoid_: Oracle (used for the reference itself elsewhere in chess literature,
but ambiguous here with "TT/eval oracle"), slow path, reference implementation
(too generic; "naive" is the load-bearing word: it must not share reasoning
with the thing it checks).
