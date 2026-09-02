//! Property tests for `move_gen::pseudo_legal`.
//!
//! `pawn_moves`, `knight_moves`, `king_moves`, and `slider_moves` each get a
//! proptest against an independent naive reference built only from
//! `Square::offset` stepping and `Board` accessors, never from `Bitboard`'s
//! shift primitives, `tables`, or `magic`, matching the discipline in
//! `tests/attacks_props.rs` and `tests/magic_props.rs`. Move
//! lists are compared as sorted `(from, to, flags)` triples, since `Move` has
//! no `Ord` and generation order isn't part of the contract.
//!
//! `castling_moves` has no proptest here: legality hinges on
//! `move_gen::attacks` (already independently tested), so what's worth
//! pinning down is the specific {Color}x{kingside,queenside} mapping, which
//! is a concrete-test job. All concrete tests (double-push blocking, en
//! passant, promotion, castling, the full aggregate) live in
//! `tests/pseudo_legal.rs`; this file is proptest only.

mod common;

use common::any_board;
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::{king_moves, knight_moves, pawn_moves, slider_moves};
use turox_engine::{Color, Move, MoveFlags, Piece, Rank};

const fn move_key(m: Move) -> (u8, u8, MoveFlags) {
    (m.from().to_u8(), m.to().to_u8(), m.flags())
}

fn sorted_keys(list: &MoveList) -> Vec<(u8, u8, MoveFlags)> {
    let mut keys: Vec<_> = list.iter().map(|&m| move_key(m)).collect();
    keys.sort_unstable();
    keys
}

fn sorted_naive_keys(moves: Vec<Move>) -> Vec<(u8, u8, MoveFlags)> {
    let mut keys: Vec<_> = moves.into_iter().map(move_key).collect();
    keys.sort_unstable();
    keys
}

// ---- Naive reference generators ----
//
// Deliberately independent of `Bitboard`'s pawn/knight/slider primitives:
// built from `Square::offset` stepping only, same discipline as
// `tests/magic_props.rs`/`tests/attacks_props.rs`.

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
