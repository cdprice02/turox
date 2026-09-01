//! `Piece` (color-independent) and `ColoredPiece` (a specific piece of a
//! specific color).

use super::color::Color;

/// A piece kind, independent of color.
#[allow(missing_docs)] // variant names are the doc
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Piece {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl Piece {
    /// Every piece kind.
    pub const ALL: [Self; 6] = [
        Self::Pawn,
        Self::Knight,
        Self::Bishop,
        Self::Rook,
        Self::Queen,
        Self::King,
    ];
}

/// A piece of a specific color, packed as a single `repr(u8)` enum rather than a
/// `{ color, piece }` struct.
///
/// The struct form is 2 bytes with no niche, so `[Option<ColoredPiece>; 64]` (the
/// board's mailbox) would cost 128 bytes. This enum form gives `Option<ColoredPiece>`
/// a 1-byte niche, halving the mailbox to 64 bytes.
#[allow(missing_docs)] // variant names are the doc
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColoredPiece {
    WhitePawn = 0,
    WhiteKnight = 1,
    WhiteBishop = 2,
    WhiteRook = 3,
    WhiteQueen = 4,
    WhiteKing = 5,
    BlackPawn = 6,
    BlackKnight = 7,
    BlackBishop = 8,
    BlackRook = 9,
    BlackQueen = 10,
    BlackKing = 11,
}

impl ColoredPiece {
    /// Build a `ColoredPiece` from its color and piece kind.
    #[must_use]
    pub const fn new(color: Color, piece: Piece) -> Self {
        match (color, piece) {
            (Color::White, Piece::Pawn) => Self::WhitePawn,
            (Color::White, Piece::Knight) => Self::WhiteKnight,
            (Color::White, Piece::Bishop) => Self::WhiteBishop,
            (Color::White, Piece::Rook) => Self::WhiteRook,
            (Color::White, Piece::Queen) => Self::WhiteQueen,
            (Color::White, Piece::King) => Self::WhiteKing,
            (Color::Black, Piece::Pawn) => Self::BlackPawn,
            (Color::Black, Piece::Knight) => Self::BlackKnight,
            (Color::Black, Piece::Bishop) => Self::BlackBishop,
            (Color::Black, Piece::Rook) => Self::BlackRook,
            (Color::Black, Piece::Queen) => Self::BlackQueen,
            (Color::Black, Piece::King) => Self::BlackKing,
        }
    }

    /// This piece's color.
    #[must_use]
    pub const fn color(self) -> Color {
        if (self as u8) < 6 {
            Color::White
        } else {
            Color::Black
        }
    }

    /// This piece's kind, independent of color.
    #[must_use]
    pub const fn piece(self) -> Piece {
        const PIECES: [Piece; 6] = [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
            Piece::King,
        ];
        PIECES[(self as u8 % 6) as usize]
    }

    /// Parse a FEN piece character (e.g. `'P'`, `'n'`) into a `ColoredPiece`.
    /// `is_ascii_uppercase`, not `is_uppercase`: FEN is ASCII-only, and
    /// `is_uppercase` isn't `const` (it's Unicode-aware, which is both wrong
    /// for FEN and unavailable in a `const fn`).
    #[must_use]
    pub const fn try_from_fen(c: char) -> Option<Self> {
        let color = if c.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let piece = match c.to_ascii_lowercase() {
            'p' => Piece::Pawn,
            'n' => Piece::Knight,
            'b' => Piece::Bishop,
            'r' => Piece::Rook,
            'q' => Piece::Queen,
            'k' => Piece::King,
            _ => return None,
        };
        Some(Self::new(color, piece))
    }

    /// Convert a `ColoredPiece` into its FEN character.
    #[must_use]
    pub const fn to_fen(self) -> char {
        match self {
            Self::WhitePawn => 'P',
            Self::WhiteKnight => 'N',
            Self::WhiteBishop => 'B',
            Self::WhiteRook => 'R',
            Self::WhiteQueen => 'Q',
            Self::WhiteKing => 'K',
            Self::BlackPawn => 'p',
            Self::BlackKnight => 'n',
            Self::BlackBishop => 'b',
            Self::BlackRook => 'r',
            Self::BlackQueen => 'q',
            Self::BlackKing => 'k',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_round_trips_through_color_and_piece() {
        for &color in &Color::ALL {
            for &piece in &Piece::ALL {
                let cp = ColoredPiece::new(color, piece);
                assert_eq!(cp.color(), color);
                assert_eq!(cp.piece(), piece);
            }
        }
    }

    #[test]
    fn fen_char_round_trips() {
        for &color in &Color::ALL {
            for &piece in &Piece::ALL {
                let cp = ColoredPiece::new(color, piece);
                assert_eq!(ColoredPiece::try_from_fen(cp.to_fen()), Some(cp));
            }
        }
    }
}
