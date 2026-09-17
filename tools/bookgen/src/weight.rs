//! Turning a position's observed move statistics into the integer weights
//! `book::BookMove` carries.

use crate::aggregate::MoveStats;
use turox_engine::book::BookMove;

/// How much a move's classical points-per-game score (win = 1, draw = 0.5,
/// loss = 0) contributes to its weight, relative to [`TIMES_PLAYED_WEIGHT`].
const WIN_PROBABILITY_WEIGHT: u64 = 1600;

/// How much raw sample size contributes to a move's weight, on its own,
/// independent of how well it scored.
const TIMES_PLAYED_WEIGHT: u64 = 2;

/// Weighs `stats`, one position's candidate moves, into `BookMove`s.
///
/// The exact formula is a tuning knob, not a fixed rule (see
/// `aggregate::BuildOptions`'s own doc on why these are found by measuring
/// output rather than decided up front), but two properties any formula
/// here should hold, since they're what "weighted by frequency and win
/// rate" actually means: holding win rate fixed, more games played must
/// weigh strictly more; holding times played fixed, a higher win rate must
/// weigh strictly more. A move with zero recorded games has no weight to
/// give it, but nothing in [`aggregate`](crate::aggregate::aggregate)
/// should ever produce one of those in the first place.
///
/// A draw counts as exactly half a win, the classical chess-scoring
/// convention, rather than its own independently-tuned term: `2 * wins +
/// draws` over `2 * times_played` is that points-per-game score, kept in
/// integer arithmetic by doubling both sides instead of working in floats.
/// Every intermediate product is computed in `u64` and only narrowed back
/// to `u32` (clamped, not wrapped) at the very end, since `wins *
/// WIN_PROBABILITY_WEIGHT` alone can exceed `u32::MAX` once a move has
/// been played a few million times, well within reach of a real
/// multi-year Lichess Elite Database run for a common first move.
#[must_use]
pub fn weigh(stats: &[MoveStats]) -> Vec<BookMove> {
    stats
        .iter()
        .map(|s| {
            let times_played = u64::from(s.times_played);
            let wins = u64::from(s.wins);
            let draws = u64::from(s.draws);

            let score_weight = (2 * wins + draws) * WIN_PROBABILITY_WEIGHT / (2 * times_played);
            let weight = times_played * TIMES_PLAYED_WEIGHT + score_weight;

            BookMove {
                mv: s.mv,
                weight: u32::try_from(weight).unwrap_or(u32::MAX),
            }
        })
        .collect()
}
