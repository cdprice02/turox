//! Concrete scenario tests for `move_gen::legal::legal_moves`.
//!
//! The classic cases a naive or subtly-wrong copy-make filter gets wrong:
//! king moves into/out of check, pins, discovered checks, the infamous
//! en-passant-discovered-check position, and check/stalemate producing an
//! empty legal move list. `tests/legal_props.rs` has the property coverage
//! (soundness/completeness against `pseudo_legal_moves`, and internal
//! consistency after a real move); this file is concrete only.

use turox_engine::board::Board;
use turox_engine::move_gen::attacks::in_check;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::Square;

fn contains(list: &MoveList, from: Square, to: Square) -> bool {
    list.iter().any(|&m| m.from() == from && m.to() == to)
}

#[test]
fn king_cannot_move_into_an_attacked_square() {
    // Black rook on e8 covers the entire e-file; the white king on e1 has
    // d1/f1 (off the file) but not e2 (still on the file, still attacked).
    let board = Board::try_from_fen("4r3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E1, Square::E2));
    assert!(contains(&moves, Square::E1, Square::D1));
    assert!(contains(&moves, Square::E1, Square::F1));
}

#[test]
fn king_in_check_can_capture_the_checking_piece() {
    let board = Board::try_from_fen("8/8/8/8/8/8/4r3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(contains(&moves, Square::E1, Square::E2));
}

#[test]
fn pinned_bishop_cannot_move_off_the_pin_line() {
    // White king e1, white bishop e2, black rook e8: the bishop is pinned
    // along the e-file. It can't step to a diagonal square off that file;
    // doing so would expose the king to the rook.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4B3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E2, Square::D3));
    assert!(!contains(&moves, Square::E2, Square::F3));
}

#[test]
fn pinned_rook_can_still_move_along_the_pin_line() {
    // Same pin, but the pinned piece is itself a rook: sliding along e-file
    // (toward or away from the king, short of capturing/passing the pinner)
    // stays legal, since the king is never exposed.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4R3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(contains(&moves, Square::E2, Square::E5));
}

#[test]
fn en_passant_capture_that_discovers_a_rank_check_is_illegal() {
    // The textbook case: White king a5, White pawn c5, Black pawn d5 (just
    // double-pushed, ep target d6), Black rook h5. Capturing en passant
    // (c5xd6) removes the d5 pawn from the board, the one piece blocking the
    // rook's rank check on the king, so it must NOT appear as legal, even
    // though it's a perfectly ordinary pseudolegal en passant capture.
    let board = Board::try_from_fen("8/8/8/K1Pp3r/8/8/8/8 w - d6 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::C5, Square::D6));
}

#[test]
fn en_passant_capture_that_discovers_a_rank_check_is_illegal_for_black_too() {
    // Color-flipped mirror of the case above: Black king a4, Black pawn c4,
    // White pawn d4 (just double-pushed from d2, ep target d3), White rook
    // h4. Capturing en passant (c4xd3) removes the d4 pawn, the one piece
    // blocking the rook's rank check on the Black king. A fix that only
    // handles White's capture direction (destination one rank *above* the
    // capturing pawn) would pass the test above and still get this one
    // wrong, since Black's capture direction is one rank *below*.
    let board = Board::try_from_fen("8/8/8/8/k1pP3R/8/8/8 b - d3 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::C4, Square::D3));
}

#[test]
fn en_passant_capture_that_discovers_a_rank_check_is_illegal_with_king_on_the_other_side() {
    // Direction-flipped mirror of the first case, same color: White king h5
    // (the high-file end this time), White pawn f5, Black pawn e5 (just
    // double-pushed from e7, ep target e6), Black rook a5 (the low-file end).
    // Capturing en passant (f5xe6) removes the e5 pawn and opens the whole
    // rank from the rook up to the king. A fix that only scans from the king
    // toward higher files (the shape of the first case, king on the low end)
    // would miss this one, where the exposed ray runs the other way.
    let board = Board::try_from_fen("8/8/8/r3pP1K/8/8/8/8 w - e6 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::F5, Square::E6));
}

#[test]
fn king_moving_off_a_sliders_ray_is_still_in_check() {
    // King on e1 stepping to e2 does not escape a rook's e-file check: e2 is
    // still on the file. This is the case copy-make gets right "for free":
    // the king's *old* square (e1) is genuinely vacated in the copy, so if the
    // king had instead tried to step *behind itself* along a rank/file/
    // diagonal it was blocking, the ray would correctly continue through.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E1, Square::E2));
}

#[test]
fn en_passant_capturing_the_checking_pawn_resolves_check() {
    // White king e4, in check from a Black pawn that just double-pushed d7-d5
    // (d5 attacks e4). Capturing it en passant (e5xd6) removes the checker,
    // but the destination square, d6, is neither the checker's own square
    // (d5) nor between it and the king: a check-response mask built only from
    // the checker's and king's squares would wrongly call this illegal.
    let board = Board::try_from_fen("k7/8/8/3pP3/4K3/8/8/8 w - d6 0 1").expect("valid FEN");
    assert!(in_check(&board, board.side_to_move()));
    let moves = legal_moves(&board);
    assert!(contains(&moves, Square::E5, Square::D6));
}

#[test]
fn checkmate_has_no_legal_moves() {
    // Fool's mate.
    let board =
        Board::try_from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
            .expect("valid FEN");
    assert!(in_check(&board, board.side_to_move()));
    assert!(legal_moves(&board).is_empty());
}

#[test]
fn stalemate_has_no_legal_moves_and_is_not_check() {
    // Classic king-and-queen-vs-king stalemate: Black king a8 has no legal
    // move and is not in check.
    let board = Board::try_from_fen("k7/8/1Q6/8/8/8/8/1K6 b - - 0 1").expect("valid FEN");
    assert!(!in_check(&board, board.side_to_move()));
    assert!(legal_moves(&board).is_empty());
}

#[test]
fn double_check_only_the_king_may_move() {
    // White king e1, attacked simultaneously by a rook on the e-file and a
    // bishop on the a5-e1 diagonal, plus a white knight on b3 that *could*
    // capture the bishop (b3-a5 is a legal knight move) if this were only a
    // single check. Since it isn't, the rook's check remains regardless,
    // that capture must still be excluded: no block or capture resolves both
    // checks at once, so every legal move must be a king move.
    let board = Board::try_from_fen("4r3/8/8/b7/8/1N6/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!moves.is_empty());
    assert!(moves.iter().all(|m| m.from() == Square::E1));
    assert!(!contains(&moves, Square::B3, Square::A5));
}
