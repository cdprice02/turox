//! The window one iteration of iterative deepening opens, and how it reopens
//! when the answer falls outside it.
//!
//! Sits beside `time` rather than under `selectivity`: both decide something
//! for the driver once per iteration, where selectivity decides per move and
//! keeps nothing. This does keep something, namely how far the window has
//! already been reopened and how many times, which is what bounds the work a
//! single iteration can be made to repeat.

use super::{is_mate_score, MATE};
use crate::eval::Score;

/// The shallowest iteration that opens a narrow window. Everything below this
/// searches the full one.
///
/// A floor exists because the iterations whose score moves most are the ones
/// worth least. Measured across four positions, the change between consecutive
/// iterations up to here reaches 54 centipawns and is 20 or more in seven of
/// twelve transitions, while from here on the median is 3 and the largest is 26.
/// Part of that is parity: a shallow search alternates with whichever side moved
/// last, and the start position reads 40, 10, 40, 10 through the fourth
/// iteration. None of it is worth avoiding, because the volatile iterations are
/// also nearly free: the fourth iteration from the start position is 3,530
/// nodes, about a thousandth of the eleventh. Narrowing there pays a re-search
/// to save a tree that was not costing anything.
///
/// Not tunable by counting nodes to a fixed depth, which is the obvious thing to
/// try. Sweeping this against total nodes to depth 9 produces a clean curve with
/// an optimum three plies below that depth, and the same setting measured to
/// depth 11 is worse than leaving all but the last two iterations alone. The
/// curve is really about how many of the final iterations get narrowed, so it
/// moves with whatever depth the sweep stops at, and no constant can satisfy
/// both. The metric is biased too: reaching a given depth charges the last
/// iteration's re-search in full while only partly crediting the cheaper
/// iterations that got there, and a real search stops mid-iteration on the
/// clock instead. Tune this in the self-play harness, where the clock is in the
/// loop.
pub const MIN_DEPTH: u8 = 5;

/// Half the width a window opens at: the previous score plus and minus this.
///
/// Sized so an escape is uncommon without the window being so wide it prunes
/// nothing. On a scan of nineteen opening positions, roughly one deep iteration
/// in ten moves 30 centipawns or more, so a re-search is the exception rather
/// than the rule, while a window 60 wide is still some three orders of magnitude
/// tighter than the mate-to-mate span it replaces.
///
/// A starting point for a self-play match rather than a tuned result, and the
/// samples behind it are small: seventeen deep transitions on four positions,
/// plus the wider scan above. The same holds for [`MIN_DEPTH`] and
/// [`MAX_ESCAPES`].
pub const DELTA: Score = 30;

/// How many escapes an iteration may pay for before it gives up and opens the
/// full window.
///
/// Doubling alone would reach the full window eventually, but it would take
/// around ten attempts to get from a 30-point half-width to a mate-to-mate one,
/// and every attempt is a re-search of the whole iteration. A cap trades the
/// last few doublings for one certainly-sufficient attempt.
///
/// Counted across both directions rather than per side. A fail-low followed by
/// a fail-high is reachable, because the bound a re-search reports is not
/// guaranteed to agree with the one before it once a transposition table and
/// reductions are involved, and a per-side count would let the two directions
/// alternate without either ever reaching its limit.
pub const MAX_ESCAPES: u8 = 3;

/// How much an iteration paid for its window being narrow.
///
/// Reported on its own `info string` line rather than folded into the cutoff
/// histogram: these count iterations and attempts, where that counts nodes, and
/// an escape is not a cutoff even when the loop stops the same way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Calls into the root, counting every attempt rather than every iteration,
    /// so this exceeds the depth reached exactly when a window was escaped.
    pub attempts: u32,
    /// Results that came back at or below their window's floor.
    pub fail_low: u32,
    /// Results that came back at or above their window's ceiling.
    pub fail_high: u32,
    /// Nodes spent on attempts that were then thrown away.
    ///
    /// The number that says whether the half-width is too tight, and the reason
    /// a bare escape count is not enough: an escape at depth 6 and one at depth
    /// 11 cost orders of magnitude apart, so counting them alike would hide
    /// which one is worth tuning for.
    pub wasted_nodes: u64,
}

/// One iteration's window, plus what it needs to remember to keep reopening
/// bounded.
///
/// Neither bound ever moves inward. One moves per escape and the other stays, so
/// each attempt searches a window containing the last and no attempt can be
/// trapped by a bound the previous one left behind. That is what makes leaving
/// the side that held in place safe: a schedule moving a bound inward would lose
/// it, and a ceiling left one point above a raised floor is a window nothing can
/// be answered in.
pub struct Window {
    /// The floor this attempt searches with.
    alpha: Score,
    /// The ceiling this attempt searches with.
    beta: Score,
    /// How far the next reopening moves a bound, doubling each time. Zero on a
    /// window that opened full, which cannot be escaped and so never reads it.
    delta: Score,
    /// How many results have already escaped this iteration's window, counted
    /// across both directions for the reason [`MAX_ESCAPES`] gives.
    escapes: u8,
}

impl Window {
    /// The window an iteration at `depth` opens, given the score the last
    /// completed iteration reported.
    ///
    /// Opens the full window whenever narrowing cannot pay: below
    /// [`MIN_DEPTH`], before any iteration has completed, and around a mate
    /// score, which sits so far out that any narrow window is escaped every
    /// time and reopened for nothing.
    ///
    /// Both bounds are clamped into the range a score can actually take. A
    /// window reaching past [`MATE`] claims a range no search can return, and
    /// clamping is also what makes the full window the limit of reopening
    /// rather than a case handled separately.
    pub fn new(previous: Option<Score>, depth: u8) -> Self {
        let narrow = previous
            .filter(|_| depth >= MIN_DEPTH)
            .filter(|score| !is_mate_score(*score));
        narrow.map_or(
            Self {
                alpha: -MATE,
                beta: MATE,
                delta: 0,
                escapes: 0,
            },
            |score| Self {
                alpha: score.saturating_sub(DELTA).max(-MATE),
                beta: score.saturating_add(DELTA).min(MATE),
                delta: DELTA,
                escapes: 0,
            },
        )
    }

    /// The bounds to search this attempt with.
    ///
    /// The single place a window leaves this type, which is why the width
    /// invariant is asserted here rather than at each mutation: a window one
    /// point wide can prove a bound but never return a score, and it stops
    /// being a principal-variation node, so reaching one is a bug in the
    /// schedule rather than a state to handle.
    pub fn bounds(&self) -> (Score, Score) {
        debug_assert!(
            self.beta.saturating_sub(self.alpha) > 1,
            "a window must be wider than a null window to return a score: ({}, {})",
            self.alpha,
            self.beta
        );
        (self.alpha, self.beta)
    }

    /// Reopen after a result came back at or below the floor, where `bound` is
    /// an upper bound on the true score.
    ///
    /// The floor drops by the current half-width, or further if `bound` says the
    /// score collapsed past that, which reaches a distant score in one attempt
    /// instead of several. The ceiling holds, because a score at or below the
    /// floor is already under it, so keeping it rules out nothing the next
    /// attempt could return.
    ///
    /// Holding it rather than reopening both sides is a choice about not
    /// discarding what the search proved, and not a measured win: reopening
    /// both was worth 532 nodes out of 3.6 million on a position that escapes
    /// four times in a depth-9 search, and nothing at all on one that escapes
    /// once. Worth knowing before spending effort here, since the schedule is
    /// reached rarely enough that neither version is distinguishable outside a
    /// position picked to escape.
    ///
    /// Once [`MAX_ESCAPES`] is spent the window opens fully instead, which
    /// cannot be escaped and ends the iteration's attempts.
    ///
    /// The half-width is shared with [`Self::failed_high`], so an escape one way
    /// followed by one the other moves the second bound by the width the first
    /// had already reached rather than starting over. Repeated escapes are
    /// themselves evidence the window is badly placed, so widening harder after
    /// one is wanted, and [`MAX_ESCAPES`] bounds how far it goes.
    pub fn failed_low(&mut self, bound: Score) {
        self.escapes = self.escapes.saturating_add(1);
        if self.escapes >= MAX_ESCAPES {
            self.alpha = -MATE;
            self.beta = MATE;
        } else {
            self.alpha = self
                .alpha
                .saturating_sub(self.delta)
                .max(-MATE)
                .min(bound.saturating_sub(1));
            self.delta = self.delta.saturating_mul(2);
        }
    }

    /// Reopen after a result came back at or above the ceiling, where `bound` is
    /// a lower bound on the true score. The mirror of [`Self::failed_low`]: the
    /// ceiling rises out past `bound` and the floor holds, for the same reasons
    /// in the other direction.
    pub fn failed_high(&mut self, bound: Score) {
        self.escapes = self.escapes.saturating_add(1);
        if self.escapes >= MAX_ESCAPES {
            self.alpha = -MATE;
            self.beta = MATE;
        } else {
            self.beta = self
                .beta
                .saturating_add(self.delta)
                .min(MATE)
                .max(bound.saturating_add(1));
            self.delta = self.delta.saturating_mul(2);
        }
    }
}
