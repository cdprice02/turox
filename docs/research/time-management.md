# Time management, and the bullet-versus-blitz gap

Two questions, answered in that order of priority:

1. Why did turox look far worse at bullet (60+1) than at blitz (300+3)?
2. What does the literature say a UCI engine should do about the clock,
   and is turox's current policy defensible at 60+1?

## Summary

**There is no bullet-versus-blitz strength gap to explain.** The two
bullet rating diffs (-428 and -83) are Glicko-2 provisional-rating
deflation, not evidence of bad play. turox's *blitz* pool opened with a
-519 diff from the same 3000 provisional seed, a larger single-game drop
than either bullet game. After two games the bullet rating sat at 2489;
after two games the blitz rating sat at 2433. The pools agree. Blitz only
looks calm because it has nine games of history and bullet has two.

**turox does not mismanage its clock at 60+1.** It never flagged, never
came close, allocated a correctly scaled fraction of the remaining time
on every move, and reached a median search depth within one ply of what
it reaches at 300+3. Clock utilisation at bullet (84% and 68% of total
available time) sits inside the range measured at blitz (68% to 97%).

**One real defect did surface, and it is time-control neutral.**
`ITERATION_TIME_SAFETY_MARGIN = 4` is calibrated well below turox's
measured effective branching factor of 9.4, so the iterative-deepening
soft limit almost never fires. turox therefore burns its entire budget on
nearly every move and discards an unfinished final iteration, rather than
stopping early and banking the remainder. This costs the same at 300+3 as
at 60+1, so it is not the answer to question 1, but it is where the
genuine headroom is.

---

## Part 1: the diagnosis

### Method

Parsed the `[%clk ...]` and `[%eval score,depth]` comments out of the ten
lichess game records in `~/repos/lichess-bot/game_records/`. Under a
Fischer control, lichess reports the clock *after* the increment is
applied, so time actually consumed on move *n* is
`clock(n-1) + increment - clock(n)`. The `[%eval]` depth is turox's own
reported depth for the move it played, so the two series together give
budget spent and depth bought, per move, per time control.

The corpus: two 60+1 games against `styx_reckless`, eight 300+3 games
against `chesscorpus-org`, plus one 180+1 and one 600+0 game excluded
from the pooled comparison because each is a single game against a
different opponent.

### Finding 1: turox never flagged, and was never close

Both bullet games carry `[Termination "Normal"]`, not `Time forfeit`.
turox lost on the board in both.

| Game | Result | turox moves | Total spent | Clock at final move |
| --- | --- | --- | --- | --- |
| 8dxafOGG (Black) | 1-0 | 35 | 80s | **15s** |
| y3Egpp50 (White) | 0-1 | 22 | 56s | **26s** |

Finishing a 60-second bullet game with 26 seconds unspent is the opposite
of a clock-management failure. Whatever lost these games happened on the
board.

### Finding 2: the allocator scales correctly across time controls

`allocate_time` computes `time_left / 30 + increment`, clamped, then
reduced by a 100ms overhead reserve. Predicted opening allocation:

- 300+3: `300/30 + 3 - 0.1` = **12.9s**. Observed on the first real move
  of every 300+3 game: 13s to 14s.
- 60+1: `60/30 + 1 - 0.1` = **2.9s**. Observed on the first real move of
  both bullet games: 3s.

The formula is confirmed against live play at both time controls, and the
fraction-of-remaining shape means the allocation decays smoothly as the
clock drains rather than holding constant. Measured medians: 11s per move
over blitz moves 1 to 15, falling to 9s over moves 16 to 25; 3s per move
over bullet moves 1 to 15, falling to 2s over moves 16 to 25. That is a
5x scaling between the two controls, matching the 5x difference in base
time. The "near-constant time regardless of remaining clock" failure mode
does not occur.

### Finding 3: bullet costs turox about one ply, not four hundred Elo

Pooling all turox moves by time control and move number:

| Moves | 300+3 median spend | 300+3 median depth | 60+1 median spend | 60+1 median depth |
| --- | --- | --- | --- | --- |
| 1 to 15 | 11s | 6 | 3s | **6** |
| 16 to 25 | 9s | 7 | 2s | **6** |

In the opening and early middlegame turox reaches *the same median depth*
at bullet as at blitz. By moves 16 to 25 it is one ply behind. That is
exactly what the arithmetic predicts: a 4x to 5x cut in thinking time at
an effective branching factor of 9.4 costs slightly under one iteration.

One ply at this strength is worth perhaps 50 to 70 Elo. It is not worth
400.

### Finding 4: the rating arithmetic accounts for the entire "gap"

Ordering every game by its UTC timestamp and reading turox's own rating
and diff:

| Time | TC | turox rating | Diff | Opponent |
| --- | --- | --- | --- | --- |
| 00:24 | 180+1 | 3000 | **-519** | NeraChess (2729) |
| 00:38 | 300+3 | 2481 | -48 | chesscorpus-org |
| 00:45 | 300+3 | 2433 | -31 | chesscorpus-org |
| 00:52 | 300+3 | 2402 | -22 | chesscorpus-org |
| 01:00 | 300+3 | 2380 | -19 | chesscorpus-org |
| 01:08 | 300+3 | 2361 | -15 | chesscorpus-org |
| 01:15 | 300+3 | 2346 | -14 | chesscorpus-org |
| 01:24 | 300+3 | 2332 | -11 | chesscorpus-org |
| 01:31 | 300+3 | 2321 | -11 | chesscorpus-org |
| 23:36 | 60+1 | 3000 | **-428** | styx_reckless (2792) |
| 23:47 | 60+1 | 2572 | -83 | styx_reckless (2796) |

Lichess rates bullet and blitz in separate pools, each seeded at the
provisional 3000 with a large rating deviation. Glicko-2 moves a
high-deviation rating enormously on the first result and progressively
less as the deviation shrinks, which is precisely the decay visible in
the blitz column: -519, then -48, -31, -22, -19, -15, -14, -11, -11.

The bullet column is the same curve, observed two games in: -428, then
-83. The blitz pool's first game produced a *larger* drop than the bullet
pool's did. Comparing a settled nine-game blitz rating against a
two-game bullet rating and reading the difference as a strength gap is a
category error; the diffs measure rating uncertainty collapsing, not
playing strength.

Both pools land in the same place. Blitz after two games: 2433. Bullet
after two games: 2489.

### Verdict

The bullet gap is a measurement artifact. The correct next step is to
play twenty or thirty more bullet games and let the rating converge
before treating any residual difference as real. On the current evidence
the expected residual is small, roughly the one ply of depth that a 5x
shorter clock buys, and it is not a time-management bug.

---

## Part 2: is turox's policy defensible at 60+1?

Yes, with one tuning constant worth revisiting. Reviewing
`turox-engine/src/search/time.rs`, `turox-engine/src/uci/session.rs`, and
the iterative-deepening driver in `turox-engine/src/search/negamax.rs`.

### The allocator is conservative but sound

`time_left / 30 + increment` is a slightly tighter variant of the
baseline CPW quotes, `base / 20 + increment / 2`
([Time Management](https://www.chessprogramming.org/Time_Management)).
turox divides by more (30 rather than 20) and claims the whole increment
rather than half of it. The divisor also sits inside the range CPW gives
for sudden-death estimation, where "programs estimate the game will last
further 25..40 moves, and divide the remaining time by this number"
(same page); `DEFAULT_MOVES_TO_GO = 30` is the midpoint of that range.

Claiming the full increment is safe under a Fischer control because the
increment is credited unconditionally, and it is the term that matters
most at 60+1, where it is a third of the budget rather than a quarter.

The important structural property is that the allocation is a *fraction
of what remains* rather than a fixed quantity. That makes flagging almost
impossible: each move can only ever consume a thirtieth of the clock plus
the increment, so the clock decays geometrically toward the increment
rather than hitting zero. The 60+1 games confirm this empirically, and it
is why turox finished them with 15s and 26s in hand.

### The soft limit is calibrated to the wrong branching factor

This is the one genuine finding in the code.

CPW's version of this rule is a comparison against the *allocation*:
stop before starting a new iteration when "the relation of elapsed and
allocated time (f.i. > 50%)" is exceeded
([Time Management](https://www.chessprogramming.org/Time_Management)).
turox's rule is better shaped, because it predicts the next iteration's
cost from the last one's rather than using a fixed fraction:

```rust
if elapsed.saturating_mul(ITERATION_TIME_SAFETY_MARGIN) > remaining {
    break;
}
```

with `ITERATION_TIME_SAFETY_MARGIN = 4`. Its doc comment reasons that
real branching factors run "roughly 7-9x between iterations" and that 4
is deliberately below that, biased toward under-triggering because
throwing away reachable depth is a strength cost while a wasted iteration
is only a time cost.

The measured effective branching factor is 9.4. So the rule starts an
iteration whenever more than `4 x elapsed` remains, while the iteration
actually needs about `9.4 x elapsed`. Every time the remaining budget
falls in that band, turox commits to an iteration it cannot finish and
runs to the hard deadline instead.

The clock data shows this is not a corner case but the normal path.
turox consumes essentially its exact computed budget on nearly every
move at both time controls: 3s against a 2.9s budget at 60+1, 13s to 14s
against a 12.9s budget at 300+3. A soft limit that fired would leave a
visible shortfall, and there is none. The soft limit is effectively
inert.

The consequence is not an overrun (the hard bound, polled every 2048th
node, holds), and not a wrong move (the partial iteration is correctly
discarded in favour of the last completed one). The consequence is that
the tail of every move's budget produces no completed depth and is not
banked either. Because the allocator is a fraction of what remains,
banked time compounds: a second saved at bullet is a second that raises
every subsequent allocation.

Whether raising the constant toward 9 is actually a strength gain is a
question for the self-play SPRT harness rather than for reasoning, since
an aborted iteration is not pure waste. It still populates the
transposition table and improves move ordering for the next search. But
the constant is currently justified by an estimate (7 to 9) that the
measured value (9.4) sits above, and the gap between 4 and 9.4 is wide
enough that the rule does nothing at all. That is worth an experiment.

### The overhead reserve has the right shape

`OVERHEAD_RESERVE` is a flat 100ms rather than a fraction, on the
reasoning that dispatch and network latency do not scale with the time
control. That reasoning is correct, and the constant is small enough to
be harmless at bullet: 100ms is 3.4% of a 2.9s bullet budget versus 0.8%
of a 12.9s blitz budget. The relative bite triples going to bullet but
remains negligible in absolute terms.

CPW's Time Management page has nothing to say about lag reserves; the
practice is real but its magnitudes come from engine sources rather than
from the wiki.

### What `go` does and does not consume

`turox-engine/src/uci/session.rs` reads `infinite`, `movetime`, then
`wtime`/`btime` and `winc`/`binc` for the side to move, passing
`movestogo` straight through to `allocate_time`. The deadline is computed
once when `go` is parsed and never revised. There is no pondering, no
panic extension, and no stability-based adjustment; the budget set at
`go` is the budget for the whole search.

That is a defensible starting point and it is not what lost the bullet
games, but it does mean every technique in Part 3 is currently absent.

---

## Part 3: the techniques turox does not yet have

Sourced from
[chessprogramming.org/Time_Management](https://www.chessprogramming.org/Time_Management)
unless noted. Coverage gaps are flagged honestly: the wiki is written at
a classical-time-control altitude and is a good source for architecture
and a poor one for tuning constants.

### Soft bound and hard bound

CPW's central structural recommendation is two thresholds rather than
one: an *optimum* time (soft bound) checked once per iterative-deepening
iteration, and a *maximum* time (hard bound) polled periodically inside
the search itself. The soft bound is a scheduling decision and is allowed
to move in response to search feedback; the hard bound is a safety limit
and is not negotiable.

turox already has both, structurally: `ITERATION_TIME_SAFETY_MARGIN` is
the soft bound and the every-2048-nodes deadline check is the hard bound.
What it lacks is any mechanism for *moving* the soft bound, which is what
the next three techniques are for.

### Move stability

CPW lists as a decision input: "How often did the best move change during
the (last N) previous iterations?" alongside the score trend across
iterations and the "ratio of subtree size under best move versus entire
search tree". A root move that has not changed for several iterations,
or whose subtree dominates the search, signals confidence and licenses
moving early.

CPW states the signal but gives no formula, coefficient, or multiplier
for converting it into a time scaling factor. The well-known
instantiations live in engine source, not on the wiki.

This is the technique most likely to help turox at bullet specifically,
because it converts confidence directly into banked time, and banked time
compounds under a fraction-of-remaining allocator.

### Panic time on a root fail-low

CPW: "Fail low situations, a severe drop of the score may cause programs
to allocate 'panic time' to hopefully solve the critical situation".

Two qualifications from the same page. The trigger is a severe *score
drop*, not merely a fail-low event that resolves to an acceptable score.
And CPW classifies panic time as largely obsolete, grouped with the
opening-book extra-time bonus, on the reasoning that continuous
move-stability feedback subsumes the discrete panic special case.

No magnitude is quoted. Any specific figure for how much extra time panic
mode grants is not attributable to CPW.

The practical read for turox: implement stability first. Panic time is
the older, coarser answer to the same problem.

### Node-based time management

Not verifiable from CPW. The page
`Node_Count_Based_Time_Management` returns HTTP 404 and does not exist on
the wiki; CPW's search was also unavailable, so the concept could not be
located under an alternate title.

The adjacent material CPW does carry is the "ratio of subtree size under
best move versus entire search tree" heuristic, which is node-derived but
is a confidence signal for stopping early rather than node-based time
management proper.

The usual meaning of the term (replacing wall-clock accounting with node
accounting so that test games are reproducible and hardware-independent)
needs the Stockfish repository and its `nodestime` UCI option as its
citation, not CPW. Recorded here as unverified.

Worth noting that turox already has the mechanism this would need:
`Search` supports a `max_nodes` bound described in its own docs as "a
deterministic alternative to `deadline`". Wiring that to a node-budget
time policy would be a small change, and the payoff is reproducible SPRT
runs rather than playing strength.

### Move overhead reserves

Absent from CPW's Time Management page, which carries no discussion of
GUI lag, communication delay, or reserve magnitudes. The page links to
`Chess Engine Communication Protocol` and `UCI`, so the material may live
there, but no CPW-quotable number exists for it.

turox's flat 100ms is therefore unvalidated against the literature but
sound in shape, per Part 2.

### Very short time controls

Also largely absent from CPW, and this is a real hole in the wiki's
coverage rather than in this survey. Nothing on the page addresses
bullet-specific behaviour, fixed-depth fallbacks, or a minimum time per
move. The nearest relevant items are structural: the hard bound is the
mechanism that prevents forfeit and therefore carries the load at fast
controls, and "only one legal move" is the single unconditional early-out
CPW names.

turox's `MIN_BUDGET` of 30ms is the floor CPW does not discuss. It only
binds when `time_left` is already tiny, and the final clamp keeps it at
or under `time_left`, so it cannot itself cause a forfeit.

### Pondering

CPW defines pondering as "using the opponent's move time to consider
likely opponent moves and thus gain a pre-processing advantage when it is
our turn to move"
([Pondering](https://www.chessprogramming.org/Pondering)). The one
concrete figure on the page: "According to Robert Hyatt, in about 50% the
prediction is right".

On a ponder hit the engine has a choice, per the same page: "either
continue searching with the saved time, or dependent on score and time
left, move immediately." That choice is itself a time management decision
and CPW does not specify the arithmetic. No Elo figure for pondering is
quoted.

turox does not ponder. Given a ~50% hit rate, pondering is roughly a 1.5x
effective time multiplier, which is a larger win than any constant
retuning in Part 2. It is also considerably more work, since it requires
handling `go ponder`, `ponderhit`, and search restart on a miss.

---

## Where the headroom actually is

Ordered by expected value per unit of effort, given the diagnosis:

1. **Play more bullet games.** The gap that prompted this investigation
   is a two-game sample against a provisional rating. This costs nothing
   and is the only way to find out whether any residual gap exists.
2. **Retune `ITERATION_TIME_SAFETY_MARGIN` against the measured 9.4
   branching factor.** The constant is currently inert. An SPRT run at 4
   versus 9 answers it directly.
3. **Move-stability-based early exit.** Converts search confidence into
   banked time, and banked time compounds under a fraction-of-remaining
   allocator. The highest-value technique turox is missing.
4. **Node-based time management for the test harness.** Not a strength
   gain, but it makes every subsequent SPRT result reproducible, which
   makes items 2 and 3 measurable.
5. **Pondering.** The largest single win available (~1.5x effective
   time) and by some distance the most work.

Panic time is explicitly deprioritised: CPW itself calls it obsolete, and
it addresses a problem that move stability handles better.

---

## Sources and their limits

Primary sources read for Part 3:

- [Time Management](https://www.chessprogramming.org/Time_Management)
- [Iterative Deepening](https://www.chessprogramming.org/Iterative_Deepening),
  which confirms the fallback property ("In case of an unfinished search,
  the program always has the option to fall back to the move selected in
  the last iteration of the search") but contains no fractions,
  thresholds, or formulas for time-based stopping
- [Pondering](https://www.chessprogramming.org/Pondering)

Pages that do not exist: `Node_Count_Based_Time_Management` and
`Time_Control` both return 404.

Two caveats on the literature. CPW pages were retrieved through a
summarising fetch, so quoted text is reported as verbatim but has not
been eyeballed against the rendered page; anything load-bearing should be
confirmed before it is relied on. And CPW quotes no Crafty or Stockfish
source excerpts for any of this, so every tuning constant a modern engine
actually uses for stability scaling, panic extension, and move overhead
needs engine source as its citation.

CPW's own bibliography is the trail to the real literature. The entries
that map most directly onto the questions here are Hyatt's *Using Time
Wisely* (ICCA Journal, 1984), Donninger's *A la Recherche du Temps Perdu:
'That was easy'* (ICCA Journal, 1994) on easy-move detection, and
Vučković and Šolak's *Time Management Procedure in Computer Chess* (Facta
Universitatis, 2009), the only entry that looks like a systematic
treatment of the allocation procedure itself.

The clock and depth figures in Part 1 are derived from the PGN records in
`~/repos/lichess-bot/game_records/`, which are outside this repository.
