//! The board: piece placement plus the rest of a chess position's state (side to
//! move, castling rights, en passant target, move clocks).

use crate::types::{Bitboard, CastlingRights, Color, ColoredPiece, File, Piece, Rank, Square};
use std::fmt;
use std::ops::Index;

pub mod error;
pub mod fen;
mod make_move;
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
/// than "where are all the knights", paying 64 bytes to make that O(1) instead of a
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
    /// position on its own; use `start_pos` or `try_from_fen` for one.
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
    /// check whether `sq` was already occupied; callers that care (e.g. FEN
    /// parsing rejecting overlapping pieces) should check `piece_at` first.
    ///
    /// This is the single path every piece placement should go through, so the
    /// mailbox/bitboard consistency invariant (checked by the `board_consistency`
    /// property test) can't be broken by construction.
    pub fn place(&mut self, sq: Square, cp: ColoredPiece) {
        self.mailbox[sq.index() as usize] = Some(cp);
        self.by_color[cp.color() as usize] = self.by_color[cp.color() as usize].or(sq.bitboard());
        self.by_piece[cp.piece() as usize] = self.by_piece[cp.piece() as usize].or(sq.bitboard());
    }

    /// Builds a full `Board` from a placement-only board (assembled via
    /// `Board::default()` plus repeated `place` calls) and the remaining
    /// position state. The single path every non-placement field goes through:
    /// `try_from_fen` builds its result this way, and so can anything else that
    /// wants a specific side-to-move/castling/en-passant/clock combination
    /// without hand-assembling a FEN string first (test helpers, in particular).
    pub const fn from_parts(
        placement: Board,
        side_to_move: Color,
        castling: CastlingRights,
        en_passant: Option<Square>,
        halfmove_clock: u8,
        fullmove_number: u16,
    ) -> Board {
        Board {
            side_to_move,
            castling,
            en_passant,
            halfmove_clock,
            fullmove_number,
            ..placement
        }
    }

    /// Removes and returns whatever was on `sq`, or `None` if it was already empty.
    pub fn remove(&mut self, sq: Square) -> Option<ColoredPiece> {
        let cp = self.mailbox[sq.index() as usize].take()?;
        self.by_color[cp.color() as usize] =
            self.by_color[cp.color() as usize].and_not(sq.bitboard());
        self.by_piece[cp.piece() as usize] =
            self.by_piece[cp.piece() as usize].and_not(sq.bitboard());
        Some(cp)
    }

    /// O(1) lookup of whatever is on `sq`, via the mailbox.
    pub const fn piece_at(&self, sq: Square) -> Option<ColoredPiece> {
        self.mailbox[sq.index() as usize]
    }

    /// All pieces of a given color and kind.
    pub const fn pieces(&self, color: Color, piece: Piece) -> Bitboard {
        // Indexes `by_piece`/`by_color` directly rather than through `self[piece]`/
        // `self[color]`: the `Index` trait's `index` method isn't `const`.
        self.by_piece[piece as usize].and(self.by_color[color as usize])
    }

    /// Every occupied square, regardless of color or piece kind.
    pub const fn occupied(&self) -> Bitboard {
        self.by_color[Color::White as usize].or(self.by_color[Color::Black as usize])
    }

    /// Every unoccupied square.
    pub const fn empty(&self) -> Bitboard {
        // `self.occupied().not()`, not `!self.occupied()`: the `Not` trait's
        // `not` method isn't `const`, only `Bitboard`'s own inherent `not` is.
        self.occupied().not()
    }

    /// Which color is to move.
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    /// The castling rights still available to either side.
    pub const fn castling_rights(&self) -> CastlingRights {
        self.castling
    }

    /// The square a pawn could capture en passant onto, if any.
    pub const fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    /// Plies since the last pawn move or capture (the fifty-move-rule counter).
    pub const fn halfmove_clock(&self) -> u8 {
        self.halfmove_clock
    }

    /// The full-move number, incrementing after each Black move.
    pub const fn fullmove_number(&self) -> u16 {
        self.fullmove_number
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
    /// dumping all 8 bitboard fields separately: this is the representation
    /// actually useful for debugging a position.
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

/// Every (color, piece) bitboard pair is disjoint and their union is exactly
/// `occupied()`, and the mailbox agrees with the bitboards at every square.
/// Shared by `board::mod`'s own placement tests and, more importantly, by
/// `make_move`'s tests; `place`/`remove` are unit-tested to keep this
/// invariant individually, but that's a claim about them in isolation, not a
/// guarantee that `make_move`'s specific sequence of place/remove calls across
/// all 14 move types preserves it too.
#[cfg(test)]
pub(crate) fn assert_board_is_internally_consistent(board: &Board) {
    let mut union = Bitboard::EMPTY;
    for color in Color::ALL {
        for piece in Piece::ALL {
            let bb = board.pieces(color, piece);
            assert_eq!(
                bb.and(union),
                Bitboard::EMPTY,
                "overlap for {color:?}/{piece:?}"
            );
            union = union.or(bb);
        }
    }
    assert_eq!(union, board.occupied(), "bitboards don't cover occupied()");

    for sq in Square::ALL {
        let via_mailbox = board.piece_at(sq);
        let via_bitboards = Color::ALL.iter().find_map(|&color| {
            Piece::ALL
                .iter()
                .find(|&&piece| board.pieces(color, piece).contains(sq))
                .map(|&piece| ColoredPiece::new(color, piece))
        });
        assert_eq!(
            via_mailbox, via_bitboards,
            "mailbox/bitboard mismatch at {sq:?}"
        );
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
        assert_eq!(board.occupied().and(board.empty()), Bitboard::EMPTY);
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
        assert_board_is_internally_consistent(&Board::start_pos());
    }
}
