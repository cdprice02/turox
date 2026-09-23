//! `Piece` (color-independent) and `ColoredPiece` (a specific piece of a
//! specific color).

use super::color::Color;
use turox_macros::Ordinal;

/// A piece kind, independent of color.
#[expect(missing_docs, reason = "variant names are the doc")]
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

impl Piece {
    /// This piece's letter, lowercase.
    ///
    /// The one fact FEN, SAN and UCI all encode. Each of them layers its own
    /// casing rule on top (FEN uppercases for White, SAN always uppercases,
    /// UCI's promotion suffix is always lowercase), so the casing stays at
    /// each format's own call site and only the mapping lives here. Four
    /// copies of this match existed before it did.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Pawn => 'p',
            Self::Knight => 'n',
            Self::Bishop => 'b',
            Self::Rook => 'r',
            Self::Queen => 'q',
            Self::King => 'k',
        }
    }

    /// The inverse of [`Piece::letter`], case-insensitive.
    ///
    /// Case carries meaning in FEN (it is the colour) and none in SAN, so it
    /// is the caller's to read; this answers only which piece the letter names.
    #[must_use]
    pub const fn from_letter(c: char) -> Option<Self> {
        match c.to_ascii_lowercase() {
            'p' => Some(Self::Pawn),
            'n' => Some(Self::Knight),
            'b' => Some(Self::Bishop),
            'r' => Some(Self::Rook),
            'q' => Some(Self::Queen),
            'k' => Some(Self::King),
            _ => None,
        }
    }
}

/// A piece of a specific color, packed as a single `repr(u8)` enum rather than a
/// `{ color, piece }` struct.
///
/// The struct form is 2 bytes with no niche, so `[Option<ColoredPiece>; 64]` (the
/// board's mailbox) would cost 128 bytes. This enum form gives `Option<ColoredPiece>`
/// a 1-byte niche, halving the mailbox to 64 bytes.
#[expect(missing_docs, reason = "variant names are the doc")]
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
        match Piece::from_letter(c) {
            Some(piece) => Some(Self::new(color, piece)),
            None => None,
        }
    }

    /// Convert a `ColoredPiece` into its FEN character.
    ///
    /// FEN's own rule, and only that rule: the letter comes from
    /// [`Piece::letter`], and the case is what makes it a colour.
    #[must_use]
    pub const fn to_fen(self) -> char {
        let letter = self.piece().letter();
        match self.color() {
            Color::White => letter.to_ascii_uppercase(),
            Color::Black => letter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_letter_round_trips() {
        for &piece in &Piece::ALL {
            assert_eq!(
                Piece::from_letter(piece.letter()),
                Some(piece),
                "{piece:?} did not survive letter -> from_letter"
            );
        }
    }

    #[test]
    fn from_letter_ignores_case_because_case_means_colour_not_kind() {
        for &piece in &Piece::ALL {
            let lower = piece.letter();
            let upper = lower.to_ascii_uppercase();
            assert_eq!(Piece::from_letter(lower), Some(piece), "{lower}");
            assert_eq!(Piece::from_letter(upper), Some(piece), "{upper}");
        }
    }

    #[test]
    fn from_letter_rejects_anything_that_is_not_a_piece() {
        for c in ['x', 'z', '1', ' ', '-', 'i'] {
            assert_eq!(Piece::from_letter(c), None, "{c:?} is not a piece letter");
        }
    }

    #[test]
    fn fen_case_is_the_colour_and_the_letter_is_the_kind() {
        // The asymmetric case: the same kind must differ only in case across
        // colours, which is what a swapped colour lookup would break.
        for &piece in &Piece::ALL {
            let white = ColoredPiece::new(Color::White, piece).to_fen();
            let black = ColoredPiece::new(Color::Black, piece).to_fen();
            assert!(white.is_ascii_uppercase(), "White {piece:?} gave {white}");
            assert!(black.is_ascii_lowercase(), "Black {piece:?} gave {black}");
            assert_eq!(
                white.to_ascii_lowercase(),
                black,
                "{piece:?} differs by more than case between colours"
            );
            assert_eq!(black, piece.letter());
        }
    }

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
