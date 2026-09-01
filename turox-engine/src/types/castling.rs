//! `CastlingRights`: which of the four castling moves are still available.

use super::color::Color;
use super::square::Square;

/// Which castling moves are still available, packed as a 4-bit set.
///
/// Bit layout: `white kingside | white queenside | black kingside | black queenside`.
/// A plain `u8` bitset rather than a `bitflags!`-generated type: four named constants
/// and a handful of `const fn` accessors is the whole implementation, and it keeps the
/// type usable in `const` contexts without pulling in a macro crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CastlingRights(u8);

impl CastlingRights {
    /// White may still castle kingside.
    pub const WHITE_KINGSIDE: Self = Self(0b0001);
    /// White may still castle queenside.
    pub const WHITE_QUEENSIDE: Self = Self(0b0010);
    /// Black may still castle kingside.
    pub const BLACK_KINGSIDE: Self = Self(0b0100);
    /// Black may still castle queenside.
    pub const BLACK_QUEENSIDE: Self = Self(0b1000);

    /// No castling rights.
    pub const NONE: Self = Self(0);
    /// All four castling rights.
    pub const ALL: Self = Self(0b1111);

    /// `color`'s kingside right, in isolation.
    #[must_use]
    pub const fn kingside(color: Color) -> Self {
        match color {
            Color::White => Self::WHITE_KINGSIDE,
            Color::Black => Self::BLACK_KINGSIDE,
        }
    }

    /// `color`'s queenside right, in isolation.
    #[must_use]
    pub const fn queenside(color: Color) -> Self {
        match color {
            Color::White => Self::WHITE_QUEENSIDE,
            Color::Black => Self::BLACK_QUEENSIDE,
        }
    }

    /// Whether every right set in `other` is also set in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` with every right in `other` also set.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// `self` with every right in `other` cleared.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Clear both castling rights for a color, e.g. after its king moves.
    #[must_use]
    pub const fn without_color(self, color: Color) -> Self {
        match color {
            Color::White => self.without(Self::WHITE_KINGSIDE.with(Self::WHITE_QUEENSIDE)),
            Color::Black => self.without(Self::BLACK_KINGSIDE.with(Self::BLACK_QUEENSIDE)),
        }
    }

    /// Whether no rights remain.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// The castling rook's home square and where it lands, for `color`'s
    /// kingside (`kingside`) or queenside corner. The single place these four
    /// squares are spelled out: `Board::make_move` looks them up here rather
    /// than hardcoding its own copy of the same `{Color}x{kingside,queenside}`
    /// table, the shape that's produced scrambled bugs elsewhere in this
    /// crate before.
    #[must_use]
    pub const fn rook_squares(color: Color, kingside: bool) -> (Square, Square) {
        match (color, kingside) {
            (Color::White, true) => (Square::H1, Square::F1),
            (Color::White, false) => (Square::A1, Square::D1),
            (Color::Black, true) => (Square::H8, Square::F8),
            (Color::Black, false) => (Square::A8, Square::D8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_contains_every_individual_right() {
        for right in [
            CastlingRights::WHITE_KINGSIDE,
            CastlingRights::WHITE_QUEENSIDE,
            CastlingRights::BLACK_KINGSIDE,
            CastlingRights::BLACK_QUEENSIDE,
        ] {
            assert!(CastlingRights::ALL.contains(right));
        }
    }

    #[test]
    fn without_color_clears_only_that_color() {
        let cleared = CastlingRights::ALL.without_color(Color::White);
        assert!(!cleared.contains(CastlingRights::WHITE_KINGSIDE));
        assert!(!cleared.contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(cleared.contains(CastlingRights::BLACK_KINGSIDE));
        assert!(cleared.contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn with_then_without_round_trips_to_none() {
        let r = CastlingRights::NONE.with(CastlingRights::WHITE_KINGSIDE);
        assert!(r.without(CastlingRights::WHITE_KINGSIDE).is_none());
    }

    // `rook_squares` is checked against all four `(Color, kingside)` combinations
    // explicitly, not just White's: a {Color}x{kingside,queenside} mapping that
    // only gets checked on one color/side passes just as easily scrambled as
    // correct.
    #[test]
    fn rook_squares_covers_all_four_corners() {
        assert_eq!(
            CastlingRights::rook_squares(Color::White, true),
            (Square::H1, Square::F1)
        );
        assert_eq!(
            CastlingRights::rook_squares(Color::White, false),
            (Square::A1, Square::D1)
        );
        assert_eq!(
            CastlingRights::rook_squares(Color::Black, true),
            (Square::H8, Square::F8)
        );
        assert_eq!(
            CastlingRights::rook_squares(Color::Black, false),
            (Square::A8, Square::D8)
        );
    }
}
