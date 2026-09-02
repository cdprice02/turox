//! `Piece` (color-independent) and `ColoredPiece` (a specific piece of a
//! specific color).

use super::color::Color;
use turox_macros::Ordinal;

/// A piece kind, independent of color.
#[allow(missing_docs, reason = "variant names are the doc")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ordinal)]
pub enum Piece {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

/// A piece of a specific color, packed as a single `repr(u8)` enum rather than a
/// `{ color, piece }` struct.
///
/// The struct form is 2 bytes with no niche, so `[Option<ColoredPiece>; 64]` (the
/// board's mailbox) would cost 128 bytes. This enum form gives `Option<ColoredPiece>`
/// a 1-byte niche, halving the mailbox to 64 bytes.
#[allow(missing_docs, reason = "variant names are the doc")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ordinal)]
pub enum ColoredPiece {
    // Declaration order matters: `zobrist.rs` and `Board`'s piece-count test
    // both index a 12-entry table by `.index()`, relying on this exact dense
    // White-then-Black ordering. Reordering these variants would desync that
    // indexing; `colored_piece_numbering_is_color_major` below and the
    // tables themselves are what catch it, not a discriminant.
    WhitePawn,
    WhiteKnight,
    WhiteBishop,
    WhiteRook,
    WhiteQueen,
    WhiteKing,
    BlackPawn,
    BlackKnight,
    BlackBishop,
    BlackRook,
    BlackQueen,
    BlackKing,
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
        match self {
            Self::WhitePawn
            | Self::WhiteKnight
            | Self::WhiteBishop
            | Self::WhiteRook
            | Self::WhiteQueen
            | Self::WhiteKing => Color::White,
            Self::BlackPawn
            | Self::BlackKnight
            | Self::BlackBishop
            | Self::BlackRook
            | Self::BlackQueen
            | Self::BlackKing => Color::Black,
        }
    }

    /// This piece's kind, independent of color.
    #[must_use]
    pub const fn piece(self) -> Piece {
        match self {
            Self::WhitePawn | Self::BlackPawn => Piece::Pawn,
            Self::WhiteKnight | Self::BlackKnight => Piece::Knight,
            Self::WhiteBishop | Self::BlackBishop => Piece::Bishop,
            Self::WhiteRook | Self::BlackRook => Piece::Rook,
            Self::WhiteQueen | Self::BlackQueen => Piece::Queen,
            Self::WhiteKing | Self::BlackKing => Piece::King,
        }
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

    /// `to_u8`/`from_u8`, both generated by `#[derive(Ordinal)]`, actually
    /// agree with each other, over every variant.
    #[test]
    fn piece_from_u8_round_trips_with_to_u8() {
        for piece in Piece::ALL {
            assert_eq!(Piece::from_u8(piece.to_u8()), Some(piece));
        }
        assert_eq!(Piece::from_u8(6), None);
    }

    #[test]
    fn colored_piece_from_u8_round_trips_with_to_u8() {
        for cp in ColoredPiece::ALL {
            assert_eq!(ColoredPiece::from_u8(cp.to_u8()), Some(cp));
        }
        assert_eq!(ColoredPiece::from_u8(12), None);
    }

    /// The exact dense White-then-Black numbering `zobrist.rs` and the
    /// mailbox piece-count test both rely on implicitly through `.index()`,
    /// pinned here rather than left to be re-derived by inspection.
    #[test]
    fn colored_piece_numbering_is_color_major() {
        for &color in &Color::ALL {
            for &piece in &Piece::ALL {
                let cp = ColoredPiece::new(color, piece);
                assert_eq!(cp.to_u8(), color.to_u8() * 6 + piece.to_u8());
            }
        }
    }
}
