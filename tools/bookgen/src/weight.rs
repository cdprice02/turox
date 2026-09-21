//! Turning a position's observed move statistics into the integer weights
//! `book::BookMove` carries.

use crate::aggregate::MoveStats;
use turox_engine::book::BookMove;

/// Weighs `stats`, one position's candidate moves, into `BookMove`s.
///
/// `weight = times_played * (0.5 + score)`, where `score` is the classical
/// points-per-game chess score (win = 1, draw = 0.5, loss = 0). Frequency
/// and win rate multiply rather than add: `score` always stays in a fixed
/// `0.5..=1.5` band regardless of how large `times_played` gets, so a
/// move's quality can never be swamped by (or swamp) its own sample size
/// the way it would if the two were added with separately-tuned
/// coefficients instead. That's what makes this formula scale-invariant:
/// "more games at the same rate weighs more" and "a better rate at the
/// same sample size weighs more" both hold whether a position has been
/// seen 10 times or 10 million times, with no constant to retune between
/// those two cases.
///
/// The algebra collapses to pure integers with nothing left to tune:
/// `times_played * score` is exactly `wins + 0.5 * draws` (a draw is
/// worth half a win, folded in rather than weighted separately), so
/// `weight = 0.5 * times_played + wins + 0.5 * draws`, and doubling both
/// sides to stay in integers gives `(times_played + 2 * wins + draws) /
/// 2`. Rounding that division up rather than down (`div_ceil`, not `/`)
/// matters for exactly one case: a single played-and-lost game
/// (`times_played = 1, wins = 0, draws = 0`) would otherwise compute
/// `(1 + 0 + 0) / 2 = 0`, the one way this formula could violate "a move
/// with at least one recorded game must never weigh zero" (nothing in
/// [`aggregate`](crate::aggregate::aggregate) should ever hand this
/// function a move with zero recorded games in the first place, so
/// that's the only case worth guarding).
///
/// Every intermediate value is computed in `u64` and only narrowed back
/// to `u32` (clamped via `unwrap_or(u32::MAX)`, not wrapped) at the very
/// end, since `2 * wins` alone can exceed `u32::MAX` once a move has been
/// played a couple billion times over... which won't happen, but the
/// headroom costs nothing and a wrapped weight silently corrupting a
/// book's move ordering is a far worse failure than one that saturates.
#[must_use]
pub fn weigh(stats: &[MoveStats]) -> Vec<BookMove> {
    stats
        .iter()
        .map(|s| {
            let times_played = u64::from(s.times_played);
            let wins = u64::from(s.wins);
            let draws = u64::from(s.draws);

            let weight = (times_played + 2 * wins + draws).div_ceil(2);

            let weight = u32::try_from(weight).unwrap_or(u32::MAX);
            match &s.opening_name {
                Some(name) => BookMove::with_name(s.mv, weight, name.clone()),
                None => BookMove::new(s.mv, weight),
            }
        })
        .collect()
}
