//! Shared proptest strategies for `move_gen`'s integration tests
//! (`attacks_props.rs`, and the pseudolegal/legal test files that follow it).
//! `tests/common/mod.rs` rather than `tests/common.rs`: the `mod.rs` name keeps
//! `cargo`/`nextest` from treating this as its own standalone test binary (which
//! would fail to build; it has no `#[test]`s of its own).
//!
//! `tests/fen_props.rs` already has its own `any_board()`, deliberately not
//! reused here: that one exists to prove FEN round-tripping doesn't care whether
//! a board is chess-legal, so it *shouldn't* constrain placement. Move
//! generation cares a great deal, so this is a separate, stricter strategy.
//!
//! Rust compiles this file fresh into every binary that does `mod common;`,
//! and no single binary uses this whole surface (`square_props.rs` only wants
//! `any_square`, `attacks_props.rs` only wants `any_board`/`any_bitboard`/
//! `any_square`, ...), so `dead_code` fires per-binary for whatever that
//! binary didn't happen to call. That's expected here, not a real problem.
#![allow(dead_code)]

use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::{Bitboard, CastlingRights, Color, ColoredPiece, Piece, Rank, Square};

/// Shared with every other test file that needs an arbitrary square/bitboard
/// (`square_props.rs`, `bitboard_props.rs`, `magic_props.rs`, `tables_props.rs`,
/// `attacks_props.rs`, ...), so the strategy itself isn't duplicated five times.
pub fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
}

/// See `any_square`'s doc.
pub fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

fn any_color() -> impl Strategy<Value = Color> {
    prop_oneof![Just(Color::White), Just(Color::Black)]
}

// Pawns never legally sit on rank 1 or 8 (they'd have already promoted / never
// have been able to reach it as a start square), so they're excluded here and
// kept off both when placing extra pieces below.
fn any_piece() -> impl Strategy<Value = Piece> {
    prop_oneof![
        3 => prop_oneof![
            Just(Piece::Knight),
            Just(Piece::Bishop),
            Just(Piece::Rook),
            Just(Piece::Queen),
        ],
        2 => Just(Piece::Pawn),
    ]
}

fn pawn_rank() -> impl Strategy<Value = Rank> {
    (1u8..7).prop_map(|i| Rank::from_index(i).expect("i in 1..7"))
}

/// A `Board` strategy for move-generation tests: always exactly one king per
/// side (distinct squares), other pieces placed at random with pawns kept off
/// the back ranks, castling rights only ever set when the king and the matching
/// rook actually sit on their home squares. En passant is always `None`; no
/// test in this crate needs a proptest-random ep state; the concrete FEN tests
/// in `pseudo_legal_props.rs` cover that rule directly, and legal move
/// generation produces real ep states from real move sequences, which is a
/// better source of them than manufacturing one here.
pub fn any_board() -> impl Strategy<Value = Board> {
    (
        any_square(),
        any_square(),
        any_color(),
        proptest::collection::vec((any_color(), any_piece(), any_square(), pawn_rank()), 0..20),
    )
        .prop_map(|(white_king, black_king, side_to_move, placements)| {
            let mut board = Board::default();
            if white_king == black_king {
                // Collision on the (rare) shared draw: nudge black's king
                // to a different square deterministically rather than
                // discarding the case (proptest's `prop_filter` would work
                // too, but this keeps every draw a valid test case).
                let black_king = Square::from_index((black_king.index() + 1) % 64).expect("mod 64");
                board.place(white_king, ColoredPiece::WhiteKing);
                board.place(black_king, ColoredPiece::BlackKing);
            } else {
                board.place(white_king, ColoredPiece::WhiteKing);
                board.place(black_king, ColoredPiece::BlackKing);
            }

            for (color, piece, sq, pawn_rank) in placements {
                // A pawn's random square gets its rank overridden to a
                // legal one; non-pawns keep the fully random square. Doing
                // it this way (rather than filtering pawn squares out of
                // `any_square`) keeps file coverage random for pawns too.
                let sq = if piece == Piece::Pawn {
                    Square::new(sq.file(), pawn_rank)
                } else {
                    sq
                };
                if board.piece_at(sq).is_none() {
                    board.place(sq, ColoredPiece::new(color, piece));
                }
            }

            let mut rights = CastlingRights::NONE;
            if board.piece_at(Square::E1) == Some(ColoredPiece::WhiteKing) {
                if board.piece_at(Square::H1) == Some(ColoredPiece::WhiteRook) {
                    rights = rights.with(CastlingRights::WHITE_KINGSIDE);
                }
                if board.piece_at(Square::A1) == Some(ColoredPiece::WhiteRook) {
                    rights = rights.with(CastlingRights::WHITE_QUEENSIDE);
                }
            }
            if board.piece_at(Square::E8) == Some(ColoredPiece::BlackKing) {
                if board.piece_at(Square::H8) == Some(ColoredPiece::BlackRook) {
                    rights = rights.with(CastlingRights::BLACK_KINGSIDE);
                }
                if board.piece_at(Square::A8) == Some(ColoredPiece::BlackRook) {
                    rights = rights.with(CastlingRights::BLACK_QUEENSIDE);
                }
            }

            Board::from_parts(board, side_to_move, rights, None, 0, 1)
        })
}
