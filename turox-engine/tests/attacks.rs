//! Concrete scenario tests for `move_gen::attacks`'s public API.
//!
//! `tests/attacks_props.rs` has the exhaustive-against-independent-reference
//! coverage; these are hand-picked positions pinning specific, easy-to-get-
//! backwards cases (kingless boards, pawn attack direction, occupancy edge
//! effects) that are worth reading as documentation in their own right.

use turox_engine::board::Board;
use turox_engine::move_gen::attacks::{attacked_by, attackers_of, in_check, king_square};
use turox_engine::{Color, Square};

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
// pawn on d3 attacks c4/e4, not c2/e2; these pin that down concretely rather
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
    // must) reveals the ray continues straight through to e1, exactly the case
    // `attacked_by` takes `occupied` explicitly for.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4K3/8 w - - 0 1").expect("valid FEN");
    let occupied_with_king = board.occupied();
    let occupied_without_king = occupied_with_king.without(Square::E2);

    assert!(!attacked_by(&board, Color::Black, occupied_with_king).contains(Square::E1));
    assert!(attacked_by(&board, Color::Black, occupied_without_king).contains(Square::E1));
}
