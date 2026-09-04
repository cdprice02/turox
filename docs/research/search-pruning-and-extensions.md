# Search pruning and extensions

Research findings on what the chess-programming literature says about
forward pruning, reductions, and extensions, and which of it turox should
adopt given a measured effective branching factor of 9.4 over depths 5 to
8.

Primary source throughout is the Chess Programming Wiki, which the repo
already treats as its reference. Where a claim is quoted it comes from
the page named in the Sources section. The wiki is deliberately sparse on
Elo numbers, so measured per-feature deltas are taken from a second
primary source: the published SPRT logs of Blunder, an engine in roughly
the strength band turox is heading for. Those are one engine's self-play
numbers against its own previous version, not universal constants, and
are used here only for ranking effort, never as targets.

## Why the ordering problem comes before the pruning problem

Almost every technique below is a bet on move ordering. Alpha-beta's
theoretical floor is "about square root of the average branching factor,"
which for chess's 35 to 38 moves per position gives roughly 6. Modern
engines beat that floor only because reductions and forward pruning
search most moves at less than full depth: "Alpha-beta enhancements,
transposition tables, null move pruning and late move reductions further
reduce the EBF below three, strong programs even near or below two."

The bet is the same in each case. Principal variation search assumes the
first move is best and pays a re-search when it is not. Late move
reductions assume moves ordered late are bad and pay a re-search when
they are not. Null-move pruning assumes a fail-high at reduced depth
predicts a fail-high at full depth. If the ordering is poor, every one of
these pays its penalty more often than it collects its saving, and some
of them lose strength outright rather than merely failing to gain.

turox's 9.4 is worse than the perfect-ordering floor. That is the signal
that its ordering, not its pruning, is what is currently missing.
Ordering work is therefore not a prerequisite in the bureaucratic sense
of "do it first because the dependency graph says so"; it is the thing
that determines whether the pruning work pays at all.

## What turox has, and what the techniques below assume

Present today: fail-soft alpha-beta, iterative deepening, quiescence
capped at 8 plies, MVV-LVA ordering, an always-replace transposition
table, and TT-move ordering in flight.

Absent, and assumed by large parts of the literature:

- **Killer moves and a history table.** Every reduction and late-move
  pruning scheme needs a way to rank quiet moves. turox ranks all quiets
  equally (`MovePriority::Quiet`), so "moves ordered late" currently
  means "whatever order move generation emitted," which is close to
  arbitrary. Reducing on that basis reduces good moves as often as bad
  ones.
- **Static exchange evaluation.** Used to separate winning from losing
  captures more accurately than MVV-LVA, to keep losing captures out of
  quiescence, and as the exemption test in several reduction schemes
  ("some programmers don't extend checks (and captures) with negative
  SEE or even reduce them"). turox approximates good-versus-bad captures
  from raw piece values, which misjudges any exchange with more than one
  defender.
- **Node-type awareness (PV, cut, all).** Almost every modern condition
  is phrased in terms of node type: null-move pruning is disabled
  "inside a non-PV node" or restricted to expected cut nodes, LMR
  exempts "PV-Nodes in PVS search", multi-cut applies "at expected
  Cut-nodes". turox has no notion of node type because it has no
  zero-window searches to distinguish them. PVS is what creates that
  vocabulary, which is why it lands before the things that depend on it.
- **A principal variation.** `SearchResult` carries a single best move,
  not a line. Aspiration windows and singular extensions are both easier
  to debug against a visible PV, and the `MovePriority::PrincipalVariation`
  rank has no producer without one.
- **Room in the node for early exits.** `negamax` currently calls
  `legal_moves` before it probes the transposition table, and long
  before any point where a pruning test could sit. Reverse futility
  pruning, null-move pruning, and razoring all get most of their value
  by returning *before* move generation. Landing them into the current
  node shape would still cut nodes, but would keep paying full move
  generation at every node it prunes. Restructuring the node so the
  terminal checks, the TT probe, and the static-eval pruning tests all
  precede move generation is a prerequisite for the pruning tranche, not
  an optimisation to do afterwards.

## Two correctness gaps found while reading the current code

Neither is part of the research question, but both interact directly
with the techniques below and would corrupt their measurement.

**Quiescence stands pat while in check.** `quiescence` computes
`stand_pat = evaluate(board)` unconditionally and then filters the move
list to captures. In a position where the side to move is in check, that
returns a static score for a position that may be checkmate, and
considers only capture evasions rather than all evasions. The wiki is
explicit: "Stand pat is not allowed if we are in check". The check
extensions page frames the fix as itself a form of check extension: "If
a program does not consider checks in the quiescence search, then we
should take care that it does not enter it while in check." This
undercuts any measurement of check extensions, since the tactical holes
a check extension is supposed to close are partly caused by this.

**The transposition table ply-adjusts every score, not just mate
scores.** `Tt::store` writes `score - Score::from(ply)` and
`Entry::cutoff_score` returns `self.score + Score::from(ply)`. That
round-trips correctly only when a position is probed at the same ply it
was stored at. Probe the same position one ply deeper via a transposition
and the score comes back shifted by the ply difference. The convention
the adjustment is imitating applies to mate scores alone: "Below the
root the absolute values of mate scores are usually decremented by ply
distance to the root, to encourage programs to prefer shorter mates if
winning or longer mates if losing," and those "scores need ply-adjustment
if stored as exact score inside the transposition table, and
re-adjustment if retrieving from TT." A quiet positional score has no
ply component to remove. The distortion is small in centipawns but grows
with depth, and it is exactly the kind of quiet score noise that makes a
forward-pruning SPRT unreadable.

## The techniques

Each entry gives what it does, what it depends on, the reported gain
where a primary source quantifies one, and the specific ways it is known
to be got wrong.

### Principal variation search

Search the first move with the full window, every later move with a null
window `[alpha, alpha+1]`, and re-search with the full window only when
the null-window search beats alpha. The null-window search cannot return
an exact score, only "worse than alpha" or "better than alpha," which is
all that is needed "to prove a move is worse or not than an already safe
score from the principal variation," and it is much cheaper because
every node beneath it is a cut node or an all node.

PVS is functionally identical to NegaScout: "the version of
principal-variation search as mentioned by Marsland (1986) is identical
to the version of negascout as mentioned by Reinefeld (1989)." The
difference is presentation, one routine or two.

Depends entirely on ordering. Modern programs fail high on the first
move roughly 90% of the time, and only then is the expected saving
positive; the wiki puts it at "with a good move ordering we expect to
save about 10% of a search effort." Ten percent is a modest direct
return, which makes PVS look unattractive on its own. It is not: PVS is
what creates the null-window searches that make node types meaningful,
and every reduction and pruning condition below is phrased in terms of
node types. It is infrastructure first and a speedup second.

Ways it is got wrong:

- Re-searching at non-PV nodes. The guard `beta - alpha > 1` must gate
  the re-search, otherwise a node that was already a null-window node
  re-searches itself pointlessly.
- Choosing the re-search window. Implementations differ over
  `{score, beta}` versus `{alpha, beta}`, and the choice changes how
  search instability manifests rather than whether it does.
- Combining with aspiration windows without handling a root fail-low,
  where "one must be aware that in this case also a normal window search
  might fail, leaving the program with no move and no PV."

### Aspiration windows

Instead of searching each iteration with `[-MATE, MATE]`, search with a
narrow window around the previous iteration's score. "Typical window
sizes are 1/2 to 1/4 of a pawn on either side of the guess." Narrower
bounds mean "more beta cutoffs are achieved, and the search takes a
shorter time," at the cost that "if the true score is outside this
window, then a costly re-search must be made."

On a fail, only the bound that failed moves: "It's important to note
that the bound that didn't fail is unchanged." Stockfish and similar
engines "start with a rather small aspiration window, and increase the
bound that fails in an exponential fashion" rather than jumping straight
to an infinite window.

Blunder measured aspiration windows at "Elo difference: 22.6 +/- 11.8".

Ways it is got wrong:

- Widening both bounds on a fail, which throws away the half of the
  window that was still valid.
- Not handling a root fail-low, which is the "no move and no PV" case
  above. The wiki records two accepted responses: finish the root move
  list before re-aspiring, or re-aspire immediately. Hyatt argues for
  the latter, because when the previous best move fails low "each and
  every one will fail low, and then you get to start over with a lowered
  alpha value after spending all that time."
- Applying a narrow window when the previous score was a mate score,
  where the natural window has no meaning.
- Aspiration windows are one of the two named causes of visible search
  instability, and one of the recorded responses is "abandoning
  aspiration windows entirely."

turox's root currently searches `[-MATE, MATE]` every iteration, so this
is a small, self-contained change once PVS exists. It should not land
before PVS, because the interaction between the two is where its
difficulty lives.

### Null-move pruning

Give the opponent a free move. If a reduced-depth search of the
resulting position still fails high over beta, conclude that the real
best move would too, and cut. The premise is the null move observation:
"In almost all chess positions, making a null move (passing a turn) is
worse than the best legal move," so "if a reduced search on a null move
fails high over beta, then by the null move observation, we can be quite
confident that the best legal move would also fail high over beta."

Reduction R is typically 2 or 3 fixed, or adaptive: "a depth scaling
factor, such as depth / 3, can be added to the depth reduction," and "a
factor scaled by the difference between (potentially TT-corrected)
evaluation and beta can also be added."

This is the single largest reported gain of anything in this document.
Blunder measured plain null-move pruning at "Elo difference: 116.0 +/-
25.2", and a later refinement to `R = 3 + depth/6` at a further "Elo
difference: 13.9".

The exception it is built on is zugzwang: "Zugzwang is a position in
which it is disadvantageous to move," which makes the null move
*better* than any legal move and the whole inference backwards. "Most of
the time it happens in late endgames, specially pawn endings, the most
obvious example being a KPK endgame."

The standard guards, all of which must be present together:

- Not while the side to move is in check, since the null move would
  leave the king capturable.
- Not in positions "where the side to move has only king and pawns,"
  which is where zugzwang concentrates. Fruit's rule is representative:
  "no null move pruning when down to king and pawns for the side to
  move, or when in check."
- Not inside a PV node.
- Only when the static evaluation is already at or above beta, since
  otherwise the null-move search is unlikely to fail high and is pure
  cost.
- Not consecutively, though the wiki notes this one is not clearly worth
  it: "some implementations also disables consecutive null moves,
  although they may not have an effect on engine strength."

Verification search is the systematic answer to zugzwang: on a null-move
fail-high, re-search at reduced depth without the null move and only cut
if that agrees. Its record is genuinely mixed. "Robert Hyatt tested
Verified Null-Move Pruning extensively with a lot of variations and depth
reductions for the verified search, and concluded it does not help at all
in Crafty." Vincent Diepeveen's double null move (two consecutive null
moves to detect zugzwang) is the other named approach. The honest
reading is that the material guards do most of the work and verification
is a refinement to test separately, not a mandatory part of the first
implementation.

Other ways it is got wrong:

- Returning a mate score from a null-move search. A mate found after
  giving the opponent a free move is not a real mate. Scores in the mate
  range from the null-move search must be clamped rather than returned
  or stored.
- Losing the "I get mated if I do nothing" information: "some kind of
  fail-soft framework is necessary to recognize 'I get mated, if I do
  nothing', otherwise the hard bound of a null window null-move search
  around beta will limit the upper bound to beta-1." turox is already
  fail-soft, so this one is free.
- Making a null move without clearing the en passant square, which
  changes the position hash and legality of the reply.
- Repetition and draw detection. A null move is not a real move, so it
  must not be pushed onto the repetition history the way `negamax`
  currently pushes `board.hash()` before each child.

Null-move reductions are the safer cousin: on a null-move fail-high,
"the search is reduced by four plies, rather than pruned," with
`R = depth > 6 ? 4 : 3`. Because nothing is cut outright they "are
therefor less vulnerable to Zugzwang and might even applied in (late)
endings."

### Late move reductions

Search the first one or two moves at full depth; search the rest at
reduced depth, and re-search at full depth only if the reduced search
beats alpha. "Classical implementation assumes a re-search at full depth
if the reduced depth search returns a score above alpha."

This is the technique that actually collapses the branching factor: it
"can reduce the effective branching factor to less than 2, depending on
the reduction conditions." Blunder measured "Elo difference: 61.3 +/-
17.6".

Modern reduction amounts are logarithmic in both depth and move count,
for example Obsidian's `0.99 + ln(depth) * ln(moves) / 3.14` or
Ethereal's `0.7844 + ln(depth) * ln(moves) / 2.4696` for quiet moves.
Engines "clamp reduction to ensure correctness" so a reduction can never
exceed the remaining depth or go negative.

Standard exemptions: depth below 3, captures and promotions, moves while
in check, moves giving check, moves that triggered an extension, PV
nodes, killer moves, passed pawn moves, and "moves with good relative
history."

Note how many of those exemptions turox cannot currently express.
Killers, history, and node type are all absent, and "in check" and
"gives check" are cheap but not currently computed at the point the move
loop would need them. LMR without those exemptions reduces the moves it
should be searching. This is the strongest argument in the whole
document for landing killers and history before the reduction tranche.

Ways it is got wrong:

- Reducing without re-searching on a fail-high, which is no longer a
  reduction but a silent prune.
- Re-searching with the wrong window, which produces exactly the
  contradictory results search instability is made of.
- Applying it at cut nodes indiscriminately: forum consensus recorded on
  the wiki is that "LMR at CUT nodes can be arbitrarily bad."
- Late endgames, where the move count is small, the ordering signal is
  weak, and "problems with LMR in late endgames" are a recurring report.

### Late move pruning (move count based pruning)

Where LMR reduces late quiet moves, late move pruning skips them
outright once the move count passes a depth-dependent threshold. The
wiki files it as a variant of futility pruning, "combining the ideas of
Fruit's History Leaf Pruning and Late Move Reductions," and classifies
it as forward pruning that skips moves at all nodes.

Blunder measured "Elo difference: 21.9 +/- 11.4".

It is strictly more dangerous than LMR because there is no re-search to
recover from a wrong guess, and it depends on the same quiet-move
ordering signal. It belongs after LMR and after history, never before.

### Futility pruning and reverse futility pruning

**Futility pruning** skips moves near the horizon that cannot plausibly
raise alpha. It "discards moves that have no potential of raising alpha,
which in turn requires some estimate of a potential value of a move.
This is calculated by adding a safety margin to the evaluation of the
current position." Classically applied at frontier nodes (depth 1), then
extended to pre-frontier nodes (depth 2) "with the greater margin. If at
depth 1 the margin does not exceed the value of a minor piece, at depth
2 it should be more like the value of a rook." Modern engines "also
perform futility pruning at non-leaf nodes, and scales margin by depth."

Exempt: captures and moves that give check. Disabled entirely when "the
side to move is in check, or when either alpha or beta are close to the
mate value, since it would leave the program blind to certain
checkmates."

The subtle bug the wiki calls out by name: "futility pruning requires
checking for the existence of at least one legal move to avoid returning
erroneous stalemate scores." Prune every move at a node and the node
looks moveless, which turox's own terminal check would read as
stalemate.

Blunder measured futility pruning at "Elo difference: 37.4 +/- 13.4",
plus a further "Elo difference: 19.7" from more aggressive margins.

**Reverse futility pruning**, also called static null move pruning, is
the mirror image: if the static evaluation is already so far above beta
that no reply can plausibly drag it back, return immediately. The
condition is "eval >= beta + margin", the margin scales with depth ("e.g.
150 * depth"), and the return is fail-soft, `return eval`. It "is a
special case of null move pruning without explicitly making one," which
is precisely why it is attractive to land first: it captures a large
share of null-move pruning's premise at a fraction of the implementation
risk, with no null move to make, no en passant square to clear, and no
repetition bookkeeping to get wrong.

It must be skipped when the position is in check, at PV nodes, and near
mate scores, for the same reasons futility pruning must be.

Blunder measured static null move pruning at "Elo difference: 57.1 +/-
16.9", second only to null-move pruning itself. That combination of high
measured gain and low implementation risk makes it the best
Elo-per-effort item in this document.

### Razoring

The riskiest of the family and the weakest performer. The original form,
Birmingham and Kent 1977, pruned at pre-frontier nodes: "Once a move
statically does no longer improve alpha, this and all further moves
(sorted below) are pruned." Amir Ban's modern reading is milder, a
search "to a reduced depth, typically one less than normal depth" rather
than skipping subtrees entirely.

The form actually used today is the drop-into-quiescence variant: at
null-window nodes near the horizon, if static evaluation falls short of
beta by roughly three pawns, run quiescence instead of a full search and
return its score. Stockfish's 2022 reintroduction uses a quadratic
margin, `if (eval < alpha - 512 - 293 * depth * depth)`.

Classical razoring is "known for being risky," and modern engines
"prefer safer alternatives like futility pruning, null move pruning, and
depth-based reductions rather than aggressive forward pruning."

Blunder's measurement matches that reputation: "Elo difference: 7.9 +/-
7.4" over 4550 games, the smallest gain of any technique it logged, and
requiring more than twice the games of most entries to resolve at all.
This belongs last in the pruning tranche, if at all. Note also that
razoring's value depends on quiescence being trustworthy, since it hands
the position to quiescence and returns that score. With turox's
quiescence standing pat in check, razoring would be handing positions to
a search that can currently be wrong about them.

### Check extensions

Search one ply deeper when a move gives check, or when the side to move
is evading one. The justification is that checks are forcing and the
subtree stays narrow: "the number of replies to check is limited, so we
do not have to be afraid of a search explosion." The cost of not doing
it is the horizon effect, since "not extending checks may easily lead to
the horizon effect, delaying the threat so far that the program cannot
see it."

The refinement worth knowing: not every check is worth a ply. "Some
programmers don't extend checks (and captures) with negative SEE or even
reduce them," and Hyatt "claimed a significant gain in Crafty by doing
so." That refinement needs SEE, which turox lacks.

Extensions in general carry a standing explosion risk: "Care must be
taken so that the search is not extended infinitely." Programs bound
this "with either a maximum limit, or via other conditions, such as
depth or iteration," or with fractional plies rather than whole ones.

The important framing point for a heavily-reducing engine, and the
reason check extensions should not land before the reductions do: "In
contemporary, heavily reducing programs former typical extensions are
often used in an inverted manner: to flag moves as exempt from
reductions." In an engine with no reductions, a check extension makes
the tree strictly bigger, which is the opposite of what a 9.4 EBF needs.
In an engine with LMR, the same knowledge is spent as a reduction
exemption and costs nothing. Landing check extensions before LMR
therefore buys tactical accuracy at the direct cost of depth, and would
likely fail an SPRT that the same change would pass after LMR.

The one part of check extensions that is unambiguously worth doing now
is the quiescence guard described earlier, which the wiki explicitly
classifies as "also a form of check extension."

### Singular extensions

Extend a move that is much better than all of its alternatives, on the
grounds that a forced line deserves more depth. Introduced by
Anantharaman, Campbell and Hsu for Deep Thought in 1988, it extends "at
expected PV- and Cut-Nodes, if one move seems to be a lot better than
all of the alternatives."

Singularity is proven, not guessed. Take the TT move, exclude it from
the move list, and run "a reduced search with a null window lowered by
some significant margin" over the remaining moves. The move is singular
"only if all alternatives fail below that window."

This is the most demanding technique in the document. It needs a
transposition table good enough to trust the stored move, a depth and
bound condition on the entry (Stockfish 1.6 restricted it "to moves
found in the TT with a lower bound flag set"; modern versions allow
"singular search on TT entries with an exact score"), and an exclusion
key so the verification search does not read back its own excluded-move
result from the table. Anantharaman flagged "implementation issues
related to the transposition table" in 1991 and the exclusion key
remains the named pitfall.

It also needs an always-replace table to become something better. turox
stores always-replace with no depth preference, which means a shallow
visit silently overwrites a deep one. Singular extensions read TT depth
as a precondition, so they are directly degraded by that.

Blunder measured "Elo difference: 6.8 +/- 7.8" over 4000 games, which
did not even clear its own error bar cleanly. That is a small return for
a large amount of subtle machinery, and it argues firmly for placing
singular extensions last.

Its byproduct is worth more than it looks: when the exclusion search
fails high above beta, multiple moves are good, which is a multi-cut
condition. "If the singular search fails high, and the bounds at which
they were searched at is greater than or equal to beta, we can predict
that multiple moves fail high."

### Internal iterative deepening, and its modern replacement

IID handles a node where "a program has no best move available from a
previous search PV or from the transposition table" by searching the
position to reduced depth first and using that search's best move as the
first move at full depth. Typical reductions are "-1, -2, /2, or /4", and
it is usually gated on depth ("only use IID if depth > 5") and node type
("Most only use IID in PV-Nodes, but it is also possible to use it at
predicted Cut-Nodes").

The honest assessment on the wiki is that it is "pretty much a washout on
average", adopted by Deep Thought because "it makes the search times more
predictable by avoiding those isolated instances when the search time
suddenly becomes 10 times larger than expected," and described as
insurance: "Most of the time it was not needed, but then it also costs
very little. And now and then it saves you big time," with the note that
the benefit is largest "for engines with weak move ordering."

Blunder measured "Elo difference: 10.9 +/- 11.7", an error bar wider
than the effect.

Internal iterative reductions are the modern replacement and are far
simpler: rather than searching to find a move, "simply reduce the depth
of the entire node, in the hope that the node must not be very important
as there was no hash move present." Same gating conditions, no extra
search. Introduced in Rebel in 2020 and since adopted by Stockfish and
Ethereal. Given equal or better results for a fraction of the code, IIR
is the version turox should implement if it implements either.

The caveat that matters for turox specifically: the phrase "weak move
ordering" describes turox exactly today, which makes IID look more
attractive than its numbers suggest. But the correct response to weak
ordering is to fix the ordering, not to buy insurance against it. IID
and IIR both belong after killers and history, at which point their
value will be lower and their measurement will be honest.

### Multi-cut

At an expected cut node, search the first M moves (typically 3 to 6) at
reduced depth R. If C of them (with C less than M) fail high, prune the
whole node and return beta. Björnsson introduced it in 1998 and it has
been "successfully employed by several of the world's strongest
commercial chess program for a number of years."

Its distinguishing claim is about which risks it takes: "pruning
decisions based not only on the risk of pruning off relevant lines of
play, but also on the likelihood of such an erroneous pruning decision
affecting the move decision."

It is the exact inverse of singular extensions, which the wiki notes
directly: singular extensions ask whether one move is much better than
all others, multi-cut asks whether several are good enough. Modern
implementations get multi-cut for free out of the singular search rather
than implementing it separately, which is the strongest argument for
treating it as a follow-on to singular extensions rather than as an
independent item.

turox cannot implement it today at all, since "expected Cut-nodes" is
not a category the search can name without PVS.

The related probabilistic technique, ProbCut, is worth knowing about and
worth skipping. It uses linear regression between shallow and deep
search scores, and the two reasons it historically failed in chess are
both relevant here: "Null-move and ProbCut are based on similar ideas,
as a result they tend to prune the same type of positions," and "Chess
searches tend to make more mistakes than Othello searches," which
widens the error term until reliable cuts are hard to find.

## Recommended landing order

The order below is by Elo-per-effort with dependencies respected, and it
departs from the order the question listed them in. Each numbered item
is meant to be one independently-SPRT-testable unit, except where the
literature treats a pair as inseparable.

**Tranche 0, prerequisites that are not themselves strength features.**

1. Fix the quiescence in-check stand pat, and fix the transposition
   table so ply adjustment applies only to mate-range scores. Neither is
   a pruning technique; both would silently corrupt the SPRTs of
   everything that follows.
2. Restructure the node so terminal checks, the TT probe, and a slot for
   static-eval pruning tests all precede move generation. Nothing to
   measure, but every pruning technique below collects less without it.

**Tranche 1, ordering. This is where the 9.4 actually moves.**

3. Killers and history together, per the convention that treats them as
   one unit. This is the enabling change for LMR, late move pruning, and
   every reduction condition that mentions quiet-move quality. Blunder
   measured history alone at 19.0 +/- 12.9, but its real value is that
   it makes the next tranche work at all.
4. Principal variation search. Modest direct return, roughly 10% of
   search effort, but it is what gives the search a notion of PV, cut,
   and all nodes, which most later conditions are written in terms of.
5. Aspiration windows. Small, self-contained, and cheap once PVS exists;
   Blunder measured 22.6 +/- 11.8. Landing it immediately after PVS
   means the two get debugged together, which is where their known
   interaction lives.

**Tranche 2, the pruning that pays.**

6. Reverse futility pruning. Highest Elo-per-effort in the document:
   57.1 +/- 16.9 measured, and a handful of lines with no null move to
   make and no repetition bookkeeping to corrupt.
7. Null-move pruning with the material and in-check guards. Largest
   single gain anywhere here at 116.0 +/- 25.2, and the largest
   correctness surface. Verification search should be a separate later
   experiment rather than part of this, given Hyatt's negative result.
8. Late move reductions. 61.3 +/- 17.6, and the technique that actually
   drives EBF toward 2. Only meaningful after item 3.
9. Futility pruning at frontier and pre-frontier nodes. 37.4 +/- 13.4.
   Watch the stalemate-score trap.
10. Late move pruning. 21.9 +/- 11.4. After LMR, since it is the same
    bet with no re-search to catch a mistake.

**Tranche 3, small or expensive.**

11. Static exchange evaluation. Not a strength feature by itself, but it
    sharpens capture ordering, cleans losing captures out of quiescence
    (Blunder measured SEE-based quiescence pruning at 25.9), and unlocks
    the negative-SEE exemptions in the reduction conditions above.
12. Check extensions, deliberately after LMR, so the knowledge is spent
    as a reduction exemption rather than as extra tree.
13. Internal iterative reductions rather than internal iterative
    deepening. IID measured 10.9 +/- 11.7; IIR is reported as equal or
    better for far less code.
14. Singular extensions, and multi-cut as its byproduct. 6.8 +/- 7.8
    measured, the most machinery for the least return, and dependent on
    a depth-preferred replacement scheme that turox does not have.
15. Razoring, if at all. 7.9 +/- 7.4 over 4550 games, and the technique
    the wiki most directly warns against.

Not recommended: ProbCut, which overlaps null-move pruning's pruning
decisions and has a poor record in chess specifically.

## What would change this order

Three things, in rough order of likelihood.

**If killers and history do not move the EBF much.** The whole order
assumes the ordering deficit is in quiet moves. If killers and history
land and the EBF barely moves, the deficit is elsewhere (TT hit rate, the
always-replace scheme, or capture ordering), and SEE plus a
depth-preferred table should jump ahead of the entire reduction tranche.

**If reverse futility pruning underperforms its reported number.** It is
the cheapest item and it stands in for null-move pruning's premise. If
it fails to gain, the static evaluation is probably too noisy to prune
on, which would push evaluation work ahead of null-move pruning, futility
pruning, and razoring alike, since all three prune on a static score plus
a margin.

**If the node restructure in item 2 turns out to be large.** It is listed
as a prerequisite on the assumption it is a moderate refactor of one
function. If it grows into a redesign of how move generation is driven
(staged or lazy generation, which is where it naturally leads), it should
become its own decision ticket rather than a silent prerequisite, since
staged generation is a design commitment and not a pruning technique.

## Sources

All Chess Programming Wiki pages, read 2026-09-04:

- Principal Variation Search
- PVS and Aspiration
- Aspiration Windows
- Null Move Pruning
- Null Move Reductions
- Zugzwang
- Late Move Reductions
- Futility Pruning
- Reverse Futility Pruning
- Razoring
- Delta Pruning
- Quiescence Search
- Check Extensions
- Extensions
- Singular Extensions
- Multi-Cut
- ProbCut
- Internal Iterative Deepening
- Internal Iterative Reductions
- Killer Heuristic
- History Heuristic
- Static Exchange Evaluation
- Branching Factor
- Node Types
- Pruning
- Fail-Soft
- Search Instability
- Transposition Table
- Mate Distance Pruning
- Score

Measured per-feature Elo deltas: Blunder's published testing log,
`docs/testing.md` in `github.com/algerbrex/blunder`, read 2026-09-04.
These are self-play SPRT results of one engine against its own previous
version, in roughly the strength band turox is heading for. They rank
effort well and predict absolute gains poorly.
