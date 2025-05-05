use bitboard::Bitboard;
pub use square::{File, Rank, Square};

pub(crate) mod bitboard;
pub mod square;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    White,
    Black,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Board {
    by_color: [Bitboard; 2],
    by_type: [Bitboard; 6],
}

impl Default for Board {
    fn default() -> Self {
        let mut board = Self::new_empty();

        board.set_squares([Square::A1, Square::H1], Color::White, Piece::Rook);
        board.set_squares([Square::B1, Square::G1], Color::White, Piece::Knight);
        board.set_squares([Square::C1, Square::F1], Color::White, Piece::Bishop);
        board.set_square(Square::D1, Color::White, Piece::King);
        board.set_square(Square::E1, Color::White, Piece::Queen);
        board.set_squares(
            [
                Square::A2,
                Square::B2,
                Square::C2,
                Square::D2,
                Square::E2,
                Square::F2,
                Square::G2,
                Square::H2,
            ],
            Color::White,
            Piece::Pawn,
        );

        board.set_squares([Square::A8, Square::H8], Color::White, Piece::Rook);
        board.set_squares([Square::B8, Square::G8], Color::White, Piece::Knight);
        board.set_squares([Square::C8, Square::F8], Color::White, Piece::Bishop);
        board.set_square(Square::D8, Color::White, Piece::King);
        board.set_square(Square::E8, Color::White, Piece::Queen);
        board.set_squares(
            [
                Square::A2,
                Square::B2,
                Square::C2,
                Square::D2,
                Square::E2,
                Square::F2,
                Square::G2,
                Square::H2,
            ],
            Color::White,
            Piece::Pawn,
        );

        board
    }
}

impl Board {
    pub fn new_empty() -> Self {
        Self {
            by_color: [Bitboard::new_empty(), Bitboard::new_empty()],
            by_type: [
                Bitboard::new_empty(),
                Bitboard::new_empty(),
                Bitboard::new_empty(),
                Bitboard::new_empty(),
                Bitboard::new_empty(),
                Bitboard::new_empty(),
            ],
        }
    }

    // #[inline]
    // fn pieces_by_color(&self, color: Color) -> Bitboard {
    //     self.by_color[color as usize]
    // }

    #[inline]
    fn pieces_by_color_mut(&mut self, color: Color) -> &mut Bitboard {
        &mut self.by_color[color as usize]
    }

    // #[inline]
    // fn pieces_by_type(&self, piece: Piece) -> Bitboard {
    //     self.by_type[piece as usize]
    // }

    #[inline]
    fn pieces_by_type_mut(&mut self, piece: Piece) -> &mut Bitboard {
        &mut self.by_type[piece as usize]
    }

    // #[inline]
    // fn pieces_by_color_and_type(&self, color: Color, piece: Piece) -> Bitboard {
    //     self.by_color[color as usize] & self.by_type[piece as usize]
    // }

    fn set_square(&mut self, square: Square, color: Color, piece: Piece) {
        let square = Bitboard::from_rank_file(square.rank(), square.file());
        *self.pieces_by_color_mut(color) |= square;
        *self.pieces_by_type_mut(piece) |= square;
    }

    fn set_squares(
        &mut self,
        square: impl IntoIterator<Item = Square>,
        color: Color,
        piece: Piece,
    ) {
        let mut board = Bitboard::new_empty();
        for sq in square {
            board |= Bitboard::from_rank_file(sq.rank(), sq.file());
        }

        *self.pieces_by_color_mut(color) |= board;
        *self.pieces_by_type_mut(piece) |= board;
    }
}
