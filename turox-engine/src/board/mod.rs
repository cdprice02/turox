//! The board: piece placement plus the rest of a chess position's state (side to
//! move, castling rights, en passant target, move clocks).

use crate::types::{
    Bitboard, CastlingRights, Color, ColoredPiece, File, Move, Piece, Rank, Square,
};
use std::fmt;
use std::ops::Index;

pub mod error;
pub mod fen;
pub mod zobrist;

pub use error::InvalidFenError;

/// A chess position.
///
/// `Copy` and ~136 bytes (8 bitboards + a 64-byte mailbox + game state), built for
/// copy-make: `make_move` takes `&self` and returns a new `Board` rather than
/// mutating in place plus an undo stack. No undo-state bugs, and it stays trivially
/// parallel if lazy SMP search happens later.
///
/// The mailbox (`piece_at` in O(1)) exists alongside the bitboards specifically
/// because captures, SEE, and eval all want "what's on this square" far more often
/// than "where are all the knights" — paying 64 bytes to make that O(1) instead of a
/// 6-bitboard scan is worth it. `ColoredPiece` being a single `repr(u8)` enum rather
/// than a `{ color, piece }` struct is what keeps `Option<ColoredPiece>` to 1 byte
/// (a niche in the 12..=255 range) instead of 2, halving this array from 128 bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    by_color: [Bitboard; 2],
    by_piece: [Bitboard; 6],
    mailbox: [Option<ColoredPiece>; 64],
    side_to_move: Color,
    castling: CastlingRights,
    en_passant: Option<Square>,
    halfmove_clock: u8,
    fullmove_number: u16,
}

impl Default for Board {
    /// An empty board, White to move, no castling rights, move 1. Not a legal
    /// position on its own — use `start_pos` or `try_from_fen` for one.
    fn default() -> Self {
        Self {
            by_color: [Bitboard::EMPTY; 2],
            by_piece: [Bitboard::EMPTY; 6],
            mailbox: [None; 64],
            side_to_move: Color::White,
            castling: CastlingRights::NONE,
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
        }
    }
}

impl Board {
    /// The standard chess starting position.
    pub fn start_pos() -> Self {
        let mut board = Self::default();
        const BACK_RANK: [Piece; 8] = [
            Piece::Rook,
            Piece::Knight,
            Piece::Bishop,
            Piece::Queen,
            Piece::King,
            Piece::Bishop,
            Piece::Knight,
            Piece::Rook,
        ];
        for (file, &piece) in File::ALL.iter().zip(BACK_RANK.iter()) {
            board.place(
                Square::new(*file, Rank::R1),
                ColoredPiece::new(Color::White, piece),
            );
            board.place(
                Square::new(*file, Rank::R2),
                ColoredPiece::new(Color::White, Piece::Pawn),
            );
            board.place(
                Square::new(*file, Rank::R7),
                ColoredPiece::new(Color::Black, Piece::Pawn),
            );
            board.place(
                Square::new(*file, Rank::R8),
                ColoredPiece::new(Color::Black, piece),
            );
        }
        board.castling = CastlingRights::ALL;
        board
    }

    /// Places `cp` on `sq`, keeping the mailbox and the bitboards in sync. Does not
    /// check whether `sq` was already occupied — callers that care (e.g. FEN
    /// parsing rejecting overlapping pieces) should check `piece_at` first.
    ///
    /// This is the single path every piece placement should go through, so the
    /// mailbox/bitboard consistency invariant (checked by the `board_consistency`
    /// property test) can't be broken by construction.
    pub fn place(&mut self, sq: Square, cp: ColoredPiece) {
        self.mailbox[sq.index() as usize] = Some(cp);
        self.by_color[cp.color() as usize] |= sq;
        self.by_piece[cp.piece() as usize] |= sq;
    }

    /// Removes and returns whatever was on `sq`, or `None` if it was already empty.
    pub fn remove(&mut self, sq: Square) -> Option<ColoredPiece> {
        let cp = self.mailbox[sq.index() as usize].take()?;
        self.by_color[cp.color() as usize] -= sq;
        self.by_piece[cp.piece() as usize] -= sq;
        Some(cp)
    }

    /// O(1) lookup of whatever is on `sq`, via the mailbox.
    pub fn piece_at(&self, sq: Square) -> Option<ColoredPiece> {
        self.mailbox[sq.index() as usize]
    }

    /// All pieces of a given color and kind.
    pub fn pieces(&self, color: Color, piece: Piece) -> Bitboard {
        self[piece] & self[color]
    }

    /// Every occupied square, regardless of color or piece kind.
    pub fn occupied(&self) -> Bitboard {
        self[Color::White] | self[Color::Black]
    }

    /// Every unoccupied square.
    pub fn empty(&self) -> Bitboard {
        !self.occupied()
    }

    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn castling_rights(&self) -> CastlingRights {
        self.castling
    }

    pub fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    pub fn halfmove_clock(&self) -> u8 {
        self.halfmove_clock
    }

    pub fn fullmove_number(&self) -> u16 {
        self.fullmove_number
    }

    /// Applies `m` to a copy of this position and returns the result (copy-make;
    /// see the struct docs). Stubbed pending move generation: this needs
    /// `Bitboard`'s scanning/shift primitives to be implemented first, and its own
    /// logic (captures, en passant, castling rook movement, promotion, clock
    /// resets) is the subject of the move-generation change.
    #[allow(unused_variables)]
    pub fn make_move(&self, m: Move) -> Board {
        todo!()
    }
}

impl Index<Color> for Board {
    type Output = Bitboard;
    fn index(&self, color: Color) -> &Bitboard {
        &self.by_color[color as usize]
    }
}

impl Index<Piece> for Board {
    type Output = Bitboard;
    fn index(&self, piece: Piece) -> &Bitboard {
        &self.by_piece[piece as usize]
    }
}

impl fmt::Debug for Board {
    /// A single 8x8 grid built from the mailbox. Deliberately not `#[derive(Debug)]`
    /// dumping all 8 bitboard fields separately — this is the representation
    /// actually useful for debugging a position, and (unlike `Bitboard`'s own
    /// `Debug`) doesn't depend on any of the still-unimplemented `Bitboard` methods,
    /// so it works today.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let sq = Square::ALL[rank * 8 + file];
                let ch = self.piece_at(sq).map_or('.', ColoredPiece::to_fen);
                write!(f, "{ch} ")?;
            }
            writeln!(f)?;
        }
        write!(f, "  a b c d e f g h")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pos_places_expected_piece_counts() {
        let board = Board::start_pos();
        for sq in Square::ALL {
            let _ = board.piece_at(sq); // mailbox lookup never panics
        }
        let mut counts = [0u32; 12];
        for sq in Square::ALL {
            if let Some(cp) = board.piece_at(sq) {
                counts[cp as usize] += 1;
            }
        }
        assert_eq!(counts[ColoredPiece::WhitePawn as usize], 8);
        assert_eq!(counts[ColoredPiece::BlackPawn as usize], 8);
        assert_eq!(counts[ColoredPiece::WhiteKing as usize], 1);
        assert_eq!(counts[ColoredPiece::BlackKing as usize], 1);
    }

    #[test]
    fn start_pos_side_to_move_and_castling() {
        let board = Board::start_pos();
        assert_eq!(board.side_to_move(), Color::White);
        assert_eq!(board.castling_rights(), CastlingRights::ALL);
        assert_eq!(board.en_passant(), None);
        assert_eq!(board.fullmove_number(), 1);
    }

    #[test]
    fn place_then_piece_at_agrees() {
        let mut board = Board::default();
        board.place(Square::E4, ColoredPiece::WhiteKnight);
        assert_eq!(board.piece_at(Square::E4), Some(ColoredPiece::WhiteKnight));
        assert_eq!(board.piece_at(Square::E5), None);
    }

    #[test]
    fn remove_clears_mailbox() {
        let mut board = Board::default();
        board.place(Square::E4, ColoredPiece::WhiteKnight);
        assert_eq!(board.remove(Square::E4), Some(ColoredPiece::WhiteKnight));
        assert_eq!(board.piece_at(Square::E4), None);
        assert_eq!(board.remove(Square::E4), None);
    }

    #[test]
    fn start_pos_occupied_and_empty() {
        let board = Board::start_pos();
        assert_eq!(board.occupied().count(), 32);
        assert_eq!(board.occupied() & board.empty(), Bitboard::EMPTY);
    }

    #[test]
    fn start_pos_one_king_per_side() {
        let board = Board::start_pos();
        for color in Color::ALL {
            assert_eq!(board.pieces(color, Piece::King).count(), 1);
        }
    }

    #[test]
    fn every_color_piece_pair_is_disjoint_and_covers_occupied() {
        let board = Board::start_pos();
        let mut union = Bitboard::EMPTY;
        for color in Color::ALL {
            for piece in Piece::ALL {
                let bb = board.pieces(color, piece);
                assert_eq!(
                    bb & union,
                    Bitboard::EMPTY,
                    "overlap for {color:?}/{piece:?}"
                );
                union |= bb;
            }
        }
        assert_eq!(union, board.occupied());
    }
}
