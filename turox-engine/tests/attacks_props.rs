//! Property tests for `move_gen::attacks`: the executable version of the
//! contracts documented on each function in `src/move_gen/attacks.rs`.
//!
//! Every function gets a reference-equivalence check against an independent
//! implementation built directly from `Square::offset` stepping, not from
//! `tables`/`magic`, same discipline as `tests/magic_props.rs`.
//! `attackers_of` in particular is checked against the *forward* naive
//! definition rather than a second reverse one, per the module doc's
//! reasoning: a forward implementation can't get the pawn-color flip wrong
//! because it never inverts anything, which is exactly what makes it
//! trustworthy as a check on the real (reverse, superpiece-trick)
//! implementation.
//!
//! Concrete scenario tests (kingless boards, pawn direction, occupancy edge
//! effects) live in `tests/attacks.rs`, not here: this file is proptest only.

mod common;

use common::{any_bitboard, any_board, any_color, any_piece_with_king, any_square};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::move_gen::attacks::{
    attacked_by, attackers_of, is_attacked, king_square, piece_attacks,
};
use turox_engine::{Bitboard, Color, Piece, Square};

const ROOK_DIRS: [(i8, i8); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const KNIGHT_DELTAS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];

fn naive_slider_attacks(sq: Square, occupied: Bitboard, dirs: &[(i8, i8)]) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for &(df, dr) in dirs {
        let mut current = sq;
        while let Some(next) = current.offset(df, dr) {
            result = result.with(next);
            if occupied.contains(next) {
                break;
            }
            current = next;
        }
    }
    result
}

/// Reference definition of `piece_attacks`, independent of `tables`/`magic`.
fn naive_piece_attacks(piece: Piece, color: Color, sq: Square, occupied: Bitboard) -> Bitboard {
    match piece {
        Piece::Pawn => {
            let dr = match color {
                Color::White => 1,
                Color::Black => -1,
            };
            let mut result = Bitboard::EMPTY;
            if let Some(t) = sq.offset(-1, dr) {
                result = result.with(t);
            }
            if let Some(t) = sq.offset(1, dr) {
                result = result.with(t);
            }
            result
        }
        Piece::Knight => {
            let mut result = Bitboard::EMPTY;
            for (df, dr) in KNIGHT_DELTAS {
                if let Some(t) = sq.offset(df, dr) {
                    result = result.with(t);
                }
            }
            result
        }
        Piece::King => {
            let mut result = Bitboard::EMPTY;
            for df in -1i8..=1 {
                for dr in -1i8..=1 {
                    if df == 0 && dr == 0 {
                        continue;
                    }
                    if let Some(t) = sq.offset(df, dr) {
                        result = result.with(t);
                    }
                }
            }
            result
        }
        Piece::Bishop => naive_slider_attacks(sq, occupied, &BISHOP_DIRS),
        Piece::Rook => naive_slider_attacks(sq, occupied, &ROOK_DIRS),
        Piece::Queen => naive_slider_attacks(sq, occupied, &ROOK_DIRS).or(naive_slider_attacks(
            sq,
            occupied,
            &BISHOP_DIRS,
        )),
    }
}

fn naive_attacked_by(board: &Board, by: Color, occupied: Bitboard) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if cp.color() == by {
                result = result.or(naive_piece_attacks(cp.piece(), by, sq, occupied));
            }
        }
    }
    result
}

fn naive_attackers_of(board: &Board, target: Square, by: Color) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if cp.color() == by
                && naive_piece_attacks(cp.piece(), by, sq, board.occupied()).contains(target)
            {
                result = result.with(sq);
            }
        }
    }
    result
}

proptest! {
    #[test]
    fn piece_attacks_matches_naive_stepper(
        piece in any_piece_with_king(),
        color in any_color(),
        sq in any_square(),
        occupied in any_bitboard(),
    ) {
        prop_assert_eq!(
            piece_attacks(piece, color, sq, occupied),
            naive_piece_attacks(piece, color, sq, occupied)
        );
    }

    #[test]
    fn attacked_by_matches_naive_union(board in any_board(), by in any_color(), occupied in any_bitboard()) {
        prop_assert_eq!(attacked_by(&board, by, occupied), naive_attacked_by(&board, by, occupied));
    }

    #[test]
    fn attacked_by_against_own_occupancy_matches_naive(board in any_board(), by in any_color()) {
        let occ = board.occupied();
        prop_assert_eq!(attacked_by(&board, by, occ), naive_attacked_by(&board, by, occ));
    }

    #[test]
    fn attackers_of_matches_naive_forward_definition(board in any_board(), sq in any_square(), by in any_color()) {
        prop_assert_eq!(attackers_of(&board, sq, by), naive_attackers_of(&board, sq, by));
    }

    #[test]
    fn is_attacked_matches_naive(board in any_board(), sq in any_square(), by in any_color()) {
        prop_assert_eq!(
            is_attacked(&board, sq, by),
            naive_attacked_by(&board, by, board.occupied()).contains(sq)
        );
    }

    #[test]
    fn king_square_finds_the_real_king(board in any_board(), color in any_color()) {
        let expected = board.pieces(color, Piece::King).lsb();
        prop_assert_eq!(king_square(&board, color), expected);
    }
}
