# Board representation and move generation throughput

Research notes for the wayfinder question: what does the literature say
about board representation and move generation throughput, and
specifically about the copy-make trade turox has already made?

Sources are the Chess Programming Wiki pages named inline and the
Stockfish source, read for this note rather than recalled. Every claim
attributed to a source carries the page or file it came from. Everything
attributed to a measurement was measured on this machine for this note;
the harness and its caveats are in the last section.

## Verdict up front

**Do not convert `Board::make_move` to make/unmake.** At turox's
measured effective branching factor of 9.4, the entire realistic upside
of an undo stack is under a third of a ply, and the measurement that
would justify it does not survive contact with run-to-run noise on this
machine. The literature does not support it either: the Chess
Programming Wiki documents both approaches without ever quantifying a
winner, which is itself the finding.

**Do not shrink `Board`.** The 64-byte mailbox is not the problem, and
CPW's own [Board Representation] page describes keeping a mailbox
alongside bitboards as the normal hybrid rather than a redundancy to
eliminate. Stockfish does exactly the same thing.

**Do restructure the legality filter.** `legal_moves` calls
`Board::make_move` once per *pseudolegal move*, not once per node, and
that filter is roughly 85% of move generation cost. Stockfish tests
legality with a pin-mask bitboard test and never makes the move. This is
a 4x-ish move generation win, and it is the finding this ticket
actually turned up.

**Do move the TT probe above `legal_moves` in `negamax`.** Two sibling
research tickets found the ordering independently. This note supplies
the cost: at ~35 moves per node and ~47 ns per move, a wasted
`legal_moves` is on the order of the entire per-node time budget.

## What turox actually does today

`Board` is 144 bytes (measured `size_of::<Board>()`, not the ~150 the
ticket estimated): 16 bytes of `by_color`, 48 of `by_piece`, 64 of
`mailbox`, 8 of Zobrist hash, and 8 covering side to move, castling
rights, en passant target, and the two clocks with padding.

`make_move` takes `&self` and returns a fresh `Board`. That is
copy-make, and CPW's [Copy-Make] page describes the technique in the
form turox uses it: keep the position per ply rather than restoring it,
so "position\[ply\] is still valid" without explicit restoration.

The part the ticket's framing understates is *where* the copies happen.
`move_gen::legal::legal_moves` is:

```rust
moves.retain(|m| !in_check(&board.make_move(m), color));
```

So a node with 35 pseudolegal moves performs 35 full `make_move` calls
inside generation, plus one more for whichever move the search actually
descends into. The per-node copy volume is not 144 bytes, it is closer
to 35 times 144, about 5 KB. Any argument about copy-make's cost has to
start there, not at one copy per node.

## Where move generation time actually goes

Measured over the 30-position corpus that `benches/move_gen.rs` already
uses (the six standard perft positions plus their first four legal
children each), 1041 pseudolegal moves total, 34.7 per position. Median
of three best-of-15 runs, nanoseconds per pseudolegal move:

| Stage | ns/move | Share |
| --- | ---: | ---: |
| `pseudo_legal_moves` alone | 6.5 | ~14% |
| plus `make_move` per candidate | 41.2 | |
| plus `in_check` per candidate | 57.5 | |
| `legal_moves` end to end | 46.9 | 100% |

`legal_moves` measures below the hand-decomposed sum because the
decomposition arms force intermediate values into memory that the real
function keeps in registers, so read the middle rows as approximate
attribution rather than an exact partition. The load-bearing number is
the first and last: **generating the moves is 6.5 ns per move and
proving them legal is the other 40**.

`perft` on this build runs 18 to 19 Mnps (startpos depth 5, best of
seven). Search runs at 650 to 736 Knps. The gap is not a contradiction:
perft uses bulk counting, which CPW's [Perft] page describes as
skipping "the last makemove/undomove," so a perft node is far cheaper
than a search node that also evaluates, probes the TT, and sorts.

At 34.7 moves per node and 46.9 ns per move, one `legal_moves` call
costs about 1.6 microseconds. A search node at 700 Knps has about 1.4
microseconds of budget in total. Move generation is not one cost among
several in turox's search; to a first approximation it *is* the node
cost.

## Copy-make versus make/unmake

### What the literature says

Less than the ticket assumes. CPW's [Copy-Make] page describes the
mechanism and contrasts it with a stack-based approach that "demands
higher memory bandwidth for copying back and forth," and links forum
threads spanning 1995 to 2016. It publishes no benchmark, no percentage,
and no recommendation. [Unmake Move] and [Make Move] are the same:
careful about *what* state is reversible versus irreversible (en
passant, castling rights, halfmove clock are called out on both pages as
the irreversible set), silent about relative cost.

Twenty years of forum argument with no published number is evidence. If
either approach were reliably worth a large fraction of throughput, the
wiki would say so, the way it does say so for hashed perft ("may speed
up from 1.5 to 4 times") and for LMR's effect on branching factor.

Stockfish uses make/unmake. `Position::do_move(Move, StateInfo&, ...)`
and `Position::undo_move(Move)` with a `StateInfo` linked list, and the
struct is explicitly split into a "Copied when making a move" half
(material key, pawn key, castling rights, rule50, en passant square) and
a "Not copied when making a move (will be recomputed anyhow)" half.
Worth noting what that means: even the engine that chose make/unmake
still copies roughly 80 bytes of `StateInfo` per move. The choice is not
"copy versus no copy," it is how much to copy.

### What the measurement says

I patched a scratch copy of the engine to add an in-place
`make_move_in_place(&mut self, m: Move)` with an identical body, and
timed both against matched `black_box` shapes. Median of three
best-of-15 runs:

- `make_move` (copy-make, returns by value): 34.4 ns per move above
  bare generation
- `make_move_in_place` on a caller-copied buffer: 23.5 ns per move

Both arms perform one 144-byte copy. The ~11 ns gap is the *second*
materialization forced by returning `Board` by value across a call
boundary the optimizer did not collapse. That is a real and useful
result, but note what it is not: it is not the cost of the copy, and it
does not require an undo stack to capture. Changing the signature to
`&mut self` and having callers write `let mut child = *board;
child.make_move_in_place(m);` keeps copy-make entirely while recovering
most of the gap.

The copy itself is smaller than that. Padding `Board` from 144 to 208
bytes, a 44% size increase, with the padding kept live so the optimizer
could not elide it, moved `make_move` from 34.5 to 35.8 ns per move and
`perft` from 18.2 to 20.0 Mnps. The padded board measured *faster* on
perft. That is not a size effect, it is code layout and run-to-run
variance, and it means the copy is below the noise floor at these sizes.

Larger padding does eventually show a cost (272 bytes and 656 bytes both
measured slower), so the copy is not free in the limit. But the step
that matters, 144 bytes up or down by a few dozen, is inside the noise.

### The verdict at EBF 9.4

Plies bought by a throughput speedup S at effective branching factor b
is log(S) / log(b). At b = 9.4:

| Speedup | Plies gained |
| --- | ---: |
| 1.25x | 0.10 |
| 2x | 0.31 |
| 4x | 0.62 |

Against that, holding the node budget fixed at whatever depth 8 costs
today and lowering the EBF instead:

| New EBF | Depth reached | Plies gained |
| --- | ---: | ---: |
| 6 (plain alpha-beta, sqrt of 35) | 10.0 | +2.0 |
| 4 | 12.9 | +4.9 |
| 3 | 16.3 | +8.3 |

CPW's [Branching Factor] page is the source for those targets: good move
ordering should get alpha-beta to roughly the square root of the average
branching factor, and "alpha-beta enhancements, transposition tables,
null move pruning and late move reductions further reduce the EBF below
three, strong programs even near or below two."

Now cost out make/unmake honestly. Best case it removes one 144-byte
copy per made move and replaces the return-by-value materialization with
an in-place update, then pays for an `undo_move` that performs
substantially the same bitboard and mailbox writes as `do_move` in
reverse. Optimistically call it 34 ns down to 25 ns per move: a 1.35x
improvement on the make step, which is 73% of `legal_moves`, which is
most of the node. Round the whole-engine effect generously up to 1.25x.

**That is 0.10 ply.** For a rewrite that replaces a signature with no
undo-state bugs possible by construction with a stateful undo stack that
has to get en passant, castling rights, the halfmove clock, and the
captured piece exactly right on every path, in a codebase whose CLAUDE.md
specifically flags scrambled White-versus-Black and kingside-versus-
queenside bugs as a recurring failure mode here. The `Board` doc comment
already names the other thing it would cost: copy-make "stays trivially
parallel if lazy SMP search happens later."

Make/unmake is the wrong trade at EBF 9.4. It is arguably the wrong
trade at EBF 3 too, but that is not a question this ticket has to
answer.

[Copy-Make]: https://www.chessprogramming.org/Copy-Make
[Unmake Move]: https://www.chessprogramming.org/Unmake_Move
[Make Move]: https://www.chessprogramming.org/Make_Move
[Perft]: https://www.chessprogramming.org/Perft
[Branching Factor]: https://www.chessprogramming.org/Branching_Factor

## Legal-only versus pseudolegal generation

This is where the throughput actually is.

CPW's [Legal Move] page defines a legal move as "a pseudo-legal move
which does not leave its own king in check," and then describes the
mainstream implementation: "most programs delay the legality test to the
child node, after incremental updates attack and defend maps or an
explicit square attacked test direct after make move." It adds that
"many programs consider absolutely pinned pieces in move generation," and
flags the one genuinely hard case: "En passant requires special
horizontal pin test of both involved pawns, which disappear from the same
rank."

turox does the explicit-square-attacked test, but *eagerly for every
pseudolegal move* rather than delayed to the child node. That is the
expensive combination: it pays the full test for moves the search will
never look at because an earlier move already caused a beta cutoff.

Stockfish's `generate<LEGAL>` shows what the alternative costs:

```cpp
Bitboard pinned = pos.blockers_for_king(us) & pos.pieces(us);
Square   ksq    = pos.square<KING>(us);
...
while (cur != moveList)
    if (((pinned & cur->from_sq()) || cur->from_sq() == ksq || cur->type_of() == EN_PASSANT)
        && !pos.legal(*cur))
        *cur = *(--moveList);
    else
        ++cur;
```

Only three categories of move get tested at all: moves from a pinned
piece, king moves, and en passant. Everything else is legal by
construction once you know the pin set. And `Position::legal` itself
makes no move:

```cpp
if (type_of(piece_on(from)) == KING)
    return !(attackers_to_exist(to, pieces() ^ from, ~us));

return !(blockers_for_king(us) & from) || line_bb(from, to) & pieces(us, KING);
```

For a non-king, non-en-passant move that is two bitboard tests. The pin
set comes from `update_slider_blockers`, computed once per position:
sweep rook and bishop rays from the king, find enemy sliders, and for
each one check whether exactly one piece sits `between_bb(ksq, sniperSq)`.

Note also the branch above it: `pos.checkers() ? generate<EVASIONS> :
generate<NON_EVASIONS>`. When in check, Stockfish does not generate
everything and filter. turox has no evasion generator.

Mapping that onto turox's numbers: in a typical position the pinned set
is zero to two pieces and king moves are two to eight, so perhaps five of
35 moves would need a test, each a couple of bitboard operations instead
of a 41 ns `make_move`. `legal_moves` should land nearer its 6.5 ns per
move generation floor than its current 46.9. Call it 4x on move
generation, which by the table above is 0.62 ply, and unlike make/unmake
it is bought with pure bitboard logic in a module that already has
`between`, `line`, and a magic-bitboard slider attack path.

Two warnings, both from the sources and both matching this repo's own
history. The en passant horizontal pin is the classic wrong answer and
CPW calls it out explicitly; turox currently gets it right for free
precisely because it makes the move and re-scans. And this is a
White-versus-Black crossed with ray-direction problem, which CLAUDE.md
names as the exact shape that has repeatedly produced scrambled bugs
here. Perft is the gate, and it is already wired up for six positions.

[Legal Move]: https://www.chessprogramming.org/Legal_Move
[Move Generation]: https://www.chessprogramming.org/Move_Generation

## Board layout, the mailbox, and cache behaviour

The ticket asks whether shrinking `Board` is a cheaper intermediate,
since the 64-byte mailbox is the largest field and partly redundant with
the bitboards. Three reasons to leave it alone.

First, the measurement above: a 44% size increase was invisible, so a
44% decrease will be too.

Second, the literature is against it. CPW's [Board Representation] page
frames piece-centric (bitboards) and square-centric (mailbox) as
complementary and says outright that "it is quite common to use redundant
board representations with elements of both," specifically that
"bitboard approaches often keep a 8x8 board to determine a piece by
square." Stockfish's `Position` carries `std::array<Piece, SQUARE_NB>
board` next to `byTypeBB` and `byColorBB` for exactly this reason.

Third, removing it makes the engine slower where it matters. `piece_at`
becomes a six-bitboard scan, and the `Board` doc comment already
identifies the callers that would pay: captures, SEE, and eval. SEE in
particular is on the move ordering critical path, which is the thing that
actually needs to get faster.

On cache behaviour the honest answer is that turox's hot working set is
not `Board`, it is the attack tables. CPW's [Magic Bitboards] page gives
fancy magics as "about 38 KiB for the bishop attacks, but still about 800
KiB for rook attacks," which is what turox has (`ROOK_TABLE_SIZE`
102,400 entries of 8 bytes, `BISHOP_TABLE_SIZE` 5,248). That is an 850 KB
table streamed through L2 and L3 while a 144-byte `Board` sits in L1. The
page is candid that the tradeoff exists: "changes in occupancy outside the
blockers ... will introduce some more cache misses." It also reports that
Robert Purves found plain magics "nearly indistinguishable from Fancy" on
a large-L3 Intel part, which is a useful calibration on how much table
layout is worth chasing.

If table size ever becomes the target, the page names Black Magic as the
densest published variant, "692 KiB for the complete rook and bishop
attack table" against turox's roughly 845 KB. That is an 18% reduction
for a full magic re-search. Not obviously worth it, and it should be
benchmarked rather than assumed.

[Board Representation]: https://www.chessprogramming.org/Board_Representation
[Magic Bitboards]: https://www.chessprogramming.org/Magic_Bitboards

## PEXT magics

**Blocked on `unsafe_code`, before the AMD question even comes up.**

`core::arch::x86_64::_pext_u64` is an unsafe function. Verified by
compiling a call to it under `#![deny(unsafe_code)]`:

```
error[E0133]: call to function `_pext_u64` with `#[target_feature]` is
unsafe and requires unsafe function or block
```

The workspace denies `unsafe_code`, so a PEXT path needs an explicit
`#[allow(unsafe_code)]` plus a `// SAFETY:` comment (clippy's
`undocumented_unsafe_blocks` is also denied). That is a policy decision
about the crate's core invariant, not a performance tuning question, and
it should be taken deliberately or not at all.

If it is taken, the AMD caveat in `.cargo/config.toml` is accurate and
CPW confirms it. The [BMI2] page states that "BMI2 was further
incorporated in AMD's Zen-architecture but until Zen 3 in November 2020
with a slow implementation of critical instructions such as PDEP and
PEXT." It gives no cycle counts. The config comment's guidance holds
unchanged: the `+bmi2` flag makes the instruction available and correct
across the 2017+ baseline, it does not make it fast on Zen1 or Zen2, and
a PEXT path would need either a runtime dispatch (more `unsafe`, since
entering a `#[target_feature]` function from a non-target-feature context
is itself unsafe) or an accepted regression on 2017 to 2020 AMD hardware.
For an engine whose deployment target is a lichess bot on hardware
turox does not choose, that is a real exposure, not a hypothetical.

What PEXT would buy is described on the same page as "the relevant up to
four ray occupancies are mapped to a dense index range," replacing the
multiply-and-shift in `magic_index` with one instruction and shrinking
the table through dense indexing. Given that `magic_index` is a single
`wrapping_mul` plus a shift and the actual cost of a slider lookup is the
850 KB table access, the upside is table density more than instruction
count.

Separately, and more cheaply: the two table accesses in
`magic::rook_attacks` and `bishop_attacks` are bounds-checked today.
Every published magic bitboard implementation uses unchecked indexing
there. Removing those checks also requires `get_unchecked` and therefore
`unsafe`, and it should be measured before it is assumed to matter, since
the branch predictor handles a never-taken bounds check well and the load
itself is the expensive part.

[BMI2]: https://www.chessprogramming.org/BMI2

## Incremental attack tables

**Do not.** This is the one item where the literature gives a direct,
attributed answer.

CPW's [Attack and Defend Maps] page acknowledges the appeal, that "a move
has often only a local influence on the attack tables," and then reports
that Joel Rivat, Robert Hyatt, Ed Schroder, and Gerd Isenberg have all
"abandoned incrementally updated attack tables" in favour of computing
what is needed on demand. The reasons given are that maintenance "does
become more expensive in the late middlegame or endings with sliding
pieces, especially queens," and that "a lot of nodes don't need the
attack information at all, or only a small part of it."

The [Incremental Updates] page reaches the same conclusion from the other
direction, noting that for attack tables specifically, "due to its size
and utilization, copy-make is no issue, but on the fly generation if
actually needed."

Worth separating from that verdict: incremental *scalar* state is
standard and turox already does the important one. The Zobrist hash is
maintained incrementally through `place`, `remove`, `from_parts`, and
`make_move`, which is exactly what the [Make Move] page prescribes. The
[Incremental Updates] page also names material signatures and the sum of
piece-square values as normal incremental candidates. With tapered eval
and PSTs now in, an incremental PST accumulator is a plausible later win,
and unlike attack tables it is cheap and local.

The one attack-adjacent structure that is worth keeping per position is
the pin set from the previous section, and the Stockfish precedent is to
recompute it once per node in `set_check_info` rather than update it
incrementally.

[Attack and Defend Maps]: https://www.chessprogramming.org/Attack_and_Defend_Maps
[Incremental Updates]: https://www.chessprogramming.org/Incremental_Updates

## The TT probe ordering, from a move generation angle

Two sibling research tickets found that `negamax` calls `legal_moves`
before probing the transposition table. In `search/negamax.rs` the
generation is at the top of the node and the probe follows it, so every
TT cutoff pays a full legal move generation it then throws away.

This note's contribution is the price tag. One `legal_moves` is about 1.6
microseconds at 34.7 moves per node; a search node at 700 Knps has about
1.4 microseconds of total budget. A TT cutoff that should cost a hash
probe instead costs more than an average node.

The reason `legal_moves` is there is real and needs preserving: the
`moves.is_empty()` branch distinguishes checkmate from stalemate. The
resolution is to probe first, return on a cutoff, and only generate when
the node is actually going to be searched, with the mate and stalemate
determination folded into the generation path that already happens.
Ordering the node as probe, then generate, then order, then loop is
strictly cheaper than the current shape and changes no result.

Weighed against make/unmake: this costs a few lines and buys back a full
move generation on every TT cutoff. Make/unmake costs a rewrite of the
most invariant-heavy function in the engine and buys 0.10 ply. It is not
close.

## Recommended order of work

1. **Move the TT probe above `legal_moves`.** Cheapest possible change,
   and it deletes work rather than making work faster.
2. **Replace the per-candidate `make_move` plus `in_check` filter with a
   pin-mask legality test**, following `Position::legal` and
   `update_slider_blockers`. This is the ~4x move generation win. Gate it
   on all six perft positions including the ignored deep depths, and
   write an asymmetric en passant horizontal pin case deliberately.
3. **Add an evasion generator** so in-check nodes generate king moves,
   captures of the checker, and blocks rather than everything.
4. **Consider changing `make_move` to `&mut self`**, keeping copy-make at
   the call site. Roughly 11 ns of the 34 ns per move, no undo stack, no
   new invariants. Measure it; the number came from a scratch patch, not
   from the real call graph.
5. **Do not** convert to make/unmake, shrink `Board`, adopt PEXT, or
   maintain incremental attack tables.

Steps 1 through 3 are throughput, and throughput at EBF 9.4 is worth
under a ply no matter how well it goes. They are worth doing because
they are cheap and because they compound with everything else, not
because they will move the rating much on their own. The rating lives in
the move ordering ticket.

## Method, and what to distrust

The engine was copied to a scratch directory, patched with an in-place
`make_move` variant and with optional live padding on `Board`, and
driven by a harness that takes the minimum of 15 repetitions of 3000
passes over the 30-position corpus, plus best of seven full perft runs.
The `.cargo/config.toml` target features were copied across so the
scratch build matches the real one. Minimum rather than mean, because for
throughput microbenchmarks the minimum is the estimator least polluted by
scheduler noise.

It is still noisy. Identical code measured `legal_moves` anywhere from
42.9 to 47.6 ns per move across runs, roughly plus or minus 10%. One
padding sweep produced a 1.8x slowdown on `pseudo_legal_moves`, which
never copies a `Board` at all and therefore cannot depend on its size;
that run was discarded as machine contention. Any claim in this note
below about 10% should be treated as unmeasured.

That noise floor is itself the answer to the ticket's central question.
The copy-make versus make/unmake difference does not clear it. Something
that cannot be measured reliably on a quiet machine is not something to
restructure the engine's core invariant for.

Criterion was not used, because the scratch crate needed a patched
`Board` and the repo's benches build against the real one. A follow-up
that wants tighter numbers should add the decomposition to
`benches/move_gen.rs` instead.

## What this note did not cover

- **Staged move generation.** Named on CPW's [Move Generation] page as
  generating moves in phases "on the premise that if one of the early
  moves causes a cutoff, then we may save on the effort of generating the
  rest." Directly relevant to turox's costs and squarely inside the move
  ordering ticket's scope, so left there rather than duplicated.
- **Alternative slider techniques**: kindergarten bitboards, hyperbola
  quintessence, obstruction difference. Not read; magic bitboards are
  already in and working, and the profile says the legality filter is the
  target.
- **Black Magic bitboards** beyond the table size figure quoted above.
- **`MoveList` layout.** It measures 520 bytes and lives on the stack per
  node, which is larger than `Board` and was never part of this ticket's
  framing. Worth a look on its own.
- **Bulk counting in search.** Perft already does it; whether the
  quiescence path can skip work the same way was not investigated.
- **Piece lists and piece counts.** Stockfish keeps `pieceCount[PIECE_NB]`;
  turox derives it from bitboard popcounts. Not measured.
