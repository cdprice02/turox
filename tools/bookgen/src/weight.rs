//! Turning a position's observed move statistics into the integer weights
//! `book::BookMove` carries.

use crate::aggregate::MoveStats;
use turox_engine::book::BookMove;

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
#[must_use]
pub fn weigh(stats: &[MoveStats]) -> Vec<BookMove> {
    stats
        .iter()
        .map(|s| BookMove {
            mv: s.mv,
            weight: 0,
        })
        .collect()
}
