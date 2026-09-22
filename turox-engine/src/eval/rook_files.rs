//! Rook on an open or semi-open file: a per-rook bonus for standing on a
//! file with no friendly pawn to block its own advance, larger again when
//! no enemy pawn holds the file either. One of a handful of cheap,
//! independent evaluation terms (alongside bishop pair and tempo) scoped
//! together as a batch of quick wins but implemented and gated as separate
//! modules, so SPRT can attribute each term's own result.

use super::weights;
use crate::board::Board;
use crate::eval::phase::{pack, Tapered};
use crate::types::{Bitboard, Color, Piece};

/// Bonus per rook on a file with no pawn of either colour.
const OPEN_FILE_BONUS: Tapered = pack(
    weights::ROOK_OPEN_FILE_BONUS.0,
    weights::ROOK_OPEN_FILE_BONUS.1,
);

/// Bonus per rook on a file with no friendly pawn but at least one enemy
/// pawn.
const SEMI_OPEN_FILE_BONUS: Tapered = pack(
    weights::ROOK_SEMI_OPEN_FILE_BONUS.0,
    weights::ROOK_SEMI_OPEN_FILE_BONUS.1,
);

/// `color`'s rook-file contribution: `OPEN_FILE_BONUS` or
/// `SEMI_OPEN_FILE_BONUS` for each of `color`'s rooks, summed.
#[must_use]
pub const fn rook_files_score(board: &Board, color: Color) -> Tapered {
    let rooks = board.pieces(color, Piece::Rook);
    let pawns = board.pieces(color, Piece::Pawn);
    let enemy_pawns = board.pieces(color.flip(), Piece::Pawn);
    rook_files_bonus(rooks, pawns, enemy_pawns)
}

/// `OPEN_FILE_BONUS` for each bit of `rooks` on a file with no bit of
/// `pawns` or `enemy_pawns`; `SEMI_OPEN_FILE_BONUS` for each bit on a file
/// with no `pawns` bit but at least one `enemy_pawns` bit; zero for every
/// other rook. A rook doubled with another rook on the same open file
/// scores the bonus twice, once per rook, not once per file: two rooks
/// stacked on an open file is a stronger formation than one.
const fn rook_files_bonus(rooks: Bitboard, pawns: Bitboard, enemy_pawns: Bitboard) -> Tapered {
    let open_files = pawns.or(enemy_pawns).file_fill().not();
    let semi_open_or_open_files = pawns.file_fill().not();
    let semi_open_files = semi_open_or_open_files.xor(open_files);
    open_files.and(rooks).count().cast_signed() * OPEN_FILE_BONUS
        + semi_open_files.and(rooks).count().cast_signed() * SEMI_OPEN_FILE_BONUS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Square;

    #[test]
    fn no_bonus_with_no_rooks() {
        assert_eq!(
            rook_files_bonus(Bitboard::EMPTY, Bitboard::EMPTY, Bitboard::EMPTY),
            0
        );
    }

    #[test]
    fn open_file_bonus_applies_with_no_pawns_of_either_colour() {
        let rooks = Bitboard::EMPTY.with(Square::H1);
        assert_eq!(
            rook_files_bonus(rooks, Bitboard::EMPTY, Bitboard::EMPTY),
            OPEN_FILE_BONUS
        );
    }

    #[test]
    fn semi_open_file_bonus_applies_with_only_an_enemy_pawn_on_the_file() {
        let rooks = Bitboard::EMPTY.with(Square::H1);
        let enemy_pawns = Bitboard::EMPTY.with(Square::H7);
        assert_eq!(
            rook_files_bonus(rooks, Bitboard::EMPTY, enemy_pawns),
            SEMI_OPEN_FILE_BONUS
        );
    }

    // A friendly pawn closes the file regardless of what the enemy has on
    // it: `SHELTER_PENALTY`-style "friendly missing" and `OPEN_FILE_PENALTY`-
    // style "neither present" are two different questions in `king_safety`,
    // and this is the rook-file analogue of the same distinction, just
    // collapsed to a single bonus/no-bonus split rather than two stacking
    // penalties.
    #[test]
    fn no_bonus_with_a_friendly_pawn_on_the_file_regardless_of_the_enemy_pawn() {
        let rooks = Bitboard::EMPTY.with(Square::H1);
        let pawns = Bitboard::EMPTY.with(Square::H2);
        let enemy_pawns = Bitboard::EMPTY.with(Square::H7);
        assert_eq!(rook_files_bonus(rooks, pawns, enemy_pawns), 0);
        assert_eq!(rook_files_bonus(rooks, pawns, Bitboard::EMPTY), 0);
    }

    // Doubled rooks on the same open file score the bonus twice: this is a
    // per-rook count, not a per-file flag.
    #[test]
    fn two_rooks_on_the_same_open_file_each_score_the_bonus() {
        let rooks = Bitboard::EMPTY.with(Square::H1).with(Square::H4);
        assert_eq!(
            rook_files_bonus(rooks, Bitboard::EMPTY, Bitboard::EMPTY),
            OPEN_FILE_BONUS + OPEN_FILE_BONUS
        );
    }

    #[test]
    fn one_open_and_one_semi_open_rook_sum_both_bonuses() {
        let rooks = Bitboard::EMPTY.with(Square::A1).with(Square::H1);
        let enemy_pawns = Bitboard::EMPTY.with(Square::H7);
        assert_eq!(
            rook_files_bonus(rooks, Bitboard::EMPTY, enemy_pawns),
            OPEN_FILE_BONUS + SEMI_OPEN_FILE_BONUS
        );
    }
}
