use super::color::Color;

/// Which castling moves are still available, packed as a 4-bit set.
///
/// Bit layout: `white kingside | white queenside | black kingside | black queenside`.
/// A plain `u8` bitset rather than a `bitflags!`-generated type: four named constants
/// and a handful of `const fn` accessors is the whole implementation, and it keeps the
/// type usable in `const` contexts without pulling in a macro crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CastlingRights(u8);

impl CastlingRights {
    pub const WHITE_KINGSIDE: Self = Self(0b0001);
    pub const WHITE_QUEENSIDE: Self = Self(0b0010);
    pub const BLACK_KINGSIDE: Self = Self(0b0100);
    pub const BLACK_QUEENSIDE: Self = Self(0b1000);

    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b1111);

    pub const fn kingside(color: Color) -> Self {
        match color {
            Color::White => Self::WHITE_KINGSIDE,
            Color::Black => Self::BLACK_KINGSIDE,
        }
    }

    pub const fn queenside(color: Color) -> Self {
        match color {
            Color::White => Self::WHITE_QUEENSIDE,
            Color::Black => Self::BLACK_QUEENSIDE,
        }
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

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

    pub const fn is_none(self) -> bool {
        self.0 == 0
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
}
