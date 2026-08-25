//! Property and concrete tests for `move_gen::pseudo_legal`.
//!
//! `pawn_moves`, `knight_moves`, `king_moves`, and `slider_moves` each get a
//! proptest against an independent naive reference built only from
//! `Square::offset` stepping and `Board` accessors — never from
//! `Bitboard::pawn_pushes`/`pawn_attacks_*`/`tables`/`magic`, matching the
//! discipline in `tests/attacks_props.rs` and `tests/tables_props.rs`. Move
//! lists are compared as sorted `(from, to, flags)` triples, since `Move` has
//! no `Ord` and generation order isn't part of the contract.
//!
//! `castling_moves` gets concrete FEN tests instead of a proptest: legality
//! hinges on `move_gen::attacks` (already independently tested), so what's
//! worth pinning down here is the specific {Color}x{kingside,queenside}
//! mapping, not attack correctness — exactly the shape flagged in `CLAUDE.md`
//! as a repeat source of bugs in this crate.

mod common;

use common::any_board;
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::{
    castling_moves, king_moves, knight_moves, pawn_moves, pseudo_legal_moves, slider_moves,
};
use turox_engine::{Color, Move, MoveFlags, Piece, Rank, Square};

fn move_key(m: Move) -> (u8, u8, u8) {
    (m.from().index(), m.to().index(), m.flags() as u8)
}

fn sorted_keys(list: &MoveList) -> Vec<(u8, u8, u8)> {
    let mut keys: Vec<_> = list.iter().map(|&m| move_key(m)).collect();
    keys.sort_unstable();
    keys
}

fn sorted_naive_keys(moves: Vec<Move>) -> Vec<(u8, u8, u8)> {
    let mut keys: Vec<_> = moves.into_iter().map(move_key).collect();
    keys.sort_unstable();
    keys
}

fn contains(list: &MoveList, from: Square, to: Square, flags: MoveFlags) -> bool {
    list.iter()
        .any(|&m| m.from() == from && m.to() == to && m.flags() == flags)
}

// ---- Naive reference generators ----
//
// Deliberately independent of `Bitboard`'s pawn/knight/slider primitives:
// built from `Square::offset` stepping only, same discipline as
// `tests/tables_props.rs`/`tests/magic_props.rs`/`tests/attacks_props.rs`.

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
const ROOK_DIRS: [(i8, i8); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

fn naive_knight_moves(board: &Board) -> Vec<Move> {
    let us = board.side_to_move();
    let mut moves = Vec::new();
    for from in board.pieces(us, Piece::Knight) {
        for (df, dr) in KNIGHT_DELTAS {
            let Some(to) = from.offset(df, dr) else {
                continue;
            };
            match board.piece_at(to) {
                Some(cp) if cp.color() == us => {}
                Some(_) => moves.push(Move::new(from, to, MoveFlags::Capture)),
                None => moves.push(Move::new(from, to, MoveFlags::Quiet)),
            }
        }
    }
    moves
}

fn naive_king_moves(board: &Board) -> Vec<Move> {
    let us = board.side_to_move();
    let mut moves = Vec::new();
    for from in board.pieces(us, Piece::King) {
        for df in -1i8..=1 {
            for dr in -1i8..=1 {
                if df == 0 && dr == 0 {
                    continue;
                }
                let Some(to) = from.offset(df, dr) else {
                    continue;
                };
                match board.piece_at(to) {
                    Some(cp) if cp.color() == us => {}
                    Some(_) => moves.push(Move::new(from, to, MoveFlags::Capture)),
                    None => moves.push(Move::new(from, to, MoveFlags::Quiet)),
                }
            }
        }
    }
    moves
}

fn naive_slider_moves_for(board: &Board, piece: Piece, dirs: &[(i8, i8)]) -> Vec<Move> {
    let us = board.side_to_move();
    let mut moves = Vec::new();
    for from in board.pieces(us, piece) {
        for &(df, dr) in dirs {
            let mut current = from;
            while let Some(to) = current.offset(df, dr) {
                match board.piece_at(to) {
                    Some(cp) if cp.color() == us => break,
                    Some(_) => {
                        moves.push(Move::new(from, to, MoveFlags::Capture));
                        break;
                    }
                    None => {
                        moves.push(Move::new(from, to, MoveFlags::Quiet));
                        current = to;
                    }
                }
            }
        }
    }
    moves
}

fn naive_slider_moves(board: &Board) -> Vec<Move> {
    let mut moves = naive_slider_moves_for(board, Piece::Bishop, &BISHOP_DIRS);
    moves.extend(naive_slider_moves_for(board, Piece::Rook, &ROOK_DIRS));
    moves.extend(naive_slider_moves_for(board, Piece::Queen, &ROOK_DIRS));
    moves.extend(naive_slider_moves_for(board, Piece::Queen, &BISHOP_DIRS));
    moves
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

fn naive_pawn_moves(board: &Board) -> Vec<Move> {
    let us = board.side_to_move();
    let them = us.flip();
    let forward: i8 = match us {
        Color::White => 1,
        Color::Black => -1,
    };
    let start_rank = match us {
        Color::White => Rank::R2,
        Color::Black => Rank::R7,
    };
    let promo_rank = match us {
        Color::White => Rank::R8,
        Color::Black => Rank::R1,
    };
    let ep = board.en_passant();

    let mut moves = Vec::new();
    for from in board.pieces(us, Piece::Pawn) {
        if let Some(one) = from.offset(0, forward) {
            if board.piece_at(one).is_none() {
                if one.rank() == promo_rank {
                    for &flag in &PROMO_QUIET {
                        moves.push(Move::new(from, one, flag));
                    }
                } else {
                    moves.push(Move::new(from, one, MoveFlags::Quiet));
                    if from.rank() == start_rank {
                        if let Some(two) = from.offset(0, 2 * forward) {
                            if board.piece_at(two).is_none() {
                                moves.push(Move::new(from, two, MoveFlags::DoublePawnPush));
                            }
                        }
                    }
                }
            }
        }

        for df in [-1i8, 1i8] {
            let Some(to) = from.offset(df, forward) else {
                continue;
            };
            if let Some(cp) = board.piece_at(to) {
                if cp.color() == them {
                    if to.rank() == promo_rank {
                        for &flag in &PROMO_CAPTURE {
                            moves.push(Move::new(from, to, flag));
                        }
                    } else {
                        moves.push(Move::new(from, to, MoveFlags::Capture));
                    }
                }
            } else if Some(to) == ep {
                moves.push(Move::new(from, to, MoveFlags::EnPassant));
            }
        }
    }
    moves
}

proptest! {
    #[test]
    fn knight_moves_matches_naive(board in any_board()) {
        let mut list = MoveList::new();
        knight_moves(&board, &mut list);
        prop_assert_eq!(sorted_keys(&list), sorted_naive_keys(naive_knight_moves(&board)));
    }

    #[test]
    fn king_moves_matches_naive(board in any_board()) {
        let mut list = MoveList::new();
        king_moves(&board, &mut list);
        prop_assert_eq!(sorted_keys(&list), sorted_naive_keys(naive_king_moves(&board)));
    }

    #[test]
    fn slider_moves_matches_naive(board in any_board()) {
        let mut list = MoveList::new();
        slider_moves(&board, &mut list);
        prop_assert_eq!(sorted_keys(&list), sorted_naive_keys(naive_slider_moves(&board)));
    }

    #[test]
    fn pawn_moves_matches_naive(board in any_board()) {
        let mut list = MoveList::new();
        pawn_moves(&board, &mut list);
        prop_assert_eq!(sorted_keys(&list), sorted_naive_keys(naive_pawn_moves(&board)));
    }
}

// ---- pawn: double push blocking ----

#[test]
fn white_double_push_blocked_by_piece_on_intermediate_square() {
    // e3 occupied blocks *both* the single push to e3 and the double push to
    // e4 — a single shift-by-16 for the double push, skipping the
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
// down is the {Color}x{kingside,queenside} mapping specifically.

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
    let board = Board::try_from_fen("r3k2r/8/8/8/8/8/5N2/R3K2R w KQkq - 0 1").expect("valid FEN");
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
    let board = Board::try_from_fen("r3k2r/8/8/8/8/8/3N4/R3K2R w KQkq - 0 1").expect("valid FEN");
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
    // king never does) — the one row of the castling table that differs from
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
    let board = Board::try_from_fen("r3k3/8/8/8/8/8/1R6/4K3 b kq - 0 1").expect("valid FEN");
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
