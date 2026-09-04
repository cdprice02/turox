//! Pawn-structure evaluation: doubled, isolated, and passed pawns.
//!
//! Three independent per-pawn terms, each scored once per pawn that
//! matches its shape and summed into the same [`super::phase::Tapered`]
//! accumulator `eval_white_pov` already carries for material and piece-
//! square terms, so no separate blending pass is needed here.

use crate::board::Board;
use crate::eval::phase::{pack, Tapered};
use crate::types::{Bitboard, Color, Piece};
use crate::Direction;

/// Penalty for each pawn beyond the first on a file: doubled pawns block
/// each other's advance and don't add proportional extra defensive value.
const DOUBLED_PENALTY: Tapered = pack(-10, -20);

/// Penalty per pawn with no friendly pawn on an adjacent file: isolated
/// pawns can never be defended by another pawn.
const ISOLATED_PENALTY: Tapered = pack(-10, -10);

/// Bonus per pawn with a clear path to promotion: worth more in the
/// endgame, where there are fewer pieces left to stop it and a king
/// nearby to escort it.
const PASSED_BONUS: Tapered = pack(10, 20);

/// `color`'s total pawn-structure contribution: doubled and isolated
/// penalties plus the passed-pawn bonus, summed over every pawn `color`
/// has on the board.
#[must_use]
pub const fn pawn_structure_score(board: &Board, color: Color) -> Tapered {
    let pawns = board.pieces(color, Piece::Pawn);
    let enemy_pawns = board.pieces(color.flip(), Piece::Pawn);
    doubled_penalty(pawns, color)
        + isolated_penalty(pawns)
        + passed_bonus(pawns, enemy_pawns, color)
}

/// `DOUBLED_PENALTY` once for every pawn beyond the first on a file: a
/// file with zero or one pawn in `pawns` contributes nothing, a file with
/// `n >= 2` contributes `n - 1` penalties.
const fn doubled_penalty(pawns: Bitboard, color: Color) -> Tapered {
    (pawns.and(pawns.front_span(color))).count().cast_signed() * DOUBLED_PENALTY
}

/// `ISOLATED_PENALTY` once for every pawn in `pawns` with no other pawn in
/// `pawns` on the file immediately to its east or west, at any rank.
/// Symmetric in direction: this doesn't depend on either color's forward
/// direction, only on which files are occupied.
const fn isolated_penalty(pawns: Bitboard) -> Tapered {
    isolani(pawns).count().cast_signed() * ISOLATED_PENALTY
}

const fn isolani(pawns: Bitboard) -> Bitboard {
    let east = pawns.shift(Direction::East);
    let west = pawns.shift(Direction::West);
    pawns.and_not(east.or(west).file_fill())
}

/// `PASSED_BONUS` once for every pawn in `pawns` with no square of
/// `enemy_pawns` on its `Bitboard::front_attack_span(color)`: own file or
/// either adjacent file, strictly ahead per `color`'s forward direction.
const fn passed_bonus(pawns: Bitboard, enemy_pawns: Bitboard, color: Color) -> Tapered {
    let enemy_front_attack_span = enemy_pawns.front_attack_span(color.flip());
    pawns.and_not(enemy_front_attack_span).count().cast_signed() * PASSED_BONUS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Square;

    // ---- doubled_penalty ----

    #[test]
    fn doubled_penalty_is_zero_on_an_empty_board() {
        assert_eq!(doubled_penalty(Bitboard::EMPTY, Color::White), 0);
    }

    #[test]
    fn doubled_penalty_is_zero_for_a_single_pawn() {
        let pawns = Bitboard::EMPTY.with(Square::D4);
        assert_eq!(doubled_penalty(pawns, Color::White), 0);
    }

    #[test]
    fn doubled_penalty_counts_one_pawn_beyond_the_first_on_one_file() {
        let pawns = Bitboard::EMPTY.with(Square::D2).with(Square::D4);
        assert_eq!(doubled_penalty(pawns, Color::White), DOUBLED_PENALTY);
    }

    #[test]
    fn doubled_penalty_counts_two_pawns_beyond_the_first_on_one_file() {
        let pawns = Bitboard::EMPTY
            .with(Square::D2)
            .with(Square::D4)
            .with(Square::D6);
        assert_eq!(
            doubled_penalty(pawns, Color::White),
            DOUBLED_PENALTY + DOUBLED_PENALTY
        );
    }

    #[test]
    fn doubled_penalty_is_zero_for_pawns_on_different_files() {
        let pawns = Bitboard::EMPTY.with(Square::D4).with(Square::E4);
        assert_eq!(doubled_penalty(pawns, Color::White), 0);
    }

    // `color` only picks a fixed forward direction to count doubled pawns
    // along; which one shouldn't matter, since exactly one pawn per file
    // is the extreme in either direction and every other pawn on that
    // file trails some other pawn from both ends.
    #[test]
    fn doubled_penalty_does_not_depend_on_which_color_is_passed() {
        let pawns = Bitboard::EMPTY
            .with(Square::D2)
            .with(Square::D4)
            .with(Square::D6);
        assert_eq!(
            doubled_penalty(pawns, Color::White),
            doubled_penalty(pawns, Color::Black)
        );
    }

    // ---- isolated_penalty ----

    #[test]
    fn isolated_penalty_counts_a_lone_pawn() {
        let pawns = Bitboard::EMPTY.with(Square::D4);
        assert_eq!(isolated_penalty(pawns), ISOLATED_PENALTY);
    }

    #[test]
    fn isolated_penalty_drops_to_zero_once_an_adjacent_file_gets_a_pawn() {
        // Neither d4 nor c4 is isolated anymore: each has a friendly pawn
        // on an adjacent file, so both contribute zero, not just one.
        let pawns = Bitboard::EMPTY.with(Square::D4).with(Square::C4);
        assert_eq!(isolated_penalty(pawns), 0);
    }

    #[test]
    fn isolated_penalty_is_not_rescued_by_a_pawn_two_files_away() {
        // b4 is two files from d4, not adjacent, so d4 stays isolated; b4
        // has nothing on a4 or c4 either, so it's isolated too.
        let pawns = Bitboard::EMPTY.with(Square::D4).with(Square::B4);
        assert_eq!(isolated_penalty(pawns), ISOLATED_PENALTY + ISOLATED_PENALTY);
    }

    // "Adjacent file" means any rank on that file, not just the same rank:
    // a friendly pawn on c6 must rescue a pawn on d4 from being isolated
    // exactly as much as one on c4 would. Every other isolated_penalty test
    // above happens to place its pawns on the same rank, which can't tell
    // "checks the adjacent file" apart from "checks the one square
    // diagonally/orthogonally adjacent" — this is the case that actually
    // distinguishes the two.
    #[test]
    fn isolated_penalty_is_rescued_by_a_pawn_on_an_adjacent_file_at_any_rank() {
        let pawns = Bitboard::EMPTY.with(Square::D4).with(Square::C6);
        assert_eq!(isolated_penalty(pawns), 0);
    }

    // ---- passed_bonus ----

    #[test]
    fn passed_bonus_counts_every_pawn_when_no_enemy_pawns_exist() {
        let pawns = Bitboard::EMPTY.with(Square::D4).with(Square::A2);
        assert_eq!(
            passed_bonus(pawns, Bitboard::EMPTY, Color::White),
            PASSED_BONUS + PASSED_BONUS
        );
    }

    #[test]
    fn passed_bonus_is_cancelled_by_an_enemy_pawn_directly_ahead_same_file() {
        let pawns = Bitboard::EMPTY.with(Square::D4);
        let enemy_pawns = Bitboard::EMPTY.with(Square::D6);
        assert_eq!(passed_bonus(pawns, enemy_pawns, Color::White), 0);
    }

    #[test]
    fn passed_bonus_is_cancelled_by_an_enemy_pawn_on_an_adjacent_file_ahead() {
        let pawns = Bitboard::EMPTY.with(Square::D4);
        let enemy_pawns = Bitboard::EMPTY.with(Square::E6);
        assert_eq!(passed_bonus(pawns, enemy_pawns, Color::White), 0);
    }

    // The asymmetric case most likely to catch a forward-direction bug:
    // an enemy pawn strictly *behind* (from the mover's own point of view)
    // must not disqualify a passed pawn, for either color.
    #[test]
    fn passed_bonus_ignores_an_enemy_pawn_behind_white() {
        let pawns = Bitboard::EMPTY.with(Square::D4);
        let enemy_pawns = Bitboard::EMPTY.with(Square::D2);
        assert_eq!(passed_bonus(pawns, enemy_pawns, Color::White), PASSED_BONUS);
    }

    #[test]
    fn passed_bonus_ignores_an_enemy_pawn_behind_black() {
        let pawns = Bitboard::EMPTY.with(Square::D5);
        let enemy_pawns = Bitboard::EMPTY.with(Square::D7);
        assert_eq!(passed_bonus(pawns, enemy_pawns, Color::Black), PASSED_BONUS);
    }
}
