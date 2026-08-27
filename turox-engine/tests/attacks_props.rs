//! Property tests for `move_gen::attacks`: the executable version of the
//! contracts documented on each function in `src/move_gen/attacks.rs`.
//!
//! Every function gets a reference-equivalence check against an independent
//! implementation built directly from `Square::offset` stepping, not from
//! `tables`/`magic` — same discipline as `tests/tables_props.rs` and
//! `tests/magic_props.rs`. `attackers_of` in
//! particular is checked against the *forward* naive definition rather than a
//! second reverse one, per the module doc's reasoning: a forward
//! implementation can't get the pawn-color flip wrong because it never inverts
//! anything, which is exactly what makes it trustworthy as a check on the real
//! (reverse, superpiece-trick) implementation.

mod common;

use common::any_board;
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::move_gen::attacks::{
    attacked_by, attackers_of, in_check, is_attacked, king_square, piece_attacks,
};
use turox_engine::{Bitboard, Color, Piece, Square};

fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
}

fn any_color() -> impl Strategy<Value = Color> {
    prop_oneof![Just(Color::White), Just(Color::Black)]
}

fn any_piece() -> impl Strategy<Value = Piece> {
    prop_oneof![
        Just(Piece::Pawn),
        Just(Piece::Knight),
        Just(Piece::Bishop),
        Just(Piece::Rook),
        Just(Piece::Queen),
        Just(Piece::King),
    ]
}

fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

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
        piece in any_piece(),
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
    fn is_attacked_matches_attacked_by_own_occupancy(board in any_board(), sq in any_square(), by in any_color()) {
        prop_assert_eq!(
            is_attacked(&board, sq, by),
            attacked_by(&board, by, board.occupied()).contains(sq)
        );
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

    #[test]
    fn in_check_matches_is_attacked_on_the_kings_square(board in any_board(), color in any_color()) {
        let expected = king_square(&board, color)
            .is_some_and(|sq| is_attacked(&board, sq, color.flip()));
        prop_assert_eq!(in_check(&board, color), expected);
    }
}

#[test]
fn king_square_is_none_on_a_kingless_board() {
    let board = Board::default();
    assert_eq!(king_square(&board, Color::White), None);
    assert_eq!(king_square(&board, Color::Black), None);
}

#[test]
fn in_check_is_false_on_a_kingless_board() {
    let board = Board::default();
    assert!(!in_check(&board, Color::White));
    assert!(!in_check(&board, Color::Black));
}

// ---- Pawn direction asymmetry ----
//
// The one place a Color-flip bug in `attackers_of` is invisible on a
// vertically symmetric board: knight/king/slider attack relations are
// symmetric ("a attacks b" iff "b attacks a"), pawn relations are not. A white
// pawn on d3 attacks c4/e4, not c2/e2 — these pin that down concretely rather
// than trusting the proptest oracle (built with the same offset-stepping
// technique) to be independently immune to the same mistake.

#[test]
fn white_pawn_attackers_are_found_diagonally_ahead_not_behind() {
    let board = Board::try_from_fen("8/8/8/8/8/3P4/8/8 w - - 0 1").expect("valid FEN");
    assert!(attackers_of(&board, Square::C4, Color::White).contains(Square::D3));
    assert!(attackers_of(&board, Square::E4, Color::White).contains(Square::D3));
    assert!(attackers_of(&board, Square::C2, Color::White).is_empty());
    assert!(attackers_of(&board, Square::E2, Color::White).is_empty());
}

#[test]
fn black_pawn_attackers_are_found_diagonally_ahead_not_behind() {
    let board = Board::try_from_fen("8/8/8/3p4/8/8/8/8 b - - 0 1").expect("valid FEN");
    assert!(attackers_of(&board, Square::C4, Color::Black).contains(Square::D5));
    assert!(attackers_of(&board, Square::E4, Color::Black).contains(Square::D5));
    assert!(attackers_of(&board, Square::C6, Color::Black).is_empty());
    assert!(attackers_of(&board, Square::E6, Color::Black).is_empty());
}

// ---- in_check ----

#[test]
fn king_in_check_from_a_rook_down_an_open_file() {
    let board = Board::try_from_fen("4r3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    assert!(in_check(&board, Color::White));
}

#[test]
fn king_not_in_check_when_the_file_is_blocked() {
    let board = Board::try_from_fen("4r3/8/8/8/4P3/8/8/4K3 w - - 0 1").expect("valid FEN");
    assert!(!in_check(&board, Color::White));
}

// ---- attacked_by / explicit occupancy ----

#[test]
fn attacked_by_with_the_kings_own_square_removed_reveals_the_square_behind_it() {
    // Rook on e8, king on e2: with the king still in the occupancy, the rook's
    // ray down the e-file stops (inclusively) at e2, so e1 reads as safe. Lifting
    // the king out of `occupied` (as a caller checking "is e1 safe to step onto"
    // must) reveals the ray continues straight through to e1 — exactly the case
    // `attacked_by` takes `occupied` explicitly for.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4K3/8 w - - 0 1").expect("valid FEN");
    let occupied_with_king = board.occupied();
    let occupied_without_king = occupied_with_king.without(Square::E2);

    assert!(!attacked_by(&board, Color::Black, occupied_with_king).contains(Square::E1));
    assert!(attacked_by(&board, Color::Black, occupied_without_king).contains(Square::E1));
}
