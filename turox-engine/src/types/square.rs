use super::bitboard::Bitboard;
use std::fmt;

/// Declares a small `repr(u8)` enum with an `ALL` lookup table and
/// `from_index`/`index` conversions, from a single list of variant names. Used for
/// `File`, `Rank`, and `Square` below — the axis size (8, 8, 64) and variant list
/// are the only things that differ between them.
macro_rules! declare_axis {
    ($(#[$meta:meta])* $name:ident, $size:literal, { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($variant),*
        }

        impl $name {
            pub const ALL: [$name; $size] = [$($name::$variant),*];

            pub const fn from_index(i: u8) -> Option<Self> {
                if (i as usize) < $size {
                    Some(Self::ALL[i as usize])
                } else {
                    None
                }
            }

            pub const fn index(self) -> u8 {
                self as u8
            }
        }
    };
}

declare_axis!(
    /// A file (column), A through H.
    #[derive(Debug)]
    File, 8, { A, B, C, D, E, F, G, H }
);

declare_axis!(
    /// A rank (row), 1 through 8.
    #[derive(Debug)]
    Rank, 8, { R1, R2, R3, R4, R5, R6, R7, R8 }
);

declare_axis!(
    /// A single board square, stored as a LERF (Little-Endian Rank-File) index:
    /// a1 = 0, b1 = 1, ..., h1 = 7, a2 = 8, ..., h8 = 63. This ordering is what the
    /// `Bitboard` transform constants (see `bitboard.rs`) assume. `Debug` is
    /// implemented manually below (algebraic notation) rather than derived.
    Square, 64, {
        A1, B1, C1, D1, E1, F1, G1, H1,
        A2, B2, C2, D2, E2, F2, G2, H2,
        A3, B3, C3, D3, E3, F3, G3, H3,
        A4, B4, C4, D4, E4, F4, G4, H4,
        A5, B5, C5, D5, E5, F5, G5, H5,
        A6, B6, C6, D6, E6, F6, G6, H6,
        A7, B7, C7, D7, E7, F7, G7, H7,
        A8, B8, C8, D8, E8, F8, G8, H8,
    }
);

impl File {
    /// Every square on this file, as a `Bitboard`.
    pub const fn bitboard(self) -> Bitboard {
        todo!()
    }
}

impl Rank {
    /// Every square on this rank, as a `Bitboard`.
    pub const fn bitboard(self) -> Bitboard {
        todo!()
    }
}

impl Square {
    pub const fn new(file: File, rank: Rank) -> Self {
        // ALL is laid out rank-major (LERF), so this is the inverse of file()/rank().
        Self::ALL[(rank.index() as usize) * 8 + file.index() as usize]
    }

    pub const fn file(self) -> File {
        match File::from_index(self.index() % 8) {
            Some(f) => f,
            None => unreachable!(),
        }
    }

    pub const fn rank(self) -> Rank {
        match Rank::from_index(self.index() / 8) {
            Some(r) => r,
            None => unreachable!(),
        }
    }

    /// This square's single-bit mask within a `Bitboard`.
    pub const fn bitboard(self) -> Bitboard {
        Bitboard::from_bits(1u64 << self.index())
    }

    /// Mirror across the horizontal midline (a1 <-> a8, e4 <-> e5).
    pub const fn flip_rank(self) -> Self {
        match Self::from_index(self.index() ^ 0b111000) {
            Some(sq) => sq,
            None => unreachable!(),
        }
    }

    /// Mirror across the vertical midline (a1 <-> h1, d4 <-> e4).
    pub const fn flip_file(self) -> Self {
        match Self::from_index(self.index() ^ 0b000111) {
            Some(sq) => sq,
            None => unreachable!(),
        }
    }

    /// The square offset by `df` files and `dr` ranks, or `None` if that would leave
    /// the board.
    pub const fn offset(self, df: i8, dr: i8) -> Option<Self> {
        let file = self.file().index() as i8 + df;
        let rank = self.rank().index() as i8 + dr;
        if file < 0 || file > 7 || rank < 0 || rank > 7 {
            return None;
        }
        // Checked above: both are in 0..=7, so these conversions always succeed.
        let file = match File::from_index(file as u8) {
            Some(f) => f,
            None => unreachable!(),
        };
        let rank = match Rank::from_index(rank as u8) {
            Some(r) => r,
            None => unreachable!(),
        };
        Some(Self::new(file, rank))
    }

    /// Chebyshev (king-move) distance between two squares.
    pub const fn distance(self, other: Self) -> u8 {
        let df = (self.file().index() as i8 - other.file().index() as i8).unsigned_abs();
        let dr = (self.rank().index() as i8 - other.rank().index() as i8).unsigned_abs();
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
        let file = (b'a' + self.file().index()) as char;
        let rank = self.rank().index() + 1;
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
        assert_eq!(Square::A1.index(), 0);
        assert_eq!(Square::H1.index(), 7);
        assert_eq!(Square::A8.index(), 56);
        assert_eq!(Square::H8.index(), 63);
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
    fn from_index_round_trips_with_index() {
        for sq in Square::ALL {
            assert_eq!(Square::from_index(sq.index()), Some(sq));
        }
        assert_eq!(Square::from_index(64), None);
    }

    #[test]
    fn flips_are_involutions() {
        for sq in Square::ALL {
            assert_eq!(sq.flip_rank().flip_rank(), sq);
            assert_eq!(sq.flip_file().flip_file(), sq);
        }
    }

    #[test]
    fn offset_out_of_bounds_is_none() {
        assert_eq!(Square::A1.offset(-1, 0), None);
        assert_eq!(Square::H8.offset(1, 0), None);
        assert_eq!(Square::A1.offset(0, -1), None);
        assert_eq!(Square::H8.offset(0, 1), None);
    }

    #[test]
    fn offset_in_bounds_matches_new() {
        assert_eq!(Square::E4.offset(1, 1), Some(Square::F5));
    }

    #[test]
    fn distance_is_symmetric_and_zero_on_diagonal() {
        for a in Square::ALL {
            assert_eq!(a.distance(a), 0);
            for b in Square::ALL {
                assert_eq!(a.distance(b), b.distance(a));
            }
        }
        assert_eq!(Square::A1.distance(Square::H8), 7);
        assert_eq!(Square::A1.distance(Square::B1), 1);
    }
}
