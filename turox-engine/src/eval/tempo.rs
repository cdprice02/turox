//! Tempo: a small midgame-only bonus for the side to move. Unlike every
//! other term in this crate, this one is a function of
//! `board.side_to_move()` directly rather than one independently-scored
//! value per colour summed against its opposite: `eval_white_pov` was
//! side-to-move-independent before this term existed, and this is the
//! first one that changes that. One of a handful of cheap, independent
//! evaluation terms (alongside bishop pair and rook files) scoped
//! together as a batch of quick wins but implemented and gated as
//! separate modules, so SPRT can attribute each term's own result.

use super::weights;
use crate::eval::phase::{pack, Tapered};
use turox_chess::board::Board;
use turox_chess::Color;

/// Bonus for the side to move, midgame lane only.
const TEMPO_BONUS: Tapered = pack(weights::TEMPO_BONUS.0, weights::TEMPO_BONUS.1);

/// `TEMPO_BONUS` signed by whoever `board.side_to_move()` actually is:
/// positive for White, negative for Black, matching `eval_white_pov`'s
/// White-relative convention.
///
/// A single value, not a per-colour pair subtracted the way every other
/// term in this crate is: tempo isn't "how much White's position is worth
/// plus how much Black's is", it's a one-time bonus for whoever's turn it
/// is, so there's nothing to compute for the side not to move.
#[must_use]
pub const fn tempo_score(board: &Board) -> Tapered {
    match board.side_to_move() {
        Color::White => TEMPO_BONUS,
        Color::Black => -TEMPO_BONUS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_to_move_gets_the_positive_bonus() {
        let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
        assert_eq!(tempo_score(&board), TEMPO_BONUS);
    }

    #[test]
    fn black_to_move_gets_the_negative_bonus() {
        let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 b - - 0 1").expect("valid FEN");
        assert_eq!(tempo_score(&board), -TEMPO_BONUS);
    }
}
