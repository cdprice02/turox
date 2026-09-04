# Evaluation terms beyond material, PST, and pawn structure

Research notes for the wayfinder question: what evaluation knowledge is
worth adding to turox now that material, tapered piece-square tables, and
doubled/isolated/passed pawns are in, what the literature says each term
is worth, what each one costs at every leaf, and in what order they
should land.

Sources are the Chess Programming Wiki pages named inline, read for this
note rather than recalled. Every quantitative claim from the literature
carries the page it came from. Every quantitative claim about turox
itself was measured on this branch and the command that produced it is
given.

## The evidence: turox has no idea its king is in danger

The wayfinder ticket points at one lichess game,
`turox-bot vs chesscorpus-org` ([WLULPprE], 300+3 blitz, turox as White,
lost by mate on move 39). The `%eval` comments in that PGN are turox's
own reported search scores, not an external analysis engine, which makes
the game a direct instrument reading rather than an opinion.

Reconstructing every position after a White move and running
`eval::eval_white_pov` on it gives the table below. `static` is turox's
evaluation of the position it just moved into. `material` is the same
position scored with `eval::PIECE_VALUES` alone. `positional` is the
difference, which is everything tapered PST and pawn structure
contribute together. `search` is what turox actually reported to lichess
after searching. `zone` counts Black pieces attacking the eight squares
around the White king, weighted by Stockfish's attack units as the
[King Safety] page gives them (minor 2, rook 3, queen 5).

| move | search | static | material | positional | zone units | White king |
| ---- | ------ | ------ | -------- | ---------- | ---------- | ---------- |
| 17. Bxd8 | +0.85 | +450 | +380 | +70 | 7 | g1, shield intact |
| 18. g3   | +0.80 | +440 | +380 | +60 | 4 | g1, g-pawn advanced |
| 21. Qc1  | +0.25 | +230 | +150 | +80 | 4 | g1 |
| 22. Kf1  | -0.75 | +243 | +150 | **+93** | 6 | f1, castling gone |
| 23. Ke1  | -0.75 | +175 | +50  | **+125** | 4 | e1, d-file open |
| 24. Kd1  | -1.15 | +125 | +50  | +75 | 2 | d1, on an open file |
| 26. fxg3 | -4.15 | +90  | +50  | +40 | 2 | d1 |
| 28. Nd4  | -8.90 | -26  | -50  | +24 | 7 | d1 |
| 29. Qd2  | -11.45| -354 | -380 | +26 | 9 | d1 |

Three things fall out of it, and none of them is the thing the score
graph makes it look like.

**The positional term peaked exactly when the king was being stripped.**
The whole positional contribution of turox's evaluation stayed inside
`+10` to `+125` centipawns for all 39 moves of the game. Its single
highest value, `+125`, is the position after `23. Ke1`, with the White
king driven off g1, castling rights gone, the d-file open onto it, and
two Black knights already inside the position. The static evaluation
liked that position more, positionally, than any other in the game.

**The king walk is nearly free under a midgame PST.** The midgame king
table in `eval::pst` scores g1 at `+30`, f1 at `+10`, e1 at `0`, d1 at
`0`. Marching the king from g1 to d1 through a shattered shelter costs
30 centipawns of PST and nothing else, because nothing else in the
evaluation looks at the king. Worse, `game_phase` reports 43 out of 256
over that stretch, so the taper has already begun mixing in the *endgame*
king table, which pays a bonus for leaving the corner. The one term that
knows anything about king placement is being interpolated toward the
table that actively rewards the move being punished.

**The score collapse is the search finding material, not the evaluation
finding danger.** Follow the `material` column instead of the `search`
column: `+380`, `+150`, `+50`, `-50`, `-380`. The reported score tracks
material with a lag of a few ply, which is exactly what a
material-and-PST evaluation inside an alpha-beta search produces. The
drop from `-0.85` to `-4.15` at move 26 is not the evaluation noticing an
attack. It is the horizon finally reaching the point where the attack
converts. An evaluation with a king-safety term would have been paying
for that attack since move 15, when the zone-unit count first went above
zero and stayed there.

That last point is the one that matters for prioritisation. The failure
is not that turox misjudged a hard position. It is that the position was
never in turox's evaluation vocabulary at all, so no amount of extra
search depth would have surfaced it any earlier than the material did.

[WLULPprE]: https://lichess.org/WLULPprE

## The cost budget, measured rather than assumed

The wayfinder framing is that turox's effective branching factor of 9.4
(see `move-ordering.md`) makes evaluation cost trade badly against depth,
so per-leaf cost should be weighed as heavily as Elo. That framing is
sharper than it first looks, and it is worth stating why before the
numbers.

Plain alpha-beta with good move ordering is supposed to deliver an
effective branching factor near the square root of the average, roughly
6. turox measures 9.4, and `search::negamax` currently has no killer
moves, no history heuristic, and no late move reductions. So turox is not
merely behind modern engines, it is behind what unaided alpha-beta
should already be giving it. Every node that a better-ordered search
would have pruned is a node that still pays the full evaluation cost, so
an expensive evaluation term is charged against a tree that is roughly
half again too wide at every ply. The same term added after killers,
history, and LMR land will cost meaningfully less in wall-clock terms for
the same depth.

That is a sequencing constraint, not a veto, and it applies unevenly:
terms costing tens of nanoseconds are unaffected by it, while terms that
double or triple evaluation are paying an inflated tree for the
privilege. The recommended order at the end of this note splits on
exactly that line.

With that said, the actual numbers on this codebase change which terms
the constraint rules out.

From `cargo bench -p turox-engine`, converted from criterion's throughput
figures to nanoseconds per position:

| operation | per position |
| --------- | ------------ |
| `eval::evaluate` (material + tapered PST + pawn structure) | 106 ns |
| `move_gen::king_moves` | 17 ns |
| `move_gen::knight_moves` | 32 ns |
| `move_gen::slider_moves` | 67 ns |
| `move_gen::pseudo_legal_moves` (one side, everything) | 250 ns |
| `move_gen::legal_moves` | 1745 ns |

The whole current evaluation costs about 6% of one `legal_moves` call.

Where that lands depends on which kind of node is being counted, and
`search::negamax`'s quiescence routine makes the split unusually sharp.
It evaluates first and only generates moves if the stand-pat score
failed to cut off:

- On a quiescence node that fails high at stand pat, evaluation is
  essentially the entire cost of the node. Doubling evaluation cost
  doubles those nodes.
- On a quiescence node that expands, evaluation is roughly one
  seventeenth of the node's cost, and a term that doubles evaluation
  costs about 6% of that node.

So the true budget depends on the stand-pat cutoff rate, which turox does
not currently instrument. That is worth measuring before committing to
the expensive terms, and the tree-shape baseline ticket is the natural
place for it. Absent that number, the safe reading is that terms costing
tens of nanoseconds are free and terms that double or triple evaluation
need an SPRT result rather than an argument, run after the search work
rather than before it.

Two levers exist for buying budget back, both already on the backlog.
Making material and PST incremental on `Board` removes most of the 106 ns
that exists today, since [Incremental Updates] names material as "only
affected by captures or promotions" and the piece-square sum as the other
standard incremental quantity. And a pawn hash table pays for every
pawn-only term at once: [Pawn Hash Table] reports hit rates "above 95% or
even 99% for most positions, specially if the pawn structure is settled
or relatively fixed after the opening," from a table needing only "a few
K" entries because "the pawn structure inside the search changes rarely
or transpose."

[Incremental Updates]: https://www.chessprogramming.org/Incremental_Updates
[Pawn Hash Table]: https://www.chessprogramming.org/Pawn_Hash_Table

## The architectural decision that shapes everything else

Most of the remaining terms want the same intermediate result: a set of
squares each side attacks, per piece type. [Mobility] describes safe
mobility as needing "maintained attack tables" and notes it "can be
computationally expensive unless incrementally updated." [King Safety]
describes square control near the enemy king as "often computed
incrementally via maintained attack tables during mobility calculation."
[Space] describes Stockfish's space term as counting "safe squares for
minor pieces," which is the same attack table again.

That is one shared pass, not four independent costs. Mobility, king-zone
attack units, threats, space, and outposts all read from it. The
practical consequence for ordering: the terms that need the attack pass
should land together once the pass exists, and the terms that need only
pawn bitboards and piece counts should land first, because they cost
nothing and do not depend on it.

turox is missing one small piece of plumbing for the attack pass.
`Board` stores `by_color: [Bitboard; 2]` but exposes only
`pieces(color, piece)`, so a caller wanting "all White pieces" has to OR
six bitboards. A `Board::color_occupancy(Color)` accessor is a one-line
prerequisite. `move_gen::attacks::piece_attacks` and `attacked_by`
already provide the rest.

[Mobility]: https://www.chessprogramming.org/Mobility
[Space]: https://www.chessprogramming.org/Space

## Term by term

### King safety

[King Safety] splits into sub-features that differ enormously in cost,
and the split is the whole recommendation: the cheap half addresses the
observed failure and the expensive half is a refinement of it.

**Pawn shelter and pawn storm** need only pawn bitboards and the king
square. The page's guidance is direct: "it is best to keep the pawns
unmoved or possibly moved up one square. The lack of a shielding pawn
deserves a penalty, even more so if there is an open file next to the
king." For storms it adds a calibration warning worth writing into the
code: "Penalties for storming enemy pawns must be lower than penalties
for (semi)open files, otherwise the pawn storm might backfire."

In the game above, the shelter term alone would have fired repeatedly.
`18. g3` moved a shield pawn two ranks off its home square and created
permanent holes on f3 and h3, which are the exact squares Black's knights
then used (`21...Nh3+`, `23...Nf3+`). Black's `12...h5` and `24...h4` are
a textbook pawn storm against a fianchetto-adjacent shelter. Neither
registered anywhere.

Cost: a handful of bitboard AND and popcount operations against masks
derived from the king's file, plus `file_fill` on both pawn sets for the
open-file test. Tens of nanoseconds at most, and the entire thing is
cacheable in a pawn hash table except for the king square dependency,
which is why [Pawn Hash Table] warns that "pawn-king stuff" requiring
"king squares in index calculations" is the one part that has to be
either keyed on the king square too or recomputed.

**Open and semi-open files toward the king** is the same computation the
rook term wants (below), evaluated against the king's file and its
neighbours instead of a rook's file, so the two should share a helper.

**King-zone attack units** is the expensive half. [King Safety] defines
the zone as "squares to which enemy King can move plus two or three
additional squares facing enemy position," which is `king_attacks(sq)`
plus a forward shift, both already available. Stockfish "counts each
minor piece attack on a king zone as 2 attack units, rook attack on king
zone as 3 attack units and a queen attack as 5 attack units," then scales
by the number of distinct attackers:

| attackers | weight |
| --------- | ------ |
| 1 | 0 |
| 2 | 50 |
| 3 | 75 |
| 4 | 88 |
| 5 | 94 |
| 6 | 97 |
| 7 | 99 |

with the final score being `valueOfAttacks * attackWeight[attackingPiecesCount] / 100`.
The single-attacker weight of zero is the load-bearing detail: one piece
pointed at a king is not an attack, and scoring it as one produces an
engine that shuffles a queen toward the enemy king for free centipawns.

Real engines replace the linear part with a nonlinear lookup. The page
gives Glaurung 1.2's `SafetyTable[100]`, which starts at `0, 0, 0, 1, 1,
2, 3, 4, 5, 6` for the first ten indices, passes 450 around index 60, and
saturates at 650. The shape is the point: near-zero for small attack
counts, steeply superlinear in the middle, capped so that an
overwhelming attack cannot outrun mate scores.

Applied to the game, the zone-unit column above shows Black holding 4 to
7 units from move 15 onward and reaching 9 by move 29. Under the CPW
scheme with three or more distinct attackers the weight is 75 or higher
rather than 0, so this is the range where the term stops being noise.

**Virtual mobility** is the page's cheapest proxy for the same idea:
place a queen on the king's square and count its moves, penalising a
large number. One magic lookup, and it captures diagonal and file
exposure without needing an enemy attack map at all. Worth considering as
an intermediate step between the two halves.

**Scaling** matters and is easy to forget: the page notes king safety
should scale with the opponent's remaining material, and warns about the
failure mode when it does not, namely that "whenever the engine finds
itself with a broken pawn shield, it tends to exchange pieces." turox
already has the machinery for this in `phase::game_phase`; king safety is
a midgame term and belongs almost entirely in the `mg` lane of
`phase::pack`.

[King Safety]: https://www.chessprogramming.org/King_Safety

### Mobility

[Mobility] gives the classical justification (Slater's study of 350
tournament games with even material showed "a definite correlation
between a player's mobility and the number of games won") along with
Turing's caution that maximising immediate mobility is not itself good
strategy.

The design choices the page lays out:

- Pseudo-legal counting is acceptable and common, including moves onto
  friendly pieces, since those "represent piece protection."
- Safe mobility, counting only squares where the piece is not en prise,
  is stronger but needs the attack tables.
- For knights specifically the page recommends a middle ground that
  "often works best": exclude only squares controlled by enemy pawns.
  That is one `front_attack_span`-style computation on the enemy pawn set
  and no attack table at all.
- Weights differ by piece and phase: "in the opening, the mobility of the
  bishops and knights is more important than that of the rooks." Forward
  mobility can be weighted above backward, and rook vertical mobility
  above horizontal.

The page is explicit that bitboard engines compute this "very quickly via
Population Count or weighted population count," which matches the
measurements above: the slider attack generation that dominates a
mobility pass is 67 ns for one side's sliders.

Cost estimate for turox: an attack pass over both sides' knights,
bishops, rooks, and queens, without the move-list writes that
`pseudo_legal_moves` pays for, lands somewhere between 150 and 300 ns.
That is 1.5x to 3x the current evaluation, and it is the single most
expensive item in this note. It also unlocks the king-zone half of king
safety, threats, and space for close to nothing extra, which is the only
reason it is worth paying.

Not incrementally updatable in any simple way. The attack sets of every
slider change whenever any piece moves onto or off their rays.

### Bishop pair

[Bishop Pair] reports Larry Kaufman's proposal of "the value of half a
pawn," with the caveat that the figure lives "within a broader system, in
which knights are stronger with many pawns on the board." [Point Value]
gives Kaufman's base values as `100, 325, 325, 500, 975` (1999) and
`100, 350, 350, 525, 1000` (2012), against turox's current
`100, 320, 330, 500, 900`. The wiki does not reproduce Kaufman's pawn
count adjustments or the rook and knight redundancy corrections; those
live in the external article it links.

Cost: two popcount comparisons, effectively zero. Two lines of code, no
new primitives, no dependency on the attack pass. This is the highest
value per line in the note.

The one subtlety worth a test: the bonus should be for having two
bishops on *opposite* colours, not merely two bishops, which after
underpromotion can differ. Checking against the light and dark square
masks costs one extra AND.

[Bishop Pair]: https://www.chessprogramming.org/Bishop_Pair
[Point Value]: https://www.chessprogramming.org/Point_Value

### Rook on an open or semi-open file

[Rook on Open File] defines an open file as one with no pawns of either
colour and a semi-open (or half-open) file as one with only enemy pawns,
both granting "greater vertical mobility as well as a chance to penetrate
the enemy camp."

Concrete values: bonuses for a rook on a fully open file "range from 8 to
20 centipawns," with a semi-open file typically about half that, so 4 to
10. The 20 upper bound comes from Toga Log's figure combining the bonus
with the penalty for a rook on a closed file. Common refinements the page
lists: an extra bonus for doubled rooks on the same open file (Rebel), a
larger bonus when the file points at the enemy king, and scaling by the
count of friendly pawns. It also notes early Fruit versions omitted the
term entirely, which is a useful calibration on how much it is worth.

Cost: one `file_fill` of each side's pawns, then two ANDs against the
rook bitboard and two popcounts. Under 10 ns, and `Bitboard::file_fill`
already exists and is already used by `pawn_structure::isolani`. The
open-file-toward-the-king half of king safety reads the same two fills.

Related terms on the same page, all cheap once the fills exist: rook on
the seventh rank, and connected or doubled rooks.

[Rook on Open File]: https://www.chessprogramming.org/Rook_on_Open_File

### Tempo

[Tempo] gives the rationale as avoiding "score oscillations on the parity
of the search depth," on the assumption that "it is usually advantageous
to be able to do something, except in zugzwang positions." It does not
name a centipawn value, describing it only as "a small bonus," and it
carries one caveat that matters: the bonus "is useful mainly in the
opening and middle game positions, but can be counterproductive in the
endgame."

For turox that caveat is free to honour, because `phase::pack` already
takes separate midgame and endgame values. A tempo bonus in the `mg` lane
and zero in the `eg` lane implements the page's advice exactly.

Cost: one addition in `evaluate`. The odd-even effect it addresses is
directly relevant, since the game above shows turox reporting depth 6 and
7 alternately in blitz, which is precisely the regime where parity
oscillation is visible.

[Tempo]: https://www.chessprogramming.org/Tempo

### Space

[Space] describes Stockfish's implementation concretely: "a space area
bonus by the number of safe squares for minor pieces on the central four
files on ranks 2 to 4, counting twice if on a rearspan of an own pawn,"
with the bonus "multiplied by a weight, determined by the number of own
pieces minus number of open files."

Two things about that definition are useful here. The rearspan condition
is `Bitboard::forward_fill` for the opposite colour, which turox already
has. And the "safe squares" condition is the attack pass again, so space
is a rider on mobility rather than an independent cost.

The page gives no Elo or centipawn figures, and notes Senpai 2.0 gets
most of the same effect through "glorified pawn chain piece-square
tables," which is a reasonable argument that space is partly subsumed by
work turox has already done. Low priority.

### Threats, connectivity, and trapped pieces

The wiki does not have a term page for threats in the Stockfish sense
(scoring enemy pieces attacked by less valuable friendly pieces).
[Connectivity] is the closest, and covers the defensive side: a term
"based on the graph theoretical relationship between the chess pieces and
the squares they control," which "encourages pawn chains, and discourages
loose pieces." The scoring scheme it quotes (single defender values of
`P=8.00, B=4.50, N=4.00, R=3.00, K=2.50, Q=2.0`, higher for multiple
defenders) is from one specific paper rather than standard practice, and
is not worth copying directly.

[Trapped Pieces] is more actionable, giving named patterns and penalties:
a bishop on h7 blocked by pawns on f7 and g6 penalised "by about 150"
centipawns, a bishop on h6 against g5 and f6 at "perhaps -50," and a rook
on h1, g1, h2, or g2 with the king on f1 or g1 at "perhaps -40," which
exists specifically to discourage the pseudo-castled shuffle.

That last pattern is worth flagging against the game above. turox's king
went to f1 and its rook was on e1, so this exact penalty does not apply,
but it is the same family of position and the same blind spot.

Cost of all three: threats and connectivity need the attack pass;
trapped-piece patterns are a small number of hardcoded mask tests and
cost nothing. All are refinements rather than gaps, and none of them is
the reason the game above was lost.

[Connectivity]: https://www.chessprogramming.org/Connectivity
[Trapped Pieces]: https://www.chessprogramming.org/Trapped_Pieces

### Outposts

[Outposts] defines an outpost as a "strong square in the center or
opponent half of the board, defended by own pawn and no longer attackable
by the opponent's pawn." Values given are modest: "10 centipawns" for a
knight on a central square per Toga Log, with bonuses "as large as 16
centipawns" possible, more if defended by two pawns or if the opponent
has no minor piece available to trade for the outpost piece.

The "no longer attackable by the opponent's pawn" condition is exactly
`front_attack_span` on the enemy pawn set, which
`pawn_structure::passed_bonus` already computes for the passed pawn test.
The defended condition is `pawn_attacks` on the friendly pawn set. So the
detection is genuinely cheap and reuses primitives that exist. Applies
mainly to knights, sometimes rooks on a wing.

[Outposts]: https://www.chessprogramming.org/Outposts

### King-pawn tropism

[King Pawn Tropism] is "an endgame evaluation feature concerning the
distance of a king to pawns, with the motivation to either defend or
support own ones, or to attack or block opponent ones." The metric is
Manhattan distance, specifically "the average Manhattan-distance of the
king square to all pawn squares," accumulated as a weighted sum over a
weight total, producing a value "in the 1..14 Manhattan range" used as a
penalty. The example weighting given is 6 for passed pawns, 3 for
backward pawns, 2 for the rest.

Cost: a loop over pawns with a distance lookup, so it depends on pawn
count, but a precomputed 64x64 Manhattan distance table makes each step a
single load. Cheap in absolute terms and, being endgame-only, it can sit
entirely in the `eg` lane where the `mg` weight is zero and the taper
already suppresses it in the midgame.

This is the term that most directly complements the endgame king PST
already in `pst::VISUAL_KING_PST_EG`, which knows about centralisation
but nothing about where the pawns are.

[King Pawn Tropism]: https://www.chessprogramming.org/King_Pawn_Tropism

### Endgame scaling and known draws

This is a category rather than a term, and the reason to take it
seriously is that its failures cost whole points rather than
centipawns.

[Draw Evaluation] gives the FIDE-derived immediate draws: both sides bare
kings, one side with a king and a single minor against a bare king, and
both sides with a king and a same-coloured bishop. Beyond those it lists
heuristic draws worth recognising: two knights against a bare king, a
minor each, the weaker side's minor against two knights, two bishops
against one, and two minors against one where the two are not a bishop
pair.

Two pieces of guidance from that page are worth carrying into the design
directly. First, on consistency: "if KBN vs KB is scored as a draw, the
same must be done with KBN vs KBP." A partial table of drawn material
configurations produces an engine that thinks trading into a drawn
position gains material, which is worse than no table at all. Second, on
implementation: rather than returning zero, the recommended approach is
"dividing scores by constants like 16 or 32 when the stronger side lacks
winning material combinations." A scale factor degrades gracefully where
a hard zero does not.

[Bishops of Opposite Colors] covers the largest single scaling case. Pure
opposite-coloured-bishop endings are "notorious difficult to win, as the
weaker side is likely to create a blockade on the squares controlled by
its own bishop," and "one pawn advantage is usually not enough to force
the win." The page's recommendation is to "scale down the material value
when the pure bishop of opposite colors ending is encountered," and it is
specific that the scaling should apply to the *pure* ending: "if some
more pieces beside the bishops are present on the board, winning the
endgame is easier."

[KBNK Endgame] is the one case that needs positive knowledge rather than
a scale factor. The mate "is delivered in the corner that can be covered
by a bishop of the attacking side," and the standard technique is "a
separate piece-square table for the position of the opponent king in
order to drive it to the correct corner," computed from the Manhattan
distance to the nearest corner of the bishop's colour. Without it a
material-and-PST engine cannot force the mate at all, because every
position in the winning process evaluates identically.

[Endgame] adds the general framing, including that the king evaluation
inverts in the endgame (already handled by the tapered king table) and
that "pawn promotion is a very important aim in most endgames" and should
be weighted heavily, which is an argument that the current
`PASSED_BONUS` of `pack(10, 20)` is probably too small at the endgame
end.

Cost of the whole category: piece counts, which `phase::game_phase`
already computes on every call, plus a small number of comparisons.
Near zero, and the natural place for it is a scale factor applied to
`phase::interpolate`'s result rather than a term summed into the
accumulator.

[Draw Evaluation]: https://www.chessprogramming.org/Draw_Evaluation
[Bishops of Opposite Colors]: https://www.chessprogramming.org/Bishops_of_Opposite_Colors
[KBNK Endgame]: https://www.chessprogramming.org/KBNK_Endgame
[Endgame]: https://www.chessprogramming.org/Endgame

## On Elo figures

The ticket asks for the typical Elo of each term, and the honest finding
is that the Chess Programming Wiki does not publish per-term Elo in a
form worth quoting. It gives centipawn magnitudes (rook on an open file
at 8 to 20, outposts at 10 to 16, trapped bishop at 150, bishop pair at
half a pawn), which are the weights a term should use, not what adding it
is worth in playing strength. Those are different questions, and the
second one is engine-specific: a term's Elo depends on what the rest of
the evaluation already covers and how deep the search goes.

turox is unusually well placed to answer it directly. `tools/selfplay/sprt.sh`
already exists, defaults to `elo0=0 elo1=10`, and runs over a 2494-position
opening suite. Every term in this note should get a run, and the terms
that cost nothing should still get one, because a cheap term with a
badly chosen weight can lose Elo just as easily as an expensive one.

For setting the weights themselves rather than accepting or rejecting a
term, [Texel's Tuning Method] is the standard approach: map the
quiescence score to a win probability through a logistic
`sigmoid(s) = 1 / (1 + e^(-K*s/400))`, then minimise the mean squared
error against actual game results over a large labelled position set.
The method's own description uses about 8.8 million positions from 64,000
fast games, with `K` fitted once and then held fixed. It tunes "several
hundreds of evaluation function parameters" simultaneously and handles
piece-square tables and positional bonuses well, but the page is clear
about its limit: it cannot learn what the engine fundamentally does not
understand, for instance endgame technique for endgames the engine never
reaches. That limit is the argument for adding the KBNK driving table by
hand rather than hoping a tuner finds it.

The sequencing implication is that tuning comes *after* the term set is
settled, not alongside it. Adding a term changes the optimum for every
other term.

[Texel's Tuning Method]: https://www.chessprogramming.org/Texel%27s_Tuning_Method

## Recommended order

Ranked by expected Elo per unit of effort and risk, with cost measured
against the 106 ns baseline above.

Steps 1 through 3 are cheap enough to land *before* the move-ordering and
pruning work, and should be: they cost tens of nanoseconds against a
106 ns baseline that is itself 6% of an expanded quiescence node, so the
inflated tree does not meaningfully tax them, and step 1 is the term the
game evidence demands. Steps 5 and 8 should wait until killers, history,
and LMR have brought the effective branching factor down toward 6, since
those are the terms whose cost is multiplied by every node a better
ordered search would have pruned. Steps 4, 6, and 7 are insensitive to
the ordering either way.

1. **King safety, pawn-only half.** Pawn shelter, pawn storm, and
   open/semi-open files toward the king. Cost is tens of nanoseconds and
   it is the term the game evidence actually demands. Reuses
   `file_fill`, `forward_fill`, and `king_attacks`. Weight it into the
   `mg` lane only.

2. **Endgame scale factors and known draws.** Insufficient material, the
   pure opposite-coloured-bishop scale-down, and the general "divide by
   16 or 32 when the stronger side cannot win" rule. Costs nothing
   beyond counts already taken by `game_phase`, and it converts lost
   half points rather than shaving centipawns. Applied as a scale on
   `interpolate`'s output.

3. **Bishop pair, rook on open/semi-open file, tempo.** Three
   independent terms, each a handful of bitboard operations, each with a
   published weight to start from. Land them separately so SPRT can
   attribute the result, but they are one afternoon's work between them.

4. **Incremental material and PST on `Board`.** Not a strength change,
   but it removes most of the current 106 ns and is what makes step 5
   affordable. Already tracked on the backlog.

5. **The shared attack pass, then mobility and king-zone attack units
   together.** The expensive step, 1.5x to 3x current evaluation cost,
   and the only one that genuinely needs the branching-factor argument
   weighed against it. **Wait for the search work.** Adding this while
   the effective branching factor is 9.4 pays the tripled per-leaf cost
   across a tree that is half again too wide, and an SPRT run now would
   measure the wrong engine. Needs `Board::color_occupancy` first. Do not
   land mobility alone: the pass is most of the cost and king safety's
   second half is most of the payoff.

6. **Outposts and king-pawn tropism.** Cheap, and both reuse spans that
   `pawn_structure` already computes. Endgame-weighted in the tropism
   case.

7. **KBNK driving table.** Small, self-contained, and the one thing a
   tuner provably cannot find on its own.

8. **Threats and space.** Riders on step 5's attack pass, so they
   inherit its wait, worth trying once it exists, low expected value
   relative to everything above.

9. **Automated weight tuning.** After the term set stops changing.

Everything from step 1 through step 3 is expected to be roughly free at
the leaf, which means the branching-factor concern that motivated the
cost analysis does not actually bind on the highest-priority work. It
binds on step 5, and only there. The convenient consequence is that the
evaluation work with the clearest evidence behind it and the search work
with the largest measured gap do not compete for the same slot: king
safety's cheap half can land now, and the attack pass can wait for the
tree to come under control without holding anything else up.

## What this note did not cover

Time-boxed research, so the following were left out and are worth a
follow-up if they become relevant.

- **Kaufman's material imbalance article itself.** [Point Value] links it
  externally but does not reproduce the pawn-count adjustments, the
  knight-with-many-pawns correction, or the rook redundancy penalty. The
  bishop pair figure quoted here is the wiki's summary of it, not the
  source.
- **Stockfish's actual evaluation source.** The attack-unit values and
  the space definition here are the wiki's descriptions of Stockfish, not
  read from `evaluate.cpp`. Anyone implementing the king-zone term should
  read the real thing before copying constants.
- **The stand-pat cutoff rate in turox's quiescence search.** The single
  number that would turn the cost budget above from a range into an
  answer. Not currently instrumented.
- **Backward pawns, connected pawns, pawn islands, hanging pawns,
  candidate passers, and holes.** [Pawn Structure] enumerates all of
  them; only doubled, isolated, and passed are implemented. They are
  extensions of a term turox already has rather than new knowledge, and
  they belong with a pawn hash table rather than ahead of king safety.
- **Blockage detection, fortresses, and the wrong rook pawn.** Named on
  the [Endgame] and [Draw Evaluation] pages, not read in detail.
- **Lazy evaluation.** Read, and it is the standard mitigation for
  expensive terms: split evaluation into stages and return early when the
  cheap stage is already far outside the alpha-beta window. It is
  premature here, since the cheap terms recommended first do not need it,
  and the page carries a specific warning about lazy evaluation
  interacting badly with sparse-material endgames such as KBNK, which is
  exactly the knowledge step 7 adds.
- **NNUE and learned evaluation.** A different answer to the same
  question, tracked separately, and out of scope for a note about
  hand-written terms.

[Pawn Structure]: https://www.chessprogramming.org/Pawn_Structure
