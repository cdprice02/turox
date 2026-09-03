//! Concrete scenario tests for `move_gen::pseudo_legal`'s public API.
//!
//! `tests/pseudo_legal_props.rs` has the exhaustive-against-independent-
//! reference coverage for `pawn_moves`/`knight_moves`/`king_moves`/
//! `slider_moves`; these are hand-picked positions for the cases a naive
//! reference (built the same way, with the same offset-stepping technique)
//! wouldn't independently catch a mistake in: pawn double-push blocking, en
//! passant, promotion, and `castling_moves`'s per-corner mapping
//! specifically, which hinges on `move_gen::attacks` (already
//! independently tested elsewhere) rather than on move-stepping at all.

use turox_engine::board::Board;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::{castling_moves, pawn_moves, pseudo_legal_moves};
use turox_engine::{MoveFlags, Square};

fn contains(list: &MoveList, from: Square, to: Square, flags: MoveFlags) -> bool {
    list.iter()
        .any(|&m| m.from() == from && m.to() == to && m.flags() == flags)
}

/// Quiet promotion flag for the piece a plain (non-underpromotion-specific)
/// promotion index selects, paired with its capturing counterpart.
const PROMO_QUIET: [MoveFlags; 4] = [
    MoveFlags::PromoteKnight,
    MoveFlags::PromoteBishop,
    MoveFlags::PromoteRook,
    MoveFlags::PromoteQueen,
];
const PROMO_CAPTURE: [MoveFlags; 4] = [
    MoveFlags::PromoteCaptureKnight,
    MoveFlags::PromoteCaptureBishop,
    MoveFlags::PromoteCaptureRook,
    MoveFlags::PromoteCaptureQueen,
];

// ---- pawn: double push blocking ----

#[test]
fn white_double_push_blocked_by_piece_on_intermediate_square() {
    // e3 occupied blocks *both* the single push to e3 and the double push to
    // e4, a single shift-by-16 for the double push, skipping the
    // intermediate-square check, would miss that e3 itself is occupied.
    let board = Board::try_from_fen("8/8/8/8/8/4n3/4P3/8 w - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(!contains(&list, Square::E2, Square::E3, MoveFlags::Quiet));
    assert!(!contains(
        &list,
        Square::E2,
        Square::E4,
        MoveFlags::DoublePawnPush
    ));
}

#[test]
fn white_double_push_blocked_by_piece_on_far_square_only() {
    // e4 occupied, e3 empty: the single push is still legal, only the double
    // push is blocked.
    let board = Board::try_from_fen("8/8/8/8/4n3/8/4P3/8 w - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(contains(&list, Square::E2, Square::E3, MoveFlags::Quiet));
    assert!(!contains(
        &list,
        Square::E2,
        Square::E4,
        MoveFlags::DoublePawnPush
    ));
}

#[test]
fn black_double_push_blocked_by_piece_on_intermediate_square() {
    let board = Board::try_from_fen("8/4p3/4N3/8/8/8/8/8 b - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(!contains(&list, Square::E7, Square::E6, MoveFlags::Quiet));
    assert!(!contains(
        &list,
        Square::E7,
        Square::E5,
        MoveFlags::DoublePawnPush
    ));
}

#[test]
fn black_double_push_blocked_by_piece_on_far_square_only() {
    let board = Board::try_from_fen("8/4p3/8/4N3/8/8/8/8 b - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(contains(&list, Square::E7, Square::E6, MoveFlags::Quiet));
    assert!(!contains(
        &list,
        Square::E7,
        Square::E5,
        MoveFlags::DoublePawnPush
    ));
}

// ---- pawn: en passant ----

#[test]
fn white_en_passant_capture_is_generated() {
    let board = Board::try_from_fen("8/8/8/3pP3/8/8/8/8 w - d6 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E5,
        Square::D6,
        MoveFlags::EnPassant
    ));
}

#[test]
fn black_en_passant_capture_is_generated() {
    let board = Board::try_from_fen("8/8/8/8/3pP3/8/8/8 b - e3 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::D4,
        Square::E3,
        MoveFlags::EnPassant
    ));
}

#[test]
fn en_passant_target_set_but_no_pawn_can_reach_it_generates_nothing() {
    // ep target a6 set, but the only white pawn (e5) isn't adjacent to the
    // a-file: no EnPassant move should appear anywhere in the list.
    let board = Board::try_from_fen("8/8/8/4P3/8/8/8/8 w - a6 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    assert!(!list.iter().any(|m| m.flags() == MoveFlags::EnPassant));
}

// ---- pawn: promotion ----

#[test]
fn white_quiet_promotion_generates_all_four_pieces() {
    let board = Board::try_from_fen("8/4P3/8/8/8/8/8/8 w - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    for flag in PROMO_QUIET {
        assert!(
            contains(&list, Square::E7, Square::E8, flag),
            "missing {flag:?}"
        );
    }
    assert_eq!(list.len(), 4);
}

#[test]
fn white_capturing_promotion_generates_all_four_pieces_alongside_quiet() {
    // e8 empty (quiet promotion available) and d8 holds a capturable knight
    // (capturing promotion available): 4 + 4 = 8 moves total from this pawn.
    let board = Board::try_from_fen("3n4/4P3/8/8/8/8/8/8 w - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    for flag in PROMO_QUIET {
        assert!(
            contains(&list, Square::E7, Square::E8, flag),
            "missing quiet {flag:?}"
        );
    }
    for flag in PROMO_CAPTURE {
        assert!(
            contains(&list, Square::E7, Square::D8, flag),
            "missing capture {flag:?}"
        );
    }
    assert_eq!(list.len(), 8);
}

#[test]
fn black_quiet_promotion_generates_all_four_pieces() {
    let board = Board::try_from_fen("8/8/8/8/8/8/4p3/8 b - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    for flag in PROMO_QUIET {
        assert!(
            contains(&list, Square::E2, Square::E1, flag),
            "missing {flag:?}"
        );
    }
    assert_eq!(list.len(), 4);
}

#[test]
fn black_capturing_promotion_generates_all_four_pieces_alongside_quiet() {
    let board = Board::try_from_fen("8/8/8/8/8/8/4p3/3N4 b - - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    pawn_moves(&board, &mut list);
    for flag in PROMO_QUIET {
        assert!(contains(&list, Square::E2, Square::E1, flag));
    }
    for flag in PROMO_CAPTURE {
        assert!(contains(&list, Square::E2, Square::D1, flag));
    }
    assert_eq!(list.len(), 8);
}

// ---- castling ----
//
// Concrete FEN tests, not a proptest: legality here hinges on
// `move_gen::attacks` (already independently tested), so what's worth pinning
// down is the per-corner rook mapping specifically.

const CASTLE_BASE_WHITE: &str = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
const CASTLE_BASE_BLACK: &str = "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1";

#[test]
fn white_both_castles_available_on_an_empty_clear_board() {
    let board = Board::try_from_fen(CASTLE_BASE_WHITE).expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn black_both_castles_available_on_an_empty_clear_board() {
    let board = Board::try_from_fen(CASTLE_BASE_BLACK).expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E8,
        Square::G8,
        MoveFlags::KingCastle
    ));
    assert!(contains(
        &list,
        Square::E8,
        Square::C8,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn kingside_castle_blocked_by_occupied_transit_square() {
    // Knight on f1 itself (not f2, a transit-square blocker has to sit on the
    // rank the king/rook actually cross).
    let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3KN1R w KQkq - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(!contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn queenside_castle_blocked_by_occupied_transit_square() {
    let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R2NK2R w KQkq - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(!contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn kingside_castle_blocked_by_attacked_f1() {
    let board = Board::try_from_fen("4k3/5r2/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(!contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
}

#[test]
fn kingside_castle_blocked_by_attacked_g1() {
    let board = Board::try_from_fen("4k3/6r1/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(!contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
}

#[test]
fn both_castles_blocked_by_attacked_e1() {
    let board = Board::try_from_fen("4k3/4r3/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(!contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(!contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn queenside_castle_blocked_by_attacked_d1() {
    let board = Board::try_from_fen("4k3/3r4/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(!contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn queenside_castle_blocked_by_attacked_c1() {
    let board = Board::try_from_fen("4k3/2r5/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E1,
        Square::G1,
        MoveFlags::KingCastle
    ));
    assert!(!contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn queenside_castle_is_still_legal_when_b1_is_attacked() {
    // b1 must be *empty* (the rook crosses it) but need not be *safe* (the
    // king never does), the one row of the castling table that differs from
    // every other transit square.
    let board = Board::try_from_fen("4k3/1r6/8/8/8/8/8/R3K2R w KQ - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E1,
        Square::C1,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn queenside_castle_is_still_legal_when_b8_is_attacked() {
    // Rights are "q" only: this board has no black rook on h8, so a claimed
    // "k" right here would itself be an invalid FEN for what's on the board.
    let board = Board::try_from_fen("r3k3/8/8/8/8/8/1R6/4K3 b q - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(contains(
        &list,
        Square::E8,
        Square::C8,
        MoveFlags::QueenCastle
    ));
}

#[test]
fn no_castling_moves_when_rights_are_absent() {
    let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w kq - 0 1").expect("valid FEN");
    let mut list = MoveList::new();
    castling_moves(&board, &mut list);
    assert!(list.is_empty());
}

// ---- pseudo_legal_moves: the full aggregate ----

#[test]
fn start_pos_has_exactly_twenty_pseudo_legal_moves() {
    let board = Board::start_pos();
    let mut list = MoveList::new();
    pseudo_legal_moves(&board, &mut list);
    assert_eq!(list.len(), 20);
}
