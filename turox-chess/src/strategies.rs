//! Proptest strategies over `Board` and its parts.
//!
//! Behind the `strategies` feature, because this is test support rather than
//! part of what the crate is for. Both `turox-chess`'s own property tests and
//! `turox-engine`'s need the same generators, and two copies of them is the
//! duplication this repo keeps getting bitten by.
//!
//! `fen_props.rs` has its own `any_board()`, deliberately not reused here:
//! that one exists to prove FEN round-tripping doesn't care whether a board is
//! chess-legal, so it *shouldn't* constrain placement. Move generation cares a
//! great deal, so this is a separate, stricter strategy.
//!
#![expect(
    clippy::expect_used,
    reason = "`clippy.toml`'s allow-expect-in-tests reaches `#[test]` functions and `#[cfg(test)]` modules, but not plain helpers in an integration test or bench, where a failed fixture should abort the run"
)]

use crate::board::Board;
use crate::move_gen::legal::legal_moves;
use crate::{Bitboard, CastlingRights, Color, ColoredPiece, Move, Piece, Rank, Square};
use proptest::prelude::*;

/// Any square on the board, uniformly.
///
/// Shared with every other test that needs an arbitrary square or bitboard,
/// so the strategy isn't written out once per test binary.
///
/// # Panics
///
/// Never in practice: the index range is `0..64`, which `Square::ALL` covers.
pub fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_u8(i).expect("i in 0..64"))
}

/// See `any_square`'s doc.
pub fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

/// Either colour, uniformly. See `any_square`'s doc for why these are
/// hand-written rather than derived.
pub fn any_color() -> impl Strategy<Value = Color> {
    prop_oneof![Just(Color::White), Just(Color::Black)]
}

/// A piece that `any_board` can safely scatter across a position.
///
/// Pawns never legally sit on rank 1 or 8 (they'd have already promoted, or
/// never have been able to reach it as a start square), so they're excluded
/// here and kept off both when placing extra pieces below. No `King` either:
/// `any_board` places both kings itself, deliberately (exactly one per side,
/// on distinct squares), so a second source of kings here would fight that
/// invariant instead of complementing it.
pub fn any_piece() -> impl Strategy<Value = Piece> {
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

/// Every `Piece` variant, `King` included, uniformly.
///
/// For strategies that don't separately guarantee king placement the way
/// `any_board` does (king-adjacency checks, and FEN round-trip coverage of
/// boards with zero or doubled kings), so `Piece::ALL` isn't silently narrowed
/// to `any_piece`'s pawn-heavy, king-free distribution.
pub fn any_piece_with_king() -> impl Strategy<Value = Piece> {
    prop_oneof![
        Just(Piece::Pawn),
        Just(Piece::Knight),
        Just(Piece::Bishop),
        Just(Piece::Rook),
        Just(Piece::Queen),
        Just(Piece::King),
    ]
}

/// A rank a pawn may legally occupy, so ranks 2 through 7.
fn pawn_rank() -> impl Strategy<Value = Rank> {
    (1u8..7).prop_map(|i| Rank::from_u8(i).expect("i in 1..7"))
}

/// A `Board` strategy for move-generation tests.
///
/// Always exactly one king per side (distinct squares), other pieces placed at
/// random with pawns kept off the back ranks, castling rights only ever set
/// when the king and the matching rook actually sit on their home squares.
///
/// En passant is always `None`; no
/// test in this crate needs a proptest-random ep state; the concrete FEN tests
/// in `pseudo_legal_props.rs` cover that rule directly, and legal move
/// generation produces real ep states from real move sequences, which is a
/// better source of them than manufacturing one here.
///
/// # Panics
///
/// Never: the only fallible step nudges the black king to `(square + 1) % 64`
/// when both kings land on the same square, and that is in range by
/// construction.
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
                let black_king = Square::from_u8((black_king.to_u8() + 1) % 64).expect("mod 64");
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

/// `any_board()`, filtered to positions that actually have a legal move.
///
/// For every test that needs to drive a real move from a random position,
/// rather than risk generating a checkmate or stalemate with nothing to play.
pub fn any_board_with_legal_move() -> impl Strategy<Value = Board> {
    any_board().prop_filter("must have at least one legal move", |board| {
        !legal_moves(board).is_empty()
    })
}

/// `any_board_with_legal_move`, paired with one legal move from that position.
///
/// Uniformly chosen rather than always the first, for tests that need to
/// actually play the move and not just confirm one exists.
pub fn any_board_and_legal_move() -> impl Strategy<Value = (Board, Move)> {
    any_board_with_legal_move().prop_flat_map(|board| {
        let moves: Vec<Move> = legal_moves(&board).iter().copied().collect();
        (Just(board), prop::sample::select(moves))
    })
}

/// The color-swapped mirror of `board`.
///
/// Every White piece becomes the same kind of Black piece on the rank-flipped
/// square, and vice versa. Side to move, castling rights, and the en passant
/// square (if any) are swapped and rank-flipped to match.
///
/// Backs the mirror-symmetry check (`eval_white_pov(b) ==
/// -eval_white_pov(mirrored(b))`), the single test most likely to catch a
/// scrambled White/Black lookup, since it runs against every randomly
/// generated position rather than one hand-written case.
///
/// Test-only and built purely from `Board`'s public API (`piece_at`,
/// `place`, `from_parts`) rather than promoted onto `Board` itself: nothing
/// outside tests needs a whole-position color swap yet, so this stays here
/// until something does.
pub fn mirrored(board: &Board) -> Board {
    let mut placement = Board::default();
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            let mirrored_cp = ColoredPiece::new(cp.color().flip(), cp.piece());
            placement.place(sq.flip_rank(), mirrored_cp);
        }
    }
    Board::from_parts(
        placement,
        board.side_to_move().flip(),
        mirror_castling_rights(board.castling_rights()),
        board.en_passant().map(Square::flip_rank),
        board.halfmove_clock(),
        board.fullmove_number(),
    )
}

/// `rights` with White's and Black's castling rights swapped.
/// `CastlingRights` has no such method itself (nothing outside this mirror
/// needs one); built from `contains`/`with` rather than poking at its
/// internal bit layout.
const fn mirror_castling_rights(rights: CastlingRights) -> CastlingRights {
    let mut swapped = CastlingRights::NONE;
    if rights.contains(CastlingRights::WHITE_KINGSIDE) {
        swapped = swapped.with(CastlingRights::BLACK_KINGSIDE);
    }
    if rights.contains(CastlingRights::WHITE_QUEENSIDE) {
        swapped = swapped.with(CastlingRights::BLACK_QUEENSIDE);
    }
    if rights.contains(CastlingRights::BLACK_KINGSIDE) {
        swapped = swapped.with(CastlingRights::WHITE_KINGSIDE);
    }
    if rights.contains(CastlingRights::BLACK_QUEENSIDE) {
        swapped = swapped.with(CastlingRights::WHITE_QUEENSIDE);
    }
    swapped
}
