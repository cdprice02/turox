# Move ordering and staged move generation

Research notes for the wayfinder question: what does the literature say
about move ordering and staged move generation, what ordering quality
should turox expect from each technique, and does the ranking
`MovePriority` already reserves match what the literature recommends?

Sources are the Chess Programming Wiki pages named inline and the
Stockfish source, read for this note rather than recalled. Every
quantitative claim below carries the page it came from.

## Why this is the highest-leverage item on the map

turox measures an effective branching factor of 9.4 across depths 5 to 8.
CPW's [Branching Factor] page gives the yardstick: the average chess
branching factor is 35 to 38, alpha-beta *with good move ordering*
reduces the effective branching factor to roughly the square root of
that, and modern engines with transposition tables, null move pruning,
and late move reductions run below 3, with the strongest near or below 2.

Square root of 35 is about 5.9. So turox at 9.4 is not merely short of a
modern engine, it is short of what plain alpha-beta is supposed to
deliver on its own. That is the specific gap this note is about: the
first 9.4 to 6 is pure ordering, and everything that takes an engine from
6 down to 2 or 3 (LMR especially) is *gated* on ordering quality, because
those techniques reduce or skip late moves on the assumption that late
means bad. CPW's [Late Move Reductions] page states the dependency
directly: LMR became viable around 2005 when Fruit and Glaurung drove it
off the history heuristic, and "LMR can often reduce the effective
branching factor to less than 2." Reducing late moves in a badly ordered
list just reduces good moves.

So ordering is not one improvement among several. It is the one that
unlocks the rest.

[Branching Factor]: https://www.chessprogramming.org/Branching_Factor
[Late Move Reductions]: https://www.chessprogramming.org/Late_Move_Reductions

## The canonical ordering

CPW's [Move Ordering] page gives the typical sequence:

1. PV move from the previous iteration's principal variation
2. Hash move from the transposition table
3. Winning captures and promotions
4. Equal captures and promotions
5. Killer moves (non-capture), often with mate killers first
6. Non-captures sorted by the history heuristic
7. Losing captures

Two things in that list are easy to skim past and both matter for
turox. Promotions are grouped *with* captures at ranks 3 and 4, not
treated as quiet moves. And losing captures sit dead last, behind
history-sorted quiets, not ahead of them.

The same page carries the number that justifies the whole exercise: at
cut-nodes, "the best move succeeds as a cutoff in greater than 90% of all
fail-high nodes." That is both the goal and, as the instrumentation
section below argues, the metric.

CPW's [Node Types] page explains why cut-nodes are where ordering pays.
A PV-node has to search every move and an all-node has to search every
move; only a cut-node can stop early, and it needs a minimum of one move
if that move is the right one. With branching factor 40 at depth 6 the
page counts 1 PV-node, 63,999 cut-nodes, and 63,999 all-nodes. Ordering
effort spent anywhere other than "find the refutation at a cut-node
first" is spent on nodes that were going to search everything anyway.

[Move Ordering]: https://www.chessprogramming.org/Move_Ordering
[Node Types]: https://www.chessprogramming.org/Node_Types

## The individual heuristics

### MVV-LVA

Per CPW's [MVV-LVA] page, the scheme sorts captures by most valuable
victim first, then by least valuable aggressor, so "pawn captures rook
before bishop captures pawn." It is cheap and it is the standard starting
point, which is where turox is today.

Its documented failure mode is precise: it "may fail, if victims attacked
by more valuable attackers are defended." MVV-LVA does not know about
defenders at all. Rook takes a defended pawn scores as a capture of a
pawn by a rook, which MVV-LVA calls bad, correctly by accident. Queen
takes a defended knight scores as losing, again by accident. But bishop
takes a defended knight scores as equal, and it is actually losing a
bishop for a knight plus whatever recaptures. Only SEE resolves that.

[MVV-LVA]: https://www.chessprogramming.org/MVV-LVA

### Static exchange evaluation

CPW's [Static Exchange Evaluation] page describes SEE as resolving the
forced exchange sequence on a single square and reports that "a positive
static exchange indicates a 'winning' move." Its two documented uses are
exactly the two turox needs: splitting captures into good and bad
buckets for move ordering, and pruning in quiescence search.

The recursive formulation on that page is short enough to be the mental
model:

```
int see(int square, int side) {
   value = 0;
   piece = get_smallest_attacker(square, side);
   if (piece) {
      make_capture(piece, square);
      value = max(0, piece_just_captured() - see(square, other(side)));
      undo_capture(piece, square);
   }
   return value;
}
```

The `max(0, ...)` is the whole trick: a side that would lose material by
recapturing simply declines, so the exchange stops.

Real engines use the iterative swap-list version on CPW's [SEE - The Swap
Algorithm] page, which is a better fit for turox because it is
bitboard-native and needs no make/unmake. Its shape:

- Build a `gain[]` array, `gain[0]` being the value of the piece
  initially on the target square.
- Repeatedly pull the least valuable attacker from the combined
  attackers-and-defenders set, alternating sides, walking piece types in
  ascending value order.
- After removing each attacker from the occupancy, re-add hidden x-ray
  attackers: sliders that were behind the piece just removed. This is the
  part that is easy to skip and wrong to skip, since a rook behind a rook
  is a completely normal exchange.
- Negamax the array backward:
  `while (--d) gain[d-1] = -max(-gain[d-1], gain[d]); return gain[0];`

The page also notes that king captures terminate the sequence and that
pinned pieces need special handling in some implementations.

For turox specifically: an attackers-to-square function that returns a
`Bitboard` of every attacker of both colors is the prerequisite, and
`move_gen::attacks` already has the per-piece machinery to build it. The
x-ray step is a re-query of rook and bishop attacks from the target
square against the updated occupancy, which is exactly what the magic
tables already do. No unsafe, no dependencies.

[Static Exchange Evaluation]: https://www.chessprogramming.org/Static_Exchange_Evaluation
[SEE - The Swap Algorithm]: https://www.chessprogramming.org/SEE_-_The_Swap_Algorithm

### Killer heuristic

CPW's [Killer Heuristic] page: a dynamic, path-dependent technique that
prioritizes moves which caused beta cutoffs at *sibling* nodes, on the
theory that positional threats stay roughly constant across siblings, so
a move that refuted one opponent reply often refutes the next.

The implementation contract, per the page:

- Table indexed by ply, typically two or three moves per ply.
- The replacement scheme must keep the slots distinct, so a repeat of the
  existing killer does not evict the second one.
- **Only quiet moves are recorded.** A capture causing a cutoff does not
  become a killer.
- Killers rank after hash moves and strong captures, though "placement
  relative to captures varies by implementation."

Two details worth carrying into a turox implementation. Killers are
looked up from a *sibling*, so they may be illegal in the current
position and must be validated against the generated move list before
being trusted. And because they are ply-indexed rather than
position-indexed, the table has to be sized to max ply and cleared per
search, not per node.

CPW's [Mate Killers] page covers the refinement: a move that caused a
cutoff with a score indicating a forced mate is stored separately and
"sorted higher than ordinary killer moves." The page does not state where
mate killers sit relative to captures, so the only claim the literature
supports is mate killer above ordinary killer.

[Killer Heuristic]: https://www.chessprogramming.org/Killer_Heuristic
[Mate Killers]: https://www.chessprogramming.org/Mate_Killers

### History heuristic

CPW's [History Heuristic] page: invented by Jonathan Schaeffer in 1983,
it accumulates cutoff counts per move independent of position, indexed
either `[from][to]` (butterfly boards) or `[piece][to]`. Stockfish uses
piece-and-destination.

The classical update is depth-weighted, `history[stm][from][to] += depth
* depth`, and the page is explicit about why: without it, moves near the
leaves, of which there are vastly more, would swamp the table.

Modern practice per the same page adds three things:

- **History gravity**: scale the bonus by how expected the cutoff was, so
  a surprising cutoff moves the value a lot and an expected one barely
  moves it. This also stops values saturating.
- **History maluses**: penalize quiet moves that were searched and
  *failed* to cause a cutoff, so the table carries negative information
  and not just positive.
- **Decay/aging**, implied by the saturation discussion, so old
  information does not permanently outrank fresh.

Variants named on the page, in increasing sophistication: relative
history heuristic (history normalized by butterfly counts), counter moves
history (indexed by the previous move), continuation history
(generalizes that to the move n plies ago), and capture history, which
"replac[es] MVV-LVA scoring" for captures outright.

History is the single technique that makes quiet moves orderable at all.
turox today has no quiet ordering whatsoever, so every quiet move is
tied. That is the largest single hole.

[History Heuristic]: https://www.chessprogramming.org/History_Heuristic

### Countermove heuristic

CPW's [Countermove Heuristic] page: introduced by Jos Uiterwijk in 1992,
built on the observation that "many moves have a 'natural' response,
irrespective of the actual position." It is complementary to killers
because it keys on the previous move rather than on the ply.

Storage is either a 64x64 butterfly table on the previous move's
`[from][to]`, or a more compact 6x64 on `[piece][to]`, with a separate
table per side. The update, quoted from the page:

```
if (score >= beta) {
   if (isNonCapture(move))
      counterMove[previousMove.from][previousMove.to] = move;
   return score;
}
```

Note it stores only non-captures, same restriction as killers. Matching
moves get a bonus during scoring rather than a hard rank of their own,
which is a meaningful design point: countermoves in modern engines are a
*score adjustment*, not a bucket.

Counter-move history and continuation history, per the History Heuristic
page, are the natural extension: instead of storing one refutation per
previous move, store a full history table conditioned on the previous
move (counter-move history) or on the move n plies back (continuation
history). Stockfish's `PieceToHistory**` array of continuation histories,
visible in `movepick.h`, is that idea in production.

[Countermove Heuristic]: https://www.chessprogramming.org/Countermove_Heuristic

### Internal iterative deepening

Worth naming because turox will hit the case it addresses. CPW's
[Internal Iterative Deepening] page: when there is no best move from the
TT or a previous search, search the position to a reduced depth first and
use that result as the first move at full depth. Applied primarily at
PV-nodes, also at expected cut-nodes, typically gated on depth greater
than about 5. Reductions are subtractive (-1, -2), divisive (/2, /4), or
hybrid; Deep Thought used -2. The page describes it as "like an
insurance," and notes that "programs with weaker move ordering see more
substantial gains." That last clause is a direct statement that turox is
in the population IID helps most. A modern lighter-weight alternative,
internal iterative reduction (IIR), is mentioned but not detailed.

[Internal Iterative Deepening]: https://www.chessprogramming.org/Internal_Iterative_Deepening

## Staged move generation

CPW's [Move Generation] page states the premise plainly: rather than
generating everything at once, generate in phases, because "if one of the
early moves causes a cutoff, then we may save on the effort of generating
the rest of the moves." Combined with the >90% first-move cutoff rate
from the Move Ordering page, that is a large fraction of cut-nodes where
generating quiet moves at all is wasted work.

The page also describes chunk generation, where tactical and quiet moves
are buffered separately, scored with MVV-LVA, SEE, history, and
piece-square tables, then consumed by *selection sort* rather than a full
sort. Selection sort matters here: it costs O(n) per move consumed, so
stopping after one move costs one pass, whereas a full sort pays O(n log
n) up front regardless.

Stockfish is the reference implementation. Its `MovePicker` header states
the contract: `next_move()` "emits one new pseudo-legal move on every
call, until there are no moves left," and the class "attempts to return
the moves which are most likely to get a cut-off first." Note
*pseudo-legal*: staged generation and legality filtering interact, since
filtering the whole list for legality up front reintroduces the cost
staging exists to avoid.

The stages, from `movepick.cpp`:

- Main search: `MAIN_TT`, `CAPTURE_INIT`, `GOOD_CAPTURE`, `QUIET_INIT`,
  `GOOD_QUIET`, `BAD_CAPTURE`, `BAD_QUIET`
- Evasions: `EVASION_TT`, `EVASION_INIT`, `EVASION`
- Probcut and quiescence: `PROBCUT_TT`, `PROBCUT_INIT`, `PROBCUT`,
  `QSEARCH_TT`, `QCAPTURE_INIT`, `QCAPTURE`

Three observations from that list, all of which bear on the ranking
question below.

First, the TT move is returned *before any generation happens at all*.
That is only sound if the TT move can be validated without a move list,
which is why engines carry a `Position::pseudo_legal(Move)` checker.

Second, the main-search order is good captures, then good quiets, then
bad captures, then bad quiets. Bad captures sit *between* the two quiet
buckets, which is the modern refinement of CPW's "losing captures last."

Third, and most surprising: there is no killer or refutation stage in the
modern list. Stockfish folds killers into quiet scoring via its history
tables and then splits `GOOD_QUIET` from `BAD_QUIET` on the resulting
score. The literature's discrete killer rank is a stepping stone toward
score-based quiet ordering, not the end state.

Also worth noting for turox: evasions are a single undifferentiated
stage. When in check the move list is short and the phase machinery is
not worth its overhead, which is a useful licence to keep the check case
simple.

[Move Generation]: https://www.chessprogramming.org/Move_Generation

## Measuring ordering quality

This is the part turox has none of, and it should land before any of the
techniques above.

### The metric

The standard diagnostic is the distribution of *which move index caused
the beta cutoff*, aggregated over fail-high nodes. The Move Ordering
page's ">90% of all fail-high nodes" is exactly a statement about the
first bucket of that histogram. So the headline number is:

```
first-move cutoff rate = cutoffs_at_index_0 / total_fail_high_nodes
```

Target above 0.9 for a well-ordered engine. The full histogram is more
informative than the single rate, because the failure modes look
different: a heavy tail says quiets are unordered, while a bump at index
1 or 2 says the capture ordering is nearly right but the TT or killer
move is missing.

The second metric is effective branching factor, which the Branching
Factor page defines as `EBF(N) = nodes(N) / nodes(N-1)` across iterative
deepening iterations. The same page warns that the odd-even effect makes
adjacent-iteration ratios noisy, so it should be read across several
iterations rather than from one pair, and that cross-engine EBF
comparison is unreliable once extensions and reductions are in play. It
remains perfectly good for turox-versus-turox A/B.

### What to add to turox

All of this is counters and arrays, so it fits the zero-dependency and
no-unsafe constraints without argument.

In `Search`:

- `fail_high_nodes: u64`, incremented at each `alpha >= beta` break in
  `negamax`.
- `cutoff_index: [u64; 16]`, indexed by the loop position of the move
  that caused the break, with index 15 as a saturating overflow bucket.
  The move loop currently uses `for &m in &moves`, so this needs
  `.enumerate()`, which is the entire code change.
- The same pair again for `quiescence`, kept separate. Quiescence sees a
  filtered capture-only list and mixing the two would flatter the
  main-search number badly, since capture lists are short and
  well-ordered by construction.
- `nodes_per_depth: Vec<u64>` (or a fixed array to max depth) captured at
  the end of each iterative-deepening iteration, so EBF is derivable
  without re-running.

Surfacing it:

- Extend `SearchResult` with the counters, since it already carries
  `nodes` and is the natural home.
- Emit a UCI `info string` line per search with the first-move cutoff
  rate and the per-iteration EBF. `info string` is free-form, so no GUI
  is confused by it, and it makes the metric visible during real games
  and lichess play rather than only in benchmarks.

On gating it behind a cargo feature: probably not worth it. The cost is
one increment and one array write per fail-high node, against a full
`make_move` plus recursion per node. Measure it with `benches/search.rs`
before deciding, but the prior should be that it is free and always-on
instrumentation is more useful than instrumentation someone has to
remember to enable.

Tests, matching this repo's habits:

- A unit test that the histogram sums to `fail_high_nodes`, which catches
  the classic off-by-one where the break increments one counter but not
  the other.
- A fixed-position regression test asserting the first-move cutoff rate
  clears a floor. Set the floor from the current measured value once the
  instrumentation lands, then raise it as each technique goes in. That
  turns "did the ordering improve" into a test rather than a
  read-through, and it is the only way a later refactor cannot silently
  regress ordering while keeping every correctness test green.
- Node counts at fixed depth on a fixed position are themselves an
  ordering proxy and already stable enough to assert on, since ordering
  changes node counts but not the returned score. A test that the score
  at depth N is unchanged while node count drops is the clean way to
  prove an ordering change is a pure win.

`benches/search.rs` already computes throughput against whatever node
count the run reports, and its own module doc says node count "depends on
pruning and move-ordering quality." Once the counters exist, that bench
should print them, since nodes per second can improve while ordering gets
worse and the two numbers need to be read together.

## Verdict on the reserved `MovePriority` ranking

Current declaration order in `turox-engine/src/search/negamax.rs`:

```
PrincipalVariation, Hash, KillerCapture, WinningCapture, EqualCapture,
MateKiller, Killer, LosingCapture, Quiet
```

Against CPW's canonical list, four things are wrong or questionable and
two are right.

**Right:** `PrincipalVariation` then `Hash` at the top matches the
canonical list exactly. `MateKiller` immediately above `Killer` matches
the Mate Killers page and the Move Ordering page's "often with mate
killers first."

**Wrong 1: `KillerCapture` should not exist, at least not there.** The
Killer Heuristic page is explicit that only quiet moves are recorded as
killers, and the Countermove page's update snippet has the same
`isNonCapture` guard. A capture that caused a cutoff at a sibling is
information, but the technique that captures it is *capture history*, not
the killer table, and capture history replaces MVV-LVA scoring within the
capture bucket rather than jumping ahead of it. Ranking a killer capture
above a winning capture also creates an ambiguity the enum cannot
express: a move that is both a winning capture and a stored killer
capture has two valid ranks, and which one it gets depends on the order
of branches in `move_priority`. Recommendation: delete the variant.

**Wrong 2: `LosingCapture` above `Quiet` is backwards.** CPW puts losing
captures last, behind history-sorted quiets. Stockfish puts `BAD_CAPTURE`
between `GOOD_QUIET` and `BAD_QUIET`. Neither puts bad captures ahead of
all quiets. This one is currently harmless, because with no history table
every quiet is tied and "ahead of an undifferentiated blob" is not a
meaningful position. It stops being harmless the moment history lands,
which is precisely why the reserved ranking should be fixed now rather
than after.

**Wrong 3: promotions are missing entirely.** The canonical list says
"winning captures *and promotions*" and "equal captures *and
promotions*." `move_priority` never calls `is_promotion()`, so a
non-capture queen promotion is classified `Quiet`, tied with a rook shuffle.
`MoveFlags` already distinguishes `PromoteQueen` from
`PromoteCaptureQueen`, so the information is right there. A
promotion-capture does get a capture rank, but scored as victim minus
pawn, which ignores the roughly +800 the promotion itself is worth. This
is the most concrete defect in the current implementation, not just in
the reserved ranks. Related: `quiescence` filters with
`retain(|m| m.flags().is_capture())`, which drops non-capture promotions
from quiescence altogether. Same root cause, worth fixing together.

**Wrong 4, structurally: a fieldless enum is the wrong shape for the sort
key.** This is the deepest issue and it is not about ordering the
variants at all. `order_moves` does
`sort_unstable_by_key(|&m| move_priority(...))`, so every move within a
bucket ties, and `sort_unstable` resolves ties arbitrarily. Concretely,
`WinningCapture` collapses rook-takes-queen and pawn-takes-knight into
one indistinguishable bucket, discarding most of what MVV-LVA computed.
The literature scores moves numerically and sorts on the score; the
bucket is the high bits of that score, not the whole of it.

The fix keeps the enum, which is genuinely good documentation of the
hierarchy and is pinned by a test, but makes it the coarse component of a
composite key:

```rust
fn move_score(...) -> (MovePriority, i32)
```

with the `i32` carrying MVV-LVA within captures and history within
quiets. That also gives countermoves somewhere to live, since the
Countermove page describes them as a scoring bonus rather than a rank.

**Recommended ranking**, after those changes:

```
PrincipalVariation
Hash
WinningCapture   (SEE > 0, promotions included)
EqualCapture     (SEE == 0)
MateKiller
Killer
Quiet            (ordered within by history; becomes GoodQuiet/BadQuiet later)
LosingCapture    (SEE < 0)
```

The eventual Stockfish-shaped end state splits `Quiet` on a history
threshold and puts `LosingCapture` between the halves. That is worth
noting in the enum's doc comment but not worth building before history
exists to threshold on.

## Two implementation blockers found in passing

Neither is strictly an ordering technique, but both stand directly in the
way of the techniques above.

**`negamax` generates the full legal move list before probing the TT.**
The current sequence is draw check, `legal_moves`, empty check, then TT
probe. So every transposition-table cutoff still pays for full legal move
generation, which is the single most expensive thing at that node. This
is the exact cost staged generation exists to avoid, and it caps the
benefit of staging before staging is even attempted. Reordering requires
a way to distinguish "no legal moves" from "not yet generated," most
likely by generating on demand after the probe and handling the terminal
case at that point instead.

**Returning a TT move before generation needs a standalone move
validator.** Stockfish's first stage returns `ttMove` with no move list
in hand, which requires `Position::pseudo_legal(Move)`. turox currently
validates the TT move with `moves.as_slice().contains(m)`, which is
correct and cheap but requires the list to already exist. A
`Board::is_pseudo_legal(Move)` predicate is the prerequisite for both the
TT-before-generation fix and for staged generation generally. It is also
exactly the shape of function this repo's proptest-against-a-reference
discipline handles well: generate arbitrary moves, compare the predicate
against membership in `pseudo_legal_moves`.

## Recommended landing order

1. **Cutoff instrumentation.** Nothing below can be evaluated without it,
   and it is the cheapest item on the list. Establish the baseline
   first-move cutoff rate and per-depth EBF on startpos and Kiwipete.
2. **Fix `MovePriority`.** Delete `KillerCapture`, add promotions, move
   `LosingCapture` behind `Quiet`, and convert the sort key to
   `(MovePriority, i32)` so MVV-LVA survives into the sort. Pure
   ordering, no new state, and measurable immediately against step 1.
3. **`Board::is_pseudo_legal`, then TT probe before move generation.**
   Unblocks staging and pays for itself on its own.
4. **Killer moves.** Two slots per ply, quiet moves only, validated
   against the generated list. The standard first real ordering win.
5. **History heuristic.** Butterfly `[stm][from][to]`, `depth * depth`
   bonus, aging, and maluses on searched-but-no-cutoff quiets. This is
   the largest expected gain, because quiets are currently entirely
   unordered, and it is the hard precondition for LMR.
6. **SEE, via the swap algorithm.** Makes the winning/equal/losing split
   correct instead of MVV-LVA's approximation, and doubles as quiescence
   pruning.
7. **Countermove, then counter-move history / continuation history.**
   Score bonuses layered onto step 5's table rather than new ranks.
8. **Staged generation.** A performance refactor once every scorer
   exists, not a strength change. Doing it earlier means rewriting the
   phase machinery each time a scorer is added.

LMR, PVS, and null move pruning sit after all of this and are where the
EBF actually falls toward 2 or 3. They are out of scope for this ticket
but they are the reason the ticket matters: every one of them assumes the
move list is already sorted well.

## What this note did not cover

Time-boxed research, so the following were left out and are worth a
follow-up if they become relevant:

- **Relative history heuristic** and **butterfly boards** as separate
  pages. Both are named in the History Heuristic page's variant list and
  summarized from there, not read directly.
- **Refutation tables**, **last best reply**, **guard heuristic**, and
  the neural approaches (Chessmaps, Neural MoveMap) named on the Move
  Ordering page. All are listed there as less common.
- **Enhanced transposition cutoff (ETC)**, named on the Move Ordering
  page, not read.
- **Stockfish's history bonus and malus formulas** in `movepick.cpp` and
  `history.h`. The stage list was read directly; the specific gravity and
  bonus constants were not, and they are tuned to Stockfish's search
  rather than transferable.
- **Actual measured deltas per technique.** CPW does not publish
  per-technique Elo or node-count reductions in a form worth quoting, and
  the honest answer is that turox should get these from its own
  `tools/selfplay/sprt.sh` and from step 1's instrumentation rather than
  from the literature.
- **Move ordering under multithreading**, irrelevant while search is
  single-threaded but a real consideration for shared history tables
  later.
