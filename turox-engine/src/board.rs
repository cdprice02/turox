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
        board.set_square(Square::D1, Color::White, Piece::Queen);
        board.set_square(Square::E1, Color::White, Piece::King);
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

        board.set_squares([Square::A8, Square::H8], Color::Black, Piece::Rook);
        board.set_squares([Square::B8, Square::G8], Color::Black, Piece::Knight);
        board.set_squares([Square::C8, Square::F8], Color::Black, Piece::Bishop);
        board.set_square(Square::D8, Color::Black, Piece::Queen);
        board.set_square(Square::E8, Color::Black, Piece::King);
        board.set_squares(
            [
                Square::A7,
                Square::B7,
                Square::C7,
                Square::D7,
                Square::E7,
                Square::F7,
                Square::G7,
                Square::H7,
            ],
            Color::Black,
            Piece::Pawn,
        );

        board
    }
}

impl Board {
    pub fn new_empty() -> Self {
        Self {
            by_color: [Bitboard::zeros(), Bitboard::zeros()],
            by_type: [
                Bitboard::zeros(),
                Bitboard::zeros(),
                Bitboard::zeros(),
                Bitboard::zeros(),
                Bitboard::zeros(),
                Bitboard::zeros(),
            ],
        }
    }

    pub fn fen(&self) -> String {
        let mut fen = String::new();
        let mut nonpiece_count = 0;

        for rank in (0..8).rev() {
            let rank = Rank::from(rank);
            for file in 0..8 {
                let file = File::from(file);

                if let Some((color, piece)) = self.colored_piece_at(rank, file) {
                    if nonpiece_count > 0 {
                        fen.push_str(nonpiece_count.to_string().as_str());
                        nonpiece_count = 0;
                    }
                    fen.push(Self::piece_fen(color, piece));
                } else {
                    nonpiece_count += 1;
                }
            }

            if nonpiece_count > 0 {
                fen.push_str(nonpiece_count.to_string().as_str());
                nonpiece_count = 0;
            }

            if rank != Rank::One {
                fen.push('/');
            }
        }

        fen
    }

    fn piece_fen(color: Color, piece: Piece) -> char {
        match (color, piece) {
            (Color::White, Piece::Pawn) => 'P',
            (Color::White, Piece::Knight) => 'N',
            (Color::White, Piece::Bishop) => 'B',
            (Color::White, Piece::Rook) => 'R',
            (Color::White, Piece::Queen) => 'Q',
            (Color::White, Piece::King) => 'K',
            (Color::Black, Piece::Pawn) => 'p',
            (Color::Black, Piece::Knight) => 'n',
            (Color::Black, Piece::Bishop) => 'b',
            (Color::Black, Piece::Rook) => 'r',
            (Color::Black, Piece::Queen) => 'q',
            (Color::Black, Piece::King) => 'k',
        }
    }

    pub fn color_at(&self, rank: Rank, file: File) -> Option<Color> {
        if (self.pieces_by_color(Color::White) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Color::White)
        } else if (self.pieces_by_color(Color::Black) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Color::Black)
        } else {
            None
        }
    }

    pub fn piece_at(&self, rank: Rank, file: File) -> Option<Piece> {
        if (self.pieces_by_type(Piece::Pawn) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::Pawn)
        } else if (self.pieces_by_type(Piece::Knight) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::Knight)
        } else if (self.pieces_by_type(Piece::Bishop) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::Bishop)
        } else if (self.pieces_by_type(Piece::Rook) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::Rook)
        } else if (self.pieces_by_type(Piece::Queen) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::Queen)
        } else if (self.pieces_by_type(Piece::King) & Bitboard::single_from_rank_file(rank, file))
            .nonzero()
        {
            Some(Piece::King)
        } else {
            None
        }
    }

    pub fn colored_piece_at(&self, rank: Rank, file: File) -> Option<(Color, Piece)> {
        if let Some(color) = self.color_at(rank, file) {
            self.piece_at(rank, file).map(|piece| (color, piece))
        } else {
            None
        }
    }

    #[inline]
    fn pieces_by_color(&self, color: Color) -> Bitboard {
        self.by_color[color as usize]
    }

    #[inline]
    fn pieces_by_color_mut(&mut self, color: Color) -> &mut Bitboard {
        &mut self.by_color[color as usize]
    }

    #[inline]
    fn pieces_by_type(&self, piece: Piece) -> Bitboard {
        self.by_type[piece as usize]
    }

    #[inline]
    fn pieces_by_type_mut(&mut self, piece: Piece) -> &mut Bitboard {
        &mut self.by_type[piece as usize]
    }

    #[inline]
    fn pieces_by_color_and_type(&self, color: Color, piece: Piece) -> Bitboard {
        self.by_color[color as usize] & self.by_type[piece as usize]
    }

    fn set_square(&mut self, square: Square, color: Color, piece: Piece) {
        let square = Bitboard::single_from_rank_file(square.rank(), square.file());
        *self.pieces_by_color_mut(color) |= square;
        *self.pieces_by_type_mut(piece) |= square;
    }

    fn set_squares(
        &mut self,
        square: impl IntoIterator<Item = Square>,
        color: Color,
        piece: Piece,
    ) {
        let mut board = Bitboard::zeros();
        for sq in square {
            board |= Bitboard::single_from_rank_file(sq.rank(), sq.file());
        }

        *self.pieces_by_color_mut(color) |= board;
        *self.pieces_by_type_mut(piece) |= board;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod fen {
        use super::{Board, Color, Piece, Square};

        #[test]
        fn empty_board() {
            let board = Board::new_empty();
            assert_eq!(board.fen(), "8/8/8/8/8/8/8/8");
        }

        #[test]
        fn single_white_pawn() {
            let mut board = Board::new_empty();
            board.set_square(Square::C2, Color::White, Piece::Pawn);
            assert_eq!(board.fen(), "8/8/8/8/8/8/2P5/8");
        }

        #[test]
        fn single_black_pawn() {
            let mut board = Board::new_empty();
            board.set_square(Square::C2, Color::Black, Piece::Pawn);
            assert_eq!(board.fen(), "8/8/8/8/8/8/2p5/8");
        }

        #[test]
        fn single_white_piece() {
            let mut board = Board::new_empty();
            board.set_square(Square::E4, Color::White, Piece::Queen);
            assert_eq!(board.fen(), "8/8/8/8/4Q3/8/8/8");
        }

        #[test]
        fn single_black_piece() {
            let mut board = Board::new_empty();
            board.set_square(Square::E4, Color::Black, Piece::Queen);
            assert_eq!(board.fen(), "8/8/8/8/4q3/8/8/8");
        }

        #[test]
        fn initial_board() {
            let board = Board::default();
            assert_eq!(board.fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR");
        }
    }
}
