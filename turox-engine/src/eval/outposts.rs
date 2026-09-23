//! Outposts and king-pawn tropism: two cheap terms grouped into one module
//! because they share their input (the same pawn bitboards
//! `pawn_structure` already computes), not because they're the same kind
//! of term. Outposts rewards a minor piece anchored where no pawn can ever
//! dislodge it; tropism rewards a king standing near the pawns that decide
//! an endgame. They're otherwise unrelated, and land in opposite phase
//! lanes on purpose: see each function's own doc.

use crate::eval::phase::{pack, Tapered};
use crate::eval::weights;
use turox_chess::board::Board;
use turox_chess::move_gen::attacks::king_square;
use turox_chess::move_gen::tables::pawn_attacks;
use turox_chess::types::{Bitboard, Color, Piece, Square};

/// Bonus per knight or bishop standing on an outpost, flat across both
/// phases per `weights::OUTPOST_BONUS`'s own doc.
const OUTPOST_BONUS: Tapered = pack(weights::OUTPOST_BONUS.0, weights::OUTPOST_BONUS.1);

/// Bonus per unit of king-pawn closeness, endgame lane only, per
/// `weights::TROPISM_BONUS`'s own doc.
const TROPISM_BONUS: Tapered = pack(weights::TROPISM_BONUS.0, weights::TROPISM_BONUS.1);

/// The longest possible `Square::distance` (Chebyshev) between two squares
/// on an 8x8 board: a structural fact about the board, not a tuned
/// magnitude, so unlike every constant in `weights`, this isn't expected
/// to move.
pub const MAX_DISTANCE: u8 = 7;

/// `color`'s outpost contribution: `OUTPOST_BONUS` for each knight or
/// bishop `color` has standing on a square that one of `color`'s own pawns
/// currently defends and no enemy pawn, now or after advancing any
/// distance, can ever attack.
#[must_use]
pub fn outpost_score(board: &Board, color: Color) -> Tapered {
    let minors = board
        .pieces(color, Piece::Knight)
        .or(board.pieces(color, Piece::Bishop));
    let pawns = board.pieces(color, Piece::Pawn);
    let enemy_pawns = board.pieces(color.flip(), Piece::Pawn);
    outpost_bonus(minors, pawns, enemy_pawns, color)
}

/// `OUTPOST_BONUS` for each bit of `minors` that's both on a square some
/// bit of `pawns` currently attacks (`color`'s own pawns defending it) and
/// outside `enemy_pawns`'s `front_attack_span` (no enemy pawn, however far
/// it advances, can ever attack it).
///
/// Not a `const fn`, unlike every sibling term in this crate: computing
/// which squares `pawns` currently defends needs a per-pawn
/// `move_gen::tables::pawn_attacks` union, the same square-by-square walk
/// `eval_white_pov` itself already uses for material and PST, not the pure
/// bitwise set algebra `pawn_structure`/`king_safety`/`bishop_pair`/
/// `rook_files` get away with.
fn outpost_bonus(
    minors: Bitboard,
    pawns: Bitboard,
    enemy_pawns: Bitboard,
    color: Color,
) -> Tapered {
    let enemy_attack_span = enemy_pawns.front_attack_span(color.flip());
    let friendly_attacks = pawns
        .into_iter()
        .map(|sq| pawn_attacks(color, sq))
        .reduce(Bitboard::or)
        .unwrap_or(Bitboard::EMPTY);

    let unspanned_minors = minors.and_not(enemy_attack_span);
    let outposts = unspanned_minors.and(friendly_attacks);
    outposts.count().cast_signed() * OUTPOST_BONUS
}

/// `color`'s king-pawn tropism contribution: `TROPISM_BONUS` once per unit
/// that `color`'s king stands closer than `MAX_DISTANCE` to each pawn on
/// the board, summed over every pawn, both colours. `0` if `color` has no
/// king (unreachable in a real game, but `any_board()`-style proptest
/// input can place none, the same guard `king_safety_score` uses).
#[must_use]
pub fn king_pawn_tropism_score(board: &Board, color: Color) -> Tapered {
    let Some(king_sq) = king_square(board, color) else {
        return 0;
    };
    let pawns = board
        .pieces(Color::White, Piece::Pawn)
        .or(board.pieces(Color::Black, Piece::Pawn));
    tropism_bonus(king_sq, pawns)
}

/// `TROPISM_BONUS` once for every unit `MAX_DISTANCE - king_sq.distance(sq)`
/// comes to, summed over every bit of `pawns`.
///
/// Not a `const fn`, for the same reason `outpost_bonus` isn't: summing a
/// per-pawn `Square::distance` needs the same square-by-square walk.
fn tropism_bonus(king_sq: Square, pawns: Bitboard) -> Tapered {
    pawns
        .into_iter()
        .map(|sq| TROPISM_BONUS * i32::from(MAX_DISTANCE - king_sq.distance(sq)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_distance_matches_the_farthest_corner_to_corner_case() {
        assert_eq!(Square::A1.distance(Square::H8), MAX_DISTANCE);
    }

    // ---- outpost_bonus ----

    #[test]
    fn outpost_bonus_is_zero_with_no_minors() {
        assert_eq!(
            outpost_bonus(
                Bitboard::EMPTY,
                Bitboard::EMPTY,
                Bitboard::EMPTY,
                Color::White
            ),
            0
        );
    }

    #[test]
    fn outpost_bonus_applies_when_a_friendly_pawn_defends_and_no_enemy_pawn_exists() {
        let minors = Bitboard::EMPTY.with(Square::D5);
        let pawns = Bitboard::EMPTY.with(Square::C4);
        assert_eq!(
            outpost_bonus(minors, pawns, Bitboard::EMPTY, Color::White),
            OUTPOST_BONUS
        );
    }

    #[test]
    fn outpost_bonus_is_zero_when_no_friendly_pawn_defends_the_square() {
        let minors = Bitboard::EMPTY.with(Square::D5);
        assert_eq!(
            outpost_bonus(minors, Bitboard::EMPTY, Bitboard::EMPTY, Color::White),
            0
        );
    }

    // A friendly pawn defends d5, but a Black pawn still on d7 could reach
    // it as it advances (d5 sits inside d7's own front-attack span), so
    // the square isn't a real outpost yet.
    #[test]
    fn outpost_bonus_is_zero_when_an_enemy_pawn_could_still_reach_the_square() {
        let minors = Bitboard::EMPTY.with(Square::D5);
        let pawns = Bitboard::EMPTY.with(Square::C4);
        let enemy_pawns = Bitboard::EMPTY.with(Square::D7);
        assert_eq!(outpost_bonus(minors, pawns, enemy_pawns, Color::White), 0);
    }

    // The asymmetric case this repo's `{Color}x{direction}` history says to
    // write explicitly: the same shape as
    // `outpost_bonus_applies_when_a_friendly_pawn_defends_and_no_enemy_pawn_exists`,
    // reflected for Black (whose pawns defend *downward*), not just White's
    // own case mirrored. A hardcoded White forward direction would read
    // Black's pawn as attacking the wrong squares and miss this outpost.
    #[test]
    fn outpost_bonus_applies_for_black_defended_from_the_opposite_direction() {
        let minors = Bitboard::EMPTY.with(Square::D4);
        let pawns = Bitboard::EMPTY.with(Square::C5);
        assert_eq!(
            outpost_bonus(minors, pawns, Bitboard::EMPTY, Color::Black),
            OUTPOST_BONUS
        );
    }

    // ---- tropism_bonus ----

    #[test]
    fn tropism_bonus_is_zero_with_no_pawns() {
        assert_eq!(tropism_bonus(Square::E4, Bitboard::EMPTY), 0);
    }

    #[test]
    fn tropism_bonus_is_zero_at_the_maximum_distance() {
        let pawns = Bitboard::EMPTY.with(Square::H8);
        assert_eq!(tropism_bonus(Square::A1, pawns), 0);
    }

    #[test]
    fn tropism_bonus_scales_with_how_far_below_the_maximum_the_distance_is() {
        let pawns = Bitboard::EMPTY.with(Square::A2);
        assert_eq!(
            tropism_bonus(Square::A1, pawns),
            TROPISM_BONUS * Tapered::from(MAX_DISTANCE - Square::A1.distance(Square::A2))
        );
    }

    #[test]
    fn tropism_bonus_sums_across_multiple_pawns() {
        let single_close = Bitboard::EMPTY.with(Square::A2);
        let single_far = Bitboard::EMPTY.with(Square::H8);
        let both = single_close.or(single_far);
        assert_eq!(
            tropism_bonus(Square::A1, both),
            tropism_bonus(Square::A1, single_close) + tropism_bonus(Square::A1, single_far)
        );
    }
}
