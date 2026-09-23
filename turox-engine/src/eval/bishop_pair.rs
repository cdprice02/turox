//! Bishop pair: a flat bonus for holding two or more bishops, independent
//! of pawn count or file openness. One of a handful of cheap, independent
//! evaluation terms (alongside rook-on-file and tempo) scoped together as a
//! batch of quick wins but implemented and gated as separate modules, so
//! SPRT can attribute each term's own result rather than one combined
//! (and likely noise-diluted) verdict for all of them at once.

use super::weights;
use crate::eval::phase::{pack, Tapered};
use turox_chess::board::Board;
use turox_chess::types::{Bitboard, Color, Piece};

/// Bonus for holding two or more bishops, flat across both phases per
/// `weights::BISHOP_PAIR_BONUS`'s own doc.
const BISHOP_PAIR_BONUS: Tapered = pack(weights::BISHOP_PAIR_BONUS.0, weights::BISHOP_PAIR_BONUS.1);

/// `color`'s bishop-pair contribution: `BISHOP_PAIR_BONUS` if `color` holds
/// two or more bishops, zero otherwise.
#[must_use]
pub const fn bishop_pair_score(board: &Board, color: Color) -> Tapered {
    bishop_pair_bonus(board.pieces(color, Piece::Bishop))
}

/// `BISHOP_PAIR_BONUS` if `bishops` has two or more bits set, zero
/// otherwise. A third bishop (e.g. from underpromotion) still counts as
/// holding the pair, not a second bonus: this is a bonus for covering both
/// square colours, not a per-bishop count.
const fn bishop_pair_bonus(bishops: Bitboard) -> Tapered {
    if bishops.count() >= 2 {
        BISHOP_PAIR_BONUS
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use turox_chess::types::Square;

    #[test]
    fn bishop_pair_bonus_is_zero_with_no_bishops() {
        assert_eq!(bishop_pair_bonus(Bitboard::EMPTY), 0);
    }

    #[test]
    fn bishop_pair_bonus_is_zero_with_exactly_one_bishop() {
        let bishops = Bitboard::EMPTY.with(Square::C1);
        assert_eq!(bishop_pair_bonus(bishops), 0);
    }

    #[test]
    fn bishop_pair_bonus_applies_with_exactly_two_bishops() {
        let bishops = Bitboard::EMPTY.with(Square::C1).with(Square::F1);
        assert_eq!(bishop_pair_bonus(bishops), BISHOP_PAIR_BONUS);
    }

    // A third bishop doesn't stack a second bonus: the term detects "the
    // pair", not "count of bishops beyond one", so a promoted third bishop
    // must not double the reward.
    #[test]
    fn bishop_pair_bonus_does_not_stack_for_a_third_bishop() {
        let bishops = Bitboard::EMPTY
            .with(Square::C1)
            .with(Square::F1)
            .with(Square::D2);
        assert_eq!(bishop_pair_bonus(bishops), BISHOP_PAIR_BONUS);
    }
}
