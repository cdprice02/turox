//! `File`, `Rank`, and `Square`: the board's coordinate axes.

use super::bitboard::Bitboard;
use std::fmt;
use turox_macros::Ordinal;

/// A file (column), A through H.
#[allow(
    missing_docs,
    reason = "each variant just names its own file; a per-variant doc would only restate that name"
)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Ordinal)]
pub enum File {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

/// A rank (row), 1 through 8.
#[allow(
    missing_docs,
    reason = "each variant just names its own rank; a per-variant doc would only restate that name"
)]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Ordinal)]
pub enum Rank {
    R1,
    R2,
    R3,
    R4,
    R5,
    R6,
    R7,
    R8,
}

/// A single board square, stored as a LERF (Little-Endian Rank-File) index.
///
/// a1 = 0, b1 = 1, ..., h1 = 7, a2 = 8, ..., h8 = 63. This ordering is what the
/// `Bitboard` transform constants (see `bitboard.rs`) assume. `Debug` is implemented
/// manually below (algebraic notation) rather than derived.
#[allow(
    missing_docs,
    reason = "each variant just names its own square (A1, B1, ...); a per-variant doc would only restate that name"
)]
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Ordinal)]
pub enum Square {
    A1,
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

impl File {
    /// Every square on this file, as a `Bitboard`.
    #[inline]
    #[must_use]
    pub const fn bitboard(self) -> Bitboard {
        match self {
            Self::A => Bitboard::from_bits(0x0101_0101_0101_0101),
            Self::B => Bitboard::from_bits(0x0202_0202_0202_0202),
            Self::C => Bitboard::from_bits(0x0404_0404_0404_0404),
            Self::D => Bitboard::from_bits(0x0808_0808_0808_0808),
            Self::E => Bitboard::from_bits(0x1010_1010_1010_1010),
            Self::F => Bitboard::from_bits(0x2020_2020_2020_2020),
            Self::G => Bitboard::from_bits(0x4040_4040_4040_4040),
            Self::H => Bitboard::from_bits(0x8080_8080_8080_8080),
        }
    }
}

impl Rank {
    /// Every square on this rank, as a `Bitboard`.
    #[inline]
    #[must_use]
    pub const fn bitboard(self) -> Bitboard {
        match self {
            Self::R1 => Bitboard::from_bits(0x0000_0000_0000_00FF),
            Self::R2 => Bitboard::from_bits(0x0000_0000_0000_FF00),
            Self::R3 => Bitboard::from_bits(0x0000_0000_00FF_0000),
            Self::R4 => Bitboard::from_bits(0x0000_0000_FF00_0000),
            Self::R5 => Bitboard::from_bits(0x0000_00FF_0000_0000),
            Self::R6 => Bitboard::from_bits(0x0000_FF00_0000_0000),
            Self::R7 => Bitboard::from_bits(0x00FF_0000_0000_0000),
            Self::R8 => Bitboard::from_bits(0xFF00_0000_0000_0000),
        }
    }
}

impl Square {
    /// The square at the intersection of `file` and `rank`.
    #[must_use]
    pub const fn new(file: File, rank: Rank) -> Self {
        Self::ALL[rank.index() * 8 + file.index()]
    }

    /// Parses algebraic notation (`"e4"`, `"a1"`, `"h8"`): the inverse of
    /// `Display`. Not `const fn`: char iteration over an arbitrary `&str`
    /// isn't const-callable on stable. Shared by FEN's en passant field
    /// (`board::fen::parse_square` wraps this with FEN's own error type)
    /// and UCI move notation (`types::moves::Move::from_uci`) rather than
    /// each parsing it separately, same reasoning `Display`'s own doc
    /// gives for the opposite direction.
    #[must_use]
    pub fn try_from_algebraic(s: &str) -> Option<Self> {
        let mut chars = s.chars();
        let file_ch = chars.next()?;
        let rank_ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let file = match file_ch {
            'a'..='h' => File::from_u8(u8::try_from(file_ch).ok()? - b'a')?,
            _ => return None,
        };
        let rank = match rank_ch {
            '1'..='8' => Rank::from_u8(u8::try_from(rank_ch).ok()? - b'1')?,
            _ => return None,
        };
        Some(Self::new(file, rank))
    }

    /// This square's file.
    ///
    /// # Panics
    ///
    /// Never: `% 8` is always < 8.
    #[must_use]
    pub const fn file(self) -> File {
        File::from_u8(self.to_u8() % 8).expect("% 8 is always < 8")
    }

    /// This square's rank.
    ///
    /// # Panics
    ///
    /// Never: a `u8` < 64, divided by 8, is always < 8.
    #[must_use]
    pub const fn rank(self) -> Rank {
        Rank::from_u8(self.to_u8() / 8).expect("a u8 < 64, divided by 8, is always < 8")
    }

    /// This square's single-bit mask within a `Bitboard`.
    #[must_use]
    pub const fn bitboard(self) -> Bitboard {
        Bitboard::from_bits(1u64 << self.to_u8())
    }

    /// Mirror across the horizontal midline (a1 <-> a8, e4 <-> e5).
    ///
    /// # Panics
    ///
    /// Never: `XOR`ing a value < 64 with a 6-bit mask stays < 64.
    #[must_use]
    pub const fn flip_rank(self) -> Self {
        Self::from_u8(self.to_u8() ^ 0b11_1000)
            .expect("XOR with a 6-bit mask on a value < 64 stays < 64")
    }

    /// Mirror across the vertical midline (a1 <-> h1, d4 <-> e4).
    ///
    /// # Panics
    ///
    /// Never: `XOR`ing a value < 64 with a 3-bit mask stays < 64.
    #[must_use]
    pub const fn flip_file(self) -> Self {
        Self::from_u8(self.to_u8() ^ 0b00_0111)
            .expect("XOR with a 3-bit mask on a value < 64 stays < 64")
    }

    /// The square offset by `df` files and `dr` ranks, or `None` if that would leave
    /// the board.
    #[must_use]
    pub const fn offset(self, df: i8, dr: i8) -> Option<Self> {
        let file = self.file().to_u8().cast_signed() + df;
        let rank = self.rank().to_u8().cast_signed() + dr;
        match (
            File::from_u8(file.cast_unsigned()),
            Rank::from_u8(rank.cast_unsigned()),
        ) {
            (Some(file), Some(rank)) => Some(Self::new(file, rank)),
            (None, _) | (_, None) => None,
        }
    }

    /// Chebyshev (king-move) distance between two squares.
    #[must_use]
    pub const fn distance(self, other: Self) -> u8 {
        let df =
            (self.file().to_u8().cast_signed() - other.file().to_u8().cast_signed()).unsigned_abs();
        let dr =
            (self.rank().to_u8().cast_signed() - other.rank().to_u8().cast_signed()).unsigned_abs();
        if df > dr {
            df
        } else {
            dr
        }
    }
}

/// Algebraic notation: `e4`, `a1`, `h8`. Shared by FEN formatting and UCI move
/// notation (see `types/moves.rs`) rather than each writing it separately.
impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Not `const fn`, so `char::from`/`u8::from` (regular `From`, not the
        // const-only-blocked kind) are fine here even though they aren't inside
        // `file()`/`rank()` above.
        let file = char::from(b'a' + self.file().to_u8());
        let rank = self.rank().to_u8() + 1;
        write!(f, "{file}{rank}")
    }
}

impl fmt::Debug for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerf_ordering_matches_file_rank() {
        assert_eq!(Square::A1.to_u8(), 0);
        assert_eq!(Square::H1.to_u8(), 7);
        assert_eq!(Square::A8.to_u8(), 56);
        assert_eq!(Square::H8.to_u8(), 63);
    }

    #[test]
    fn new_and_file_rank_round_trip() {
        for &file in &File::ALL {
            for &rank in &Rank::ALL {
                let sq = Square::new(file, rank);
                assert_eq!(sq.file(), file);
                assert_eq!(sq.rank(), rank);
            }
        }
    }

    #[test]
    fn from_u8_round_trips_with_to_u8() {
        for sq in Square::ALL {
            assert_eq!(Square::from_u8(sq.to_u8()), Some(sq));
        }
        assert_eq!(Square::from_u8(64), None);
    }

    /// `File`/`Rank` share `Square`'s `#[derive(Ordinal)]`; this is the same
    /// property, checked for the other two so a bug isn't only ever caught
    /// on the one of the three that happens to have 64 variants.
    #[test]
    fn file_and_rank_from_u8_round_trip_with_to_u8() {
        for file in File::ALL {
            assert_eq!(File::from_u8(file.to_u8()), Some(file));
        }
        assert_eq!(File::from_u8(8), None);

        for rank in Rank::ALL {
            assert_eq!(Rank::from_u8(rank.to_u8()), Some(rank));
        }
        assert_eq!(Rank::from_u8(8), None);
    }

    #[test]
    fn try_from_algebraic_parses_every_square() {
        for sq in Square::ALL {
            assert_eq!(Square::try_from_algebraic(&sq.to_string()), Some(sq));
        }
    }

    #[test]
    fn try_from_algebraic_rejects_malformed_input() {
        for bad in ["", "e", "e4e", "i4", "e9", "e0", "44", "ee"] {
            assert_eq!(
                Square::try_from_algebraic(bad),
                None,
                "expected None for {bad:?}"
            );
        }
    }

    /// A different angle from `try_from_algebraic_parses_every_square`: that
    /// one round-trips through parsing, this one checks `Display`'s own
    /// character construction (`b'a' + file`, `rank + 1`) matches `file()`/
    /// `rank()` directly, so a bug that happened to cancel out across
    /// `Display` and `try_from_algebraic` (both getting the same axis
    /// backwards, say) wouldn't hide from both checks at once.
    #[test]
    fn algebraic_display_matches_file_and_rank_directly() {
        for sq in Square::ALL {
            let s = sq.to_string();
            let mut chars = s.chars();
            let file_ch = chars.next().unwrap();
            let rank_ch = chars.next().unwrap();
            assert_eq!(file_ch, char::from(b'a' + sq.file().to_u8()), "sq={sq:?}");
            assert_eq!(
                u8::try_from(rank_ch.to_digit(10).unwrap()).expect("a single digit fits u8"),
                sq.rank().to_u8() + 1,
                "sq={sq:?}"
            );
        }
    }

    #[test]
    fn flips_are_involutions() {
        for sq in Square::ALL {
            assert_eq!(sq.flip_rank().flip_rank(), sq);
            assert_eq!(sq.flip_file().flip_file(), sq);
        }
    }

    #[test]
    fn flips_preserve_the_other_axis() {
        for sq in Square::ALL {
            assert_eq!(sq.flip_rank().file(), sq.file(), "sq={sq:?}");
            assert_eq!(sq.flip_file().rank(), sq.rank(), "sq={sq:?}");
        }
    }

    #[test]
    fn offset_zero_zero_is_identity() {
        for sq in Square::ALL {
            assert_eq!(sq.offset(0, 0), Some(sq));
        }
    }

    #[test]
    fn offset_in_bounds_matches_new() {
        assert_eq!(Square::E4.offset(1, 1), Some(Square::F5));
    }

    #[test]
    fn offset_out_of_range_is_none() {
        for sq in Square::ALL {
            for df in -20i8..=20 {
                for dr in -20i8..=20 {
                    let file = sq.file().to_u8().cast_signed() + df;
                    let rank = sq.rank().to_u8().cast_signed() + dr;
                    if !(0..=7).contains(&file) || !(0..=7).contains(&rank) {
                        assert_eq!(sq.offset(df, dr), None, "sq={sq:?} df={df} dr={dr}");
                    }
                }
            }
        }
    }

    #[test]
    fn distance_is_symmetric_and_zero_iff_equal() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(a.distance(b), b.distance(a), "a={a:?} b={b:?}");
                assert_eq!(a.distance(b) == 0, a == b, "a={a:?} b={b:?}");
            }
        }
        assert_eq!(Square::A1.distance(Square::H8), 7);
        assert_eq!(Square::A1.distance(Square::B1), 1);
    }
}
