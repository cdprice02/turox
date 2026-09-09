//! Concrete scenario tests for `move_gen::attacks`'s public API.
//!
//! `tests/attacks_props.rs` has the exhaustive-against-independent-reference
//! coverage; these are hand-picked positions pinning specific, easy-to-get-
//! backwards cases (kingless boards, pawn attack direction, occupancy edge
//! effects) that are worth reading as documentation in their own right.

use turox_engine::board::Board;
use turox_engine::move_gen::attacks::{attacked_by, attackers_of, in_check, king_square, pinned};
use turox_engine::{Bitboard, Color, Square};

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

// ---- pinned (#113) ----

#[test]
fn a_piece_pinned_along_a_file_by_a_rook_is_reported() {
    // White king e1, White bishop e2, Black rook e8: the bishop is the only
    // piece between king and rook, so it's pinned. Same position
    // `tests/legal.rs`'s `pinned_bishop_cannot_move_off_the_pin_line` uses to
    // check the *consuming* legality rule; this checks the fact underneath it.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4B3/4K3 w - - 0 1").expect("valid FEN");
    let expected = Bitboard::EMPTY.with(Square::E2);
    assert_eq!(pinned(&board, Color::White, Square::E1), expected);
}

#[test]
fn a_piece_pinned_along_a_diagonal_by_a_bishop_is_reported() {
    // White king e1, White knight d2, Black bishop a5: e1-a5 is a diagonal
    // (e1, d2, c3, b4, a5), so the knight on d2 is pinned. A rook-only
    // implementation (missing the bishop-ray half entirely) would miss this
    // one while still passing the file-pin case above.
    let board = Board::try_from_fen("8/8/8/b7/8/8/3N4/4K3 w - - 0 1").expect("valid FEN");
    let expected = Bitboard::EMPTY.with(Square::D2);
    assert_eq!(pinned(&board, Color::White, Square::E1), expected);
}

#[test]
fn a_queen_pins_along_a_file_exactly_like_a_rook() {
    let board = Board::try_from_fen("4q3/8/8/8/8/8/4N3/4K3 w - - 0 1").expect("valid FEN");
    let expected = Bitboard::EMPTY.with(Square::E2);
    assert_eq!(pinned(&board, Color::White, Square::E1), expected);
}

#[test]
fn two_own_pieces_on_the_same_ray_pin_neither() {
    // White king e1, White bishop e2, White knight e3, Black rook e8: the
    // rook's ray never reaches past e3, so it never even reaches the enemy
    // rook, and moving either e2 or e3 alone would still leave the other
    // blocking the file. Neither piece is actually pinned.
    let board = Board::try_from_fen("4r3/8/8/8/8/4N3/4B3/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(pinned(&board, Color::White, Square::E1), Bitboard::EMPTY);
}

#[test]
fn an_enemy_piece_between_king_and_slider_pins_nothing() {
    // White king e1, Black knight e3 (not a slider, blocks regardless), Black
    // rook e8: the knight isn't `color`'s own piece, so even though it's the
    // thing actually blocking the file, it can never be in the returned set,
    // and nothing of White's is pinned by this arrangement at all.
    let board = Board::try_from_fen("4r3/8/8/8/8/4n3/8/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(pinned(&board, Color::White, Square::E1), Bitboard::EMPTY);
}

#[test]
fn pin_detection_is_color_symmetric() {
    // Exact color-flip of `a_piece_pinned_along_a_file_by_a_rook_is_reported`:
    // Black king e8, Black bishop e7, White rook e1. This project's standing
    // warning about Color-crossed-with-direction bugs means the White-only
    // case above proves nothing about Black on its own.
    let board = Board::try_from_fen("4k3/4b3/8/8/8/8/8/4R3 b - - 0 1").expect("valid FEN");
    let expected = Bitboard::EMPTY.with(Square::E7);
    assert_eq!(pinned(&board, Color::Black, Square::E8), expected);
}

#[test]
fn two_simultaneous_pins_on_different_rays_are_both_found() {
    // White king e1: White rook e2 pinned along the file by Black queen e8,
    // and independently White knight c3 pinned along the a5-e1 diagonal by
    // Black bishop a5. Neither ray's computation should interfere with the
    // other's.
    let board = Board::try_from_fen("4q3/8/8/b7/8/2N5/4R3/4K3 w - - 0 1").expect("valid FEN");
    let expected = Bitboard::EMPTY.with(Square::E2).with(Square::C3);
    assert_eq!(pinned(&board, Color::White, Square::E1), expected);
}

#[test]
fn start_position_has_no_pins() {
    let board = Board::start_pos();
    assert_eq!(pinned(&board, Color::White, Square::E1), Bitboard::EMPTY);
    assert_eq!(pinned(&board, Color::Black, Square::E8), Bitboard::EMPTY);
}
