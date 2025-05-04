use bitboard::Bitboard;

pub(crate) mod bitboard;

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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    One = 0,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum File {
    A = 0,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Square {
    A1 = 0,
    B1,
    C1,
    D1,
    E1,
    F1,
    G1,
    H1,
    A2,
    B2,
    C2,
    D2,
    E2,
    F2,
    G2,
    H2,
    A3,
    B3,
    C3,
    D3,
    E3,
    F3,
    G3,
    H3,
    A4,
    B4,
    C4,
    D4,
    E4,
    F4,
    G4,
    H4,
    A5,
    B5,
    C5,
    D5,
    E5,
    F5,
    G5,
    H5,
    A6,
    B6,
    C6,
    D6,
    E6,
    F6,
    G6,
    H6,
    A7,
    B7,
    C7,
    D7,
    E7,
    F7,
    G7,
    H7,
    A8,
    B8,
    C8,
    D8,
    E8,
    F8,
    G8,
    H8,
}

impl Square {
    pub fn rank(&self) -> Rank {
        match self {
            Self::A1
            | Self::B1
            | Self::C1
            | Self::D1
            | Self::E1
            | Self::F1
            | Self::G1
            | Self::H1 => Rank::One,
            Self::A2
            | Self::B2
            | Self::C2
            | Self::D2
            | Self::E2
            | Self::F2
            | Self::G2
            | Self::H2 => Rank::Two,
            Self::A3
            | Self::B3
            | Self::C3
            | Self::D3
            | Self::E3
            | Self::F3
            | Self::G3
            | Self::H3 => Rank::Three,
            Self::A4
            | Self::B4
            | Self::C4
            | Self::D4
            | Self::E4
            | Self::F4
            | Self::G4
            | Self::H4 => Rank::Four,
            Self::A5
            | Self::B5
            | Self::C5
            | Self::D5
            | Self::E5
            | Self::F5
            | Self::G5
            | Self::H5 => Rank::Five,
            Self::A6
            | Self::B6
            | Self::C6
            | Self::D6
            | Self::E6
            | Self::F6
            | Self::G6
            | Self::H6 => Rank::Six,
            Self::A7
            | Self::B7
            | Self::C7
            | Self::D7
            | Self::E7
            | Self::F7
            | Self::G7
            | Self::H7 => Rank::Seven,
            Self::A8
            | Self::B8
            | Self::C8
            | Self::D8
            | Self::E8
            | Self::F8
            | Self::G8
            | Self::H8 => Rank::Eight,
        }
    }

    pub fn file(&self) -> File {
        match self {
            Self::A1
            | Self::A2
            | Self::A3
            | Self::A4
            | Self::A5
            | Self::A6
            | Self::A7
            | Self::A8 => File::A,
            Self::B1
            | Self::B2
            | Self::B3
            | Self::B4
            | Self::B5
            | Self::B6
            | Self::B7
            | Self::B8 => File::B,
            Self::C1
            | Self::C2
            | Self::C3
            | Self::C4
            | Self::C5
            | Self::C6
            | Self::C7
            | Self::C8 => File::C,
            Self::D1
            | Self::D2
            | Self::D3
            | Self::D4
            | Self::D5
            | Self::D6
            | Self::D7
            | Self::D8 => File::D,
            Self::E1
            | Self::E2
            | Self::E3
            | Self::E4
            | Self::E5
            | Self::E6
            | Self::E7
            | Self::E8 => File::E,
            Self::F1
            | Self::F2
            | Self::F3
            | Self::F4
            | Self::F5
            | Self::F6
            | Self::F7
            | Self::F8 => File::F,
            Self::G1
            | Self::G2
            | Self::G3
            | Self::G4
            | Self::G5
            | Self::G6
            | Self::G7
            | Self::G8 => File::G,
            Self::H1
            | Self::H2
            | Self::H3
            | Self::H4
            | Self::H5
            | Self::H6
            | Self::H7
            | Self::H8 => File::H,
        }
    }
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

    pub fn pieces_by_color(&self, color: Color) -> Bitboard {
        self.by_color[color as usize]
    }

    pub fn pieces_by_color_mut(&mut self, color: Color) -> &mut Bitboard {
        &mut self.by_color[color as usize]
    }

    pub fn pieces_by_type(&self, piece: Piece) -> Bitboard {
        self.by_type[piece as usize]
    }

    pub fn pieces_by_type_mut(&mut self, piece: Piece) -> &mut Bitboard {
        &mut self.by_type[piece as usize]
    }

    pub fn pieces_by_color_and_type(&self, color: Color, piece: Piece) -> Bitboard {
        self.by_color[color as usize] & self.by_type[piece as usize]
    }

    pub fn set_square(&mut self, square: Square, color: Color, piece: Piece) {
        let square = Bitboard::from_rank_file(square.rank(), square.file());
        *self.pieces_by_color_mut(color) |= square;
        *self.pieces_by_type_mut(piece) |= square;
    }

    pub fn set_squares(
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
