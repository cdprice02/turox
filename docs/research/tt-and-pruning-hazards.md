# Search correctness hazards with a transposition table and pruning

Research note. Audits turox at `a5ad984` (`feat: TT-informed move ordering`)
against the known correctness hazards of a searching engine that carries a
transposition table and, later, pruning.

Every hazard here shares a shape: it compiles, it reads as correct, the
existing tests stay green, and it costs playing strength that nothing
attributes back to it. That is the same failure mode this repo already has a
documented history with in `Board::make_move`, the castling rook lookup, and
`between`/`line` direction classification. So each section below carries a
verdict against the current code and the test that would actually pin the
behaviour down, not just a description of the bug.

Verdicts use three labels:

- **(a) present today**: the bug is in `main` right now.
- **(b) safe, and why**: the code already avoids it, deliberately or not, and
  the reason is worth knowing before someone refactors it away.
- **(c) acquired when pruning lands**: not reachable yet, because the
  technique that triggers it does not exist.

## Summary of verdicts

| Hazard | Verdict |
| ------ | ------- |
| Mate scores: ply adjustment direction | **(a) present today** |
| Non-mate scores: ply adjustment applied at all | **(a) present today** |
| Graph history interaction (repetition) | (a) present today, bounded |
| Fifty-move clock absent from the key | (a) present today, bounded |
| Repetition detected inside the search, not only at the root | (b) safe |
| Bound derived from the original alpha | (b) safe |
| Aborted search storing a partial result | (b) safe |
| Quiescence result stored under a negamax depth | (b) safe |
| TT move validated before use | (b) safe |
| Always-replace with no generation counter | (b) correct, costly, and it amplifies the two (a)s above |
| Null-move and reduced-depth entries trusted at full width | (c) |
| Null-window (PVS) scores stored as exact | (c), and already prevented by construction |
| Forward pruning storing a score that was never searched | (c) |
| Root TT probe returning a cutoff with no move to play | (c) |

---

## 1. Mate scores and the ply adjustment

### The hazard

A mate score is not a property of a position alone. It carries a distance, and
what the search propagates is distance *from the root*: turox's own
`MATE` doc (`turox-engine/src/search/negamax.rs:21-31`) fixes the convention
as `Score::from(ply) - MATE` at a mated node, so a mate found nearer the root
scores further from zero and shorter mates win.

The transposition table is indexed by position, not by path, so a stored score
has to be in a form independent of where the storing node happened to sit. The
conversion is a distance change: from "plies to mate counted from the root" to
"plies to mate counted from this node." Chessprogramming's `Score` article
states the requirement without giving the formula: mate scores "need
ply-adjustment if stored as exact score inside the transposition table, and
re-adjustment if retrieving from TT."

Stockfish is the primary source for the formula
(`official-stockfish/Stockfish`, `src/search.cpp`):

```cpp
// Adjusts a mate or TB score from "plies to mate from the root" to
// "plies to mate from the current position". Standard scores are unchanged.
Value value_to_tt(Value v, int ply) {
    return is_win(v) ? v + ply : is_loss(v) ? v - ply : v;
}
```

and its inverse on the way out subtracts `ply` from a win and adds it to a
loss. Three cases, not one. A winning mate score moves one way, a losing mate
score moves the other way, and an ordinary positional score is not touched at
all.

### Verdict: (a) present today, in two independent ways

`turox-engine/src/search/tt.rs:190` stores:

```rust
score: score - Score::from(ply),
```

and `turox-engine/src/search/tt.rs:67` reads back:

```rust
let score = self.score + Score::from(ply);
```

Both are unconditional. That is the `is_loss` arm of `value_to_tt` applied to
every score in the table.

**Bug 1a: ordinary scores are corrupted whenever the probing ply differs from
the storing ply.** A plain positional score is a fixed number of centipawns; it
means the same thing regardless of how deep the node sits. Subtracting `ply` on
store and adding a *different* `ply` on probe shifts it by the difference. This
is the more damaging of the two, because it fires on nearly every cross-ply
transposition rather than only in mate lines, and because it silently changes
which move looks best without ever producing an implausible-looking number.

**Bug 1b: winning mate scores move in the wrong direction, by twice the ply
delta.** For a winning score the adjustment should add on store and subtract on
probe. Turox does the opposite of both, so the two errors compound rather than
cancelling. A mate found close to the root and re-encountered deeper is
reported as *further* away, and one found deep and re-encountered shallower is
reported as *closer* than it is. The second direction is the dangerous one: the
engine can announce and play toward a mate that is not there at that distance.

Losing mate scores happen to be handled correctly, because the unconditional
formula is exactly the `is_loss` arm. This asymmetry is why the bug survives
inspection: half the mate cases genuinely work, which is precisely the
`{Color}x{direction}`-shaped trap that `CLAUDE.md` warns about, here in the
form `{winning, losing, neither} x {store, probe}`.

### Why the existing tests do not catch it

`turox-engine/tests/tt_props.rs` has one round-trip property, and its own doc
comment names the constraint out loud: a store followed by a probe "at the
*same* `ply` and `depth` (the simplest case: no path-dependence to
reconstruct)." At equal ply the `-ply` and `+ply` cancel exactly, so the
property passes for every score including mates.

`turox-engine/tests/search.rs`'s mate puzzles (`white_delivers_mate_in_one`,
`black_delivers_mate_in_one`, `philidors_legacy_smothered_mate`) all build a
`Search` without `with_tt`, so `self.tt` is `None`
(`turox-engine/src/search/negamax.rs:143-148`) and neither the store nor the
probe path runs.

### The tests that do catch it

These were run against `a5ad984` and are reproduced with their real output.

```rust
// A plain positional score of +50, found at ply 2, depth 4, exact.
tt.store(key, 2, 4, 50, -MATE, MATE, any_move());
let e = tt.probe(key).unwrap();
assert_eq!(e.cutoff_score(4, -MATE, MATE, 2), Some(50)); // passes
assert_eq!(e.cutoff_score(4, -MATE, MATE, 6), Some(50)); // FAILS: got 54
```

```rust
// A node at ply 4 that is mate one ply below it: absolute mate ply 5,
// so the score there is MATE - 5. The same position reached at ply 2 by a
// shorter path still has the mate one ply below, so it is worth MATE - 3.
tt.store(key, 4, 3, MATE - 5, -MATE, MATE, any_move());
let e = tt.probe(key).unwrap();
assert_eq!(e.cutoff_score(3, -MATE, MATE, 2), Some(MATE - 3)); // FAILS: got MATE - 7
```

The losing-mate mirror of the second test passes today, and should be kept
alongside it rather than dropped: it is the case that pins the asymmetry, and
a naive "just flip the signs" fix would break it.

Above the unit level, the single highest-value test for this whole document is
a differential one, because it needs no oracle beyond the engine itself:

> For a given position and depth, `Search::new(history).search(board, depth)`
> and `Search::new(history).with_tt(&mut tt).search(board, depth)` must return
> the same score.

A transposition table is a memoisation of a pure function. Any score
difference is a table bug by definition. Run it over the existing mate puzzle
FENs, over `any_board()`, and critically over a *warm* table (search once,
then search again reusing the same `Tt`), since a cold table on a fresh
position produces far fewer cross-ply hits than the real cross-`go` reuse the
ADR was written for. This one property covers hazards 1, 2, 3 and 4 at once.

### Suggested shape of the fix

Bounds first, then three arms in each direction, mirroring `value_to_tt`. The
threshold wants naming (`MATE - MAX_PLY`, say) rather than being spelled
inline in two places where the two copies can drift.

Worth noting while fixing: an unbounded `score + Score::from(ply)` on `Score =
i16` can also push a near-`MATE` value above `MATE` itself, which is not a
legal score in this scheme and would be reported to a GUI as a nonsense mate
distance. Clamping, or the correct three-arm form, removes that too.

---

## 2. Graph history interaction

### The hazard

Chessprogramming's *Graph History Interaction* article states it directly: "the
same game position behaves differently when reached via different paths." A
transposition table stores board state and discards the move sequence that
produced it, which yields two symmetric failures:

- **False draws**: a score of zero cached from a path where a repetition was
  available, returned on a path where it is not.
- **Missed draws**: a decisive score cached from a path with no repetition,
  returned on a path where the side to move could actually force one.

The article credits Kishimoto and Müller (2004-2005) with general solutions
and Lincke and Andersson with related work on cycles in perfect-information
games. No cheap general fix exists; production engines accept a bounded
version of the bug.

### Verdict: (a) present today, in its bounded form

Two things in the current code limit the damage, and one does not.

The draw check runs *before* the probe. `turox-engine/src/search/negamax.rs:483`
calls `is_draw` and returns `Some(0)` at 484, and only then does line 497
probe. A node that is itself already drawn can therefore never take a TT
cutoff, and because the draw path returns early it is also never stored. That
closes the most visible direction: the table cannot talk turox out of a draw
it can see on the current path.

Turox also requires a genuine threefold. `draw.rs:35-41` looks for a *second*
prior occurrence, not a first. Many engines score the first repetition inside
the tree as a draw, which is cheaper and prunes cycles hard, but it multiplies
GHI exposure because far more nodes get a path-dependent zero. Not doing that
is a real mitigation, whether or not it was chosen for this reason.

What remains is the ancestors. `turox-engine/src/search/negamax.rs:538-542`
stores `max` at every node whose move loop ran, with no record of whether that
score came from a subtree containing a `is_draw` zero. A node three plies above
a repetition inherits a path-dependent score and files it in a path-independent
table.

This is amplified in turox specifically by ADR 0001. The table deliberately
outlives the `Search` that filled it, so an entry stored during the search for
move 20, under that move's repetition context, is probed during the search for
move 24 with four more positions on `history` and a different set of
repetitions reachable. That is the exact scenario the ADR was written to make
possible, and it is also the widest GHI window in the engine.

### The scenario that catches it

The reproducible version does not need an exotic position, only the cross-`go`
path:

1. Drive `uci::session::run` through a game that reaches a position where one
   side can force a perpetual, playing the moves in an order that repeats.
2. Search it, then play a move and search again, reusing the same `Tt`.
3. Compare against the same sequence with `tt.clear()` between every `go`.

A score difference is a GHI hit. This is worth writing as a session-level test
in `turox-engine/tests/uci_session.rs` rather than a search-level one, because
the cross-`go` reuse is a session property.

For a concrete position, the standard family is a perpetual-check draw in an
otherwise lost position: the defending side is down material, so a
path-independent evaluation reports a decisive score, and only the repetition
makes it a draw. `8/8/8/8/8/1k6/pP6/K7 w - - 0 1` and its relatives are the
usual textbook shape for the pure stalemate/fortress version.

### What not to do about it

Not much, yet. Full GHI correctness costs more than it returns at turox's
current strength. What is worth doing is the cheap defensive half: do not store
an entry whose score is a draw score returned from a descendant, or add a
"this subtree contained a repetition" flag that suppresses the store. Recording
the decision (including the decision not to fix it) is more valuable here than
the fix.

---

## 3. The fifty-move clock is not in the Zobrist key

### The hazard

Chessprogramming's *Repetitions* article names this alongside GHI: since the
halfmove clock is not part of the hash key, "retrieving a position from the
table may yield incorrect draw assessments." Two positions with identical
pieces, side to move, castling rights and en passant square share a TT entry
even when one is two plies from a fifty-move draw and the other is fifty moves
away from it.

Stockfish treats this as real enough to plumb the counter into the retrieval
path: `value_from_tt(Value v, int ply, int r50c)` takes the fifty-move counter
and downgrades win scores near the limit.

### Verdict: (a) present today, bounded the same way as GHI

`draw.rs:15-17` reads `Board::halfmove_clock()` directly and
`negamax.rs:483` checks it before the probe, so a node that is already a
fifty-move draw scores zero and is never stored. Good.

The gap is again the ancestors, and again the cross-`go` reuse widens it: an
entry stored at halfmove clock 40 is probed at clock 96 with no adjustment. A
won rook endgame cached early in a long shuffling phase keeps reporting won
after the clock has made it a draw.

### The test

A pair of FENs identical except for the halfmove clock:
`8/8/8/8/8/4k3/4p3/4K3 w - - 0 60` against
`8/8/8/8/8/4k3/4p3/4K3 w - - 96 60`. Search the first with a shared `Tt`, then
the second with the same table, and compare against searching the second with
a cleared table. Any divergence is this hazard. Note that this test only fires
once the search is deep enough to reach the boundary from the second position
but not from the first, so the depth has to be chosen against the clock
difference rather than picked arbitrarily.

---

## 4. Repetition inside the search versus only at the root

### The hazard

An engine that only checks repetition at the root cannot see a repetition its
own search creates, so it will happily play into a forced perpetual believing
it is winning, or fail to find one when it is losing. The *Repetitions* article
describes the standard remedy: an array of Zobrist keys covering game history
plus the current search path.

### Verdict: (b) safe, and the reason is subtle enough to be worth guarding

Turox already does the right thing, in a way that is easy to break by
accident.

`Search::history` (`negamax.rs:118-127`) holds the hashes of every position on
the path leading up to but *not including* the node being searched.
`negamax.rs:517` pushes `board.hash()` before descending and 522 pops after,
and `search_root` does the same at 408/413. `draw.rs:35-41` documents the
matching contract from the other side, so the caller cannot get the push/check
order backwards. It is seeded from the real game (`uci/session.rs:58, 80, 93`
pass a clone of the session's `history` into each `Search`), so repetitions
that already happened on the board are visible, not just ones the tree
revisits. `session.rs:76` clears it on `ucinewgame`.

Two details that make it hold and that a refactor could quietly remove:

- `quiescence` (`negamax.rs:571-617`) neither pushes nor checks. That is
  consistent, not an omission: it never calls `is_draw`, so it needs no stack
  entry, and captures are irreversible so a pure-capture line cannot repeat.
  If quiescence ever gains a check-evasion or quiet-move extension, it acquires
  both obligations at once.
- The push/pop pair straddles the recursive call and there is an early return
  between them at `negamax.rs:524` (`let score = -score?;`) which happens
  *after* the pop at 522. Correct as written. Moving the pop after the `?` is
  the natural-looking simplification, and it would leak a stack entry on every
  aborted node.

A property test worth adding: after any `Search::search` call, including one
aborted by `with_max_nodes`, `history.len()` equals the length it was seeded
with. That catches the leak directly rather than through its downstream draw
misdetections.

---

## 5. Node types, bounds, and the original alpha

### The hazard

The bound stored with a score has to be derived by comparing that score to the
window the node was *entered* with. Chessprogramming's *Node Types* article
gives the mapping: a score strictly inside `[alpha, beta]` is exact, a score at
or above beta is a lower bound, a score at or below alpha is an upper bound.
The trap is that `alpha` is a mutable local in a negamax loop. By the time the
loop ends, `alpha` has usually been raised by the best move found so far.
Comparing against that raised value classifies exact scores as upper bounds and
vice versa, which produces cutoffs that look plausible and are not.

### Verdict: (b) safe

`negamax.rs:512` captures `let original_alpha = alpha;` before the loop and
`negamax.rs:541` passes `original_alpha` to `store`, not the narrowed `alpha`.
`tt.rs:180-186` does the three-way comparison. `Bound`'s doc at `tt.rs:14-19`
names the hazard explicitly.

Fail-soft helps here too. `negamax` returns `max`, the real best score found,
rather than clamping to the window, so a `LowerBound` entry carries a genuinely
informative bound rather than the constant `beta`, and an `UpperBound` carries
a real ceiling rather than `alpha`. Store and search agree on this, which they
have to: storing a fail-soft score under a fail-hard bound interpretation is
another quiet way to widen a bound incorrectly.

---

## 6. What must not be stored

Three cases where the guard exists, and one where it exists as a side effect.

**Aborted searches.** A node interrupted mid-loop has a `max` that reflects
only the moves that finished, which is not a bound on anything. `negamax.rs:524`
propagates the `None` out with `?` before reaching the store at 538. **(b)
safe**, and worth a regression test with `with_max_nodes`, because "the abort
returns early, so nothing is stored" is exactly the kind of invariant a later
refactor to a `for`-loop-with-flag breaks silently.

**Quiescence results.** A quiescence score has no depth comparable to a
negamax depth, so storing it under `depth` would let a later `depth >= 3` probe
trust a capture-only search. `negamax.rs:504-506` returns the quiescence result
directly and never falls through to the store. **(b) safe.** The doc at
`negamax.rs:464-466` records the reasoning, which matters, because "quiescence
does not store" reads like an optimisation left undone rather than a
correctness requirement.

**The root.** `search_root` (`negamax.rs:368-433`) neither probes nor stores.
That means the root can never take a TT cutoff and thus can never return
without a move to play, which is a correctness property worth keeping even
though it was probably not the reason. It also means the root's own best move
is not in the table for the next iteration to order by, which is a strength
cost rather than a bug (its children at ply 1 are stored, so ordering at ply 1
still benefits).

**Illegal TT moves.** A 64-bit key match is not a proof of identity, and
`Move::from_bits` (`types/moves.rs:136-138`) is a bare wrapper that does no
validation, so a colliding entry could yield a bit pattern that is not a legal
move at all. `negamax.rs:508-510` filters the TT move against the generated
move list before it can reach `order_moves`, and `Move`'s `PartialEq` compares
raw bits, so an undecodable pattern is simply filtered out rather than
panicking in `flags()`. **(b) safe**, and this is the right guard.

---

## 7. Replacement scheme

### The options and their costs

Chessprogramming's *Transposition Table* article lists four families:

| Scheme | Mechanism | Cost |
| ------ | --------- | ---- |
| Always-replace | New entry overwrites unconditionally | Loses deep entries to shallow ones; cheapest possible code |
| Depth-preferred | Overwrite only on greater or equal depth | Deep entries survive, but a full table ossifies: stale deep entries from an earlier position block everything |
| Aging / generation | Store a generation counter, prefer replacing stale entries | Fixes ossification; costs a field and a comparison |
| Two-tier / bucket | Several slots per index, replace the lowest-depth one | Best behaviour of the four; most implementation cost, and it interacts with cache line layout |

Depth-preferred without aging is the classic trap: it is strictly better than
always-replace within one search and strictly worse across a game, because
nothing ever evicts a deep entry from a position that will never recur.

### Verdict: (b) correct but costly, and it amplifies hazards 1 and 3

`tt.rs:196` is unconditional always-replace, and `Entry` (`tt.rs:32-49`) has no
generation field. This is not a correctness bug on its own: every entry in the
table is a real result of a real search, and `depth` gates whether it can be
trusted (`tt.rs:64-66`).

The strength cost is the ordinary one. The overwhelming majority of stores come
from near-horizon nodes, so the shallow entries the search generates by the
million evict the small number of deep entries that were expensive to compute.
The usual next step is a two-tier scheme (depth-preferred in one slot,
always-replace in a second) with a generation counter, which also makes
`Tt::clear` (`tt.rs:124-126`) cheap: bumping a generation instead of writing
every slot.

Where it matters *here* is the interaction. Always-replace plus no aging plus
ADR 0001's cross-`go` lifetime means entries stored at one game ply are
routinely probed at a very different one. That is the exact condition under
which hazard 1 fires (different probing ply than storing ply) and hazard 3
fires (different halfmove clock). Fixing the ply adjustment is a prerequisite
for the table's persistence being a win rather than a liability; adding aging
without fixing it would make the corruption *less* frequent and correspondingly
harder to find.

One incidental note in the table's favour: `Option<Entry>` should occupy 16
bytes rather than 24, because `Bound` is a three-variant fieldless enum and
Rust can put the `None` discriminant in its unused bit patterns. `Tt::new`
(`tt.rs:98-113`) computes the entry count from `size_of::<Option<Entry>>()`
rather than assuming, so this stays correct if the layout ever changes.

---

## 8. Pruning interactions, none of which exist yet

Nothing in `negamax` prunes beyond plain alpha-beta today, so all of these are
**(c)**. What matters is which of turox's existing structures already defend
against them and which do not.

### Null-move pruning

Three separate hazards arrive with it.

*The reduced-depth entry.* A null-move search runs at `depth - R`. If its
result is stored under the full `depth`, a later full-width search probes it,
sees a sufficient depth, and takes a cutoff backed by a search that never
happened. Turox's `cutoff_score(min_depth, ...)` (`tt.rs:63-66`) is already the
correct mechanism: an entry stored with the reduced depth genuinely searched
cannot satisfy a probe asking for more. The discipline is therefore "store the
depth you actually searched," and the existing `min_depth` gate enforces the
rest. This is the single most important thing the current design already gets
right for future pruning.

*Unproven mate scores.* Stockfish guards this explicitly:

```cpp
if (nullValue >= beta && !is_win(nullValue))
```

with the comment "Do not return unproven mate or TB scores." A null-move
search can manufacture a mate score that depends on the opponent having passed,
which is not a legal continuation. Returning it, and worse storing it, puts a
fabricated mate distance in the table. Given hazard 1 is already live, this one
should not be attempted before the ply adjustment is fixed.

*Zugzwang.* The null-move observation fails where passing is the best available
option, most often in pawn endgames. Chessprogramming notes that verification
search is the standard defence and that its value is disputed (Hyatt "concluded
it does not help at all in Crafty"). The usual cheap guard is to disable null
move when the side to move has no non-pawn material, which turox can already
answer from `eval::phase`.

*The en passant square in the null-move key.* Making a null move must flip the
side to move *and* clear the en passant square in the Zobrist key, because the
real position reached after a pass has no en passant target. Getting this wrong
makes the null child's key collide with a genuinely different position, and it
is a pure hashing bug with no visible symptom other than strength. This is
exactly the `{Color}x{direction}`-shaped territory `CLAUDE.md` flags, so it
wants a concrete asymmetric test: a position with a live en passant target,
null-moved, hashed, compared against the same position reached by a real
quiet move that clears the target.

### Late move reductions

The rule is that a reduced search that fails high must be re-searched at full
depth, and the TT rule follows from it: whichever search produced the score you
store, store *its* depth. Storing the nominal depth after a reduced search is
the same bug as the null-move case, arriving through a different door.

`negamax`'s current store passes the same `depth` it was called with
(`negamax.rs:541`), which is correct today because there is exactly one search
per node. Once a node can run a reduced search and then a re-search, `depth` at
the store site stops being the depth that produced `max`, and the store needs a
separate variable.

### Principal variation search / null-window scouting

A scout search runs with the window `(alpha, alpha + 1)`. A score cannot land
strictly between those, so it is always a bound and never exact. Storing such a
score as `Exact` would be badly wrong.

Turox is already safe here by construction: `tt.rs:180-186` derives the bound
from the actual `alpha`/`beta` it was handed, so a null-window call
automatically yields `UpperBound` or `LowerBound`. This only holds as long as
the bound keeps being *derived* rather than passed in by the caller, which is
worth remembering if `store` ever grows a `bound` parameter to reduce its
argument count.

### Futility pruning, razoring, and reverse futility

These skip a subtree based on a static estimate, so the "score" they produce
was never searched. It must not be stored, and it must not raise `max` in a way
that gets stored either. The rule is that a store belongs only on a path whose
move loop actually ran, which is what `negamax.rs:538` currently does and what
a `return` inserted above it for futility would preserve.

### Mate distance pruning

The standard optimisation narrows the window to the best mate already
guaranteed, which by definition operates on mate scores at a specific ply. It
cannot be implemented correctly on top of the current adjustment, and adding it
would probably surface hazard 1 as visible misbehaviour rather than as quiet
strength loss.

### Root TT probing

If aspiration windows or an explicit PV table later cause `search_root` to
probe, the root gains the ability to return a cutoff score with no move
attached, and the engine has nothing to send as `bestmove`. The current design
sidesteps this by not probing at the root at all; the property to preserve is
that `search_root` always returns a move for a position with legal moves.

---

## 9. Search instability

Worth naming even though it is not a bug. TT cutoffs make the search
non-monotonic: the same position at the same nominal depth can return different
scores on different iterations depending on what happened to be in the table,
and an exact-bound cutoff in the middle of a PV truncates the reported line.
Chessprogramming lists this under the transposition table's own caveats. It is
the reason a differential "with TT versus without TT" test is only sound for
the *score*, not for the node count or the reported move, and the reason
`tools/selfplay/sprt.sh` rather than `cargo bench` is what decides whether a TT
or pruning change is an improvement.

---

## Recommended order of work

1. Fix the ply adjustment (hazard 1). It is a live bug, it is small, and every
   other item on this list is either measured through it or made worse by it.
2. Add the differential "search with TT equals search without TT" property,
   over a warm table, before anything else touches the table.
3. Add the history-stack-length invariant (hazard 4) so the push/pop pairing
   stays pinned.
4. Decide and record what turox does about GHI and the fifty-move clock
   (hazards 2 and 3), including deciding to accept them.
5. Replacement scheme and aging (hazard 7), which is a strength change and
   wants an SPRT run rather than a test.
6. Pruning (hazard 8), storing the depth actually searched at every site.

## Sources

- Chessprogramming wiki, *Transposition Table*:
  <https://www.chessprogramming.org/Transposition_Table>
- Chessprogramming wiki, *Score*:
  <https://www.chessprogramming.org/Score>
- Chessprogramming wiki, *Graph History Interaction*:
  <https://www.chessprogramming.org/Graph_History_Interaction>
- Chessprogramming wiki, *Repetitions*:
  <https://www.chessprogramming.org/Repetitions>
- Chessprogramming wiki, *Node Types*:
  <https://www.chessprogramming.org/Node_Types>
- Chessprogramming wiki, *Null Move Pruning*:
  <https://www.chessprogramming.org/Null_Move_Pruning>
- Chessprogramming wiki, *Late Move Reductions*:
  <https://www.chessprogramming.org/Late_Move_Reductions>
- Stockfish, `src/search.cpp` (`value_to_tt`, `value_from_tt`, null-move
  guard): <https://github.com/official-stockfish/Stockfish>
