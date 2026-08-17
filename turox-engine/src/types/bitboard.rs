use super::color::Color;
use super::square::Square;
use std::fmt;
use std::ops::{
    BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Shl, Shr, Sub, SubAssign,
};

/// A set of squares, one bit per square, LERF-indexed (bit 0 = a1, bit 63 = h8).
///
/// `#[repr(transparent)]` matches `u64`'s layout (sound for `transmute`/FFI) but
/// isn't what makes this free at runtime — LLVM scalar-replaces a one-field newtype
/// regardless of `repr`. Inherent methods are `const fn` (not just trait operators)
/// so `move_gen` can build attack tables at compile time; the operator impls below
/// just forward to them. No `From<u64>`/`PartialEq<u64>`: raw integers cross the
/// boundary only through `from_bits`/`bits()`.
///
/// Every operation here is implemented and verified against a naive reference
/// (see `tests/bitboard_props.rs`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Bitboard(u64);

impl Bitboard {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self(!0);

    pub const FILE_A: Self = Self(0x0101_0101_0101_0101);
    pub const FILE_H: Self = Self(0x8080_8080_8080_8080);
    pub const RANK_1: Self = Self(0x0000_0000_0000_00FF);
    pub const RANK_8: Self = Self(0xFF00_0000_0000_0000);

    #[inline]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[inline]
    pub const fn bits(self) -> u64 {
        self.0
    }

    // ---- Core arithmetic ----

    #[inline]
    pub const fn and(self, other: Self) -> Self {
        Self(self.bits() & other.bits())
    }

    #[inline]
    pub const fn or(self, other: Self) -> Self {
        Self(self.bits() | other.bits())
    }

    #[inline]
    pub const fn xor(self, other: Self) -> Self {
        Self(self.bits() ^ other.bits())
    }

    #[inline]
    pub const fn not(self) -> Self {
        Self(!self.bits())
    }

    /// `self & !other`. The workhorse of move generation: "attacks, minus squares
    /// occupied by our own pieces".
    #[inline]
    pub const fn and_not(self, other: Self) -> Self {
        Self(self.bits() & !other.bits())
    }

    /// Shift toward higher squares. `n >= 64` yields `EMPTY` rather than panicking.
    #[inline]
    pub const fn shl(self, n: u32) -> Self {
        Self(self.bits().unbounded_shl(n))
    }

    /// Shift toward lower squares. Same `n >= 64` contract as `shl`.
    #[inline]
    pub const fn shr(self, n: u32) -> Self {
        Self(self.bits().unbounded_shr(n))
    }

    // ---- Square membership and scanning ----

    #[inline]
    pub const fn contains(self, sq: Square) -> bool {
        !self.and(sq.bitboard()).is_empty()
    }

    #[inline]
    #[must_use]
    pub const fn with(self, sq: Square) -> Self {
        self.or(sq.bitboard())
    }

    #[inline]
    #[must_use]
    pub const fn without(self, sq: Square) -> Self {
        self.and_not(sq.bitboard())
    }

    #[inline]
    #[must_use]
    pub const fn toggled(self, sq: Square) -> Self {
        self.xor(sq.bitboard())
    }

    #[inline]
    pub const fn count(self) -> u32 {
        self.bits().count_ones()
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.bits() == 0
    }

    /// True iff exactly one square is set. Cheaper than `count() == 1`; on the hot
    /// path of legal move generation ("in check from exactly one piece?").
    #[inline]
    pub const fn is_single(self) -> bool {
        ((self.bits() ^ self.bits().wrapping_sub(1)) >> 1) == self.bits().wrapping_sub(1)
    }

    /// True iff two or more squares are set. Cheaper than `count() > 1`.
    #[inline]
    pub const fn has_multiple(self) -> bool {
        (self.bits() & self.bits().wrapping_sub(1)) != 0
    }

    /// The lowest-indexed set square, or `None` if empty.
    #[inline]
    pub const fn lsb(self) -> Option<Square> {
        Square::from_index(self.bits().trailing_zeros() as u8)
    }

    /// The highest-indexed set square, or `None` if empty.
    #[inline]
    pub const fn msb(self) -> Option<Square> {
        Square::from_index(63u8.wrapping_sub(self.bits().leading_zeros() as u8))
    }

    /// Clears and returns the lowest-indexed set square, or `None` if already
    /// empty. Not `const` (mutates through `&mut self`); the primitive
    /// `BitboardIter` below is built on.
    #[inline]
    pub fn pop_lsb(&mut self) -> Option<Square> {
        let sq = self.lsb()?;
        *self = self.without(sq);
        Some(sq)
    }

    // ---- Direction shift ----

    /// Shifts every set square one step in `dir`, discarding squares that would
    /// wrap around a file edge (e.g. h-file shifted `East` must vanish, not
    /// reappear on the a-file of the next rank) or off the top/bottom.
    #[inline]
    pub const fn shift(self, dir: Direction) -> Self {
        match dir {
            Direction::North => self.shl(8),
            Direction::South => self.shr(8),
            Direction::East => self.and_not(Self::FILE_H).shl(1),
            Direction::West => self.and_not(Self::FILE_A).shr(1),
            Direction::NorthEast => self.and_not(Self::FILE_H).shl(9),
            Direction::NorthWest => self.and_not(Self::FILE_A).shl(7),
            Direction::SouthEast => self.and_not(Self::FILE_H).shr(7),
            Direction::SouthWest => self.and_not(Self::FILE_A).shr(9),
        }
    }

    // ---- Flips and rotations ----
    // https://www.chessprogramming.org/Flipping_Mirroring_and_Rotating

    /// Mirror across the vertical midline (file a <-> h). Each rank's byte is
    /// bit-reversed.
    #[inline]
    #[must_use]
    pub const fn flip_horizontal(self) -> Self {
        const K1: u64 = 0x5555555555555555;
        const K2: u64 = 0x3333333333333333;
        const K4: u64 = 0x0F0F0F0F0F0F0F0F;
        let mut x = self.bits();
        x = ((x >> 1) & K1) + 2 * (x & K1);
        x = ((x >> 2) & K2) + 4 * (x & K2);
        x = ((x >> 4) & K4) + 16 * (x & K4);
        Self(x)
    }

    /// Mirror across the horizontal midline (rank 1 <-> rank 8).
    #[inline]
    #[must_use]
    pub const fn flip_vertical(self) -> Self {
        Self(self.bits().swap_bytes())
    }

    /// Mirror across the a1-h8 diagonal: (file, rank) -> (rank, file).
    #[inline]
    #[must_use]
    pub const fn flip_diagonal_a1h8(self) -> Self {
        let mut x = self.bits();
        let mut t: u64;
        const K1: u64 = 0x5500550055005500;
        const K2: u64 = 0x3333000033330000;
        const K4: u64 = 0x0F0F0F0F00000000;
        t = K4 & (x ^ (x << 28));
        x ^= t ^ (t >> 28);
        t = K2 & (x ^ (x << 14));
        x ^= t ^ (t >> 14);
        t = K1 & (x ^ (x << 7));
        x ^= t ^ (t >> 7);
        Self(x)
    }

    /// Mirror across the a8-h1 anti-diagonal.
    #[inline]
    #[must_use]
    pub const fn flip_diagonal_a8h1(self) -> Self {
        let mut x = self.bits();
        let mut t: u64;
        const K1: u64 = 0xAA00AA00AA00AA00;
        const K2: u64 = 0xCCCC0000CCCC0000;
        const K4: u64 = 0xF0F0F0F00F0F0F0F;
        t = x ^ (x << 36);
        x ^= K4 & (t ^ (x >> 36));
        t = K2 & (x ^ (x << 18));
        x ^= t ^ (t >> 18);
        t = K1 & (x ^ (x << 9));
        x ^= t ^ (t >> 9);
        Self(x)
    }

    /// Rotate 90 degrees clockwise (a1 -> a8; verified against a concrete corner
    /// mapping in `mod tests` below — the group-law property tests alone can't
    /// distinguish this from counter-clockwise).
    #[inline]
    #[must_use]
    pub const fn rotate_90_cw(self) -> Self {
        self.flip_diagonal_a1h8().flip_vertical()
    }

    /// Rotate 90 degrees counter-clockwise (a1 -> h1).
    #[inline]
    #[must_use]
    pub const fn rotate_90_ccw(self) -> Self {
        self.flip_vertical().flip_diagonal_a1h8()
    }

    /// Rotate 180 degrees. Equal to `flip_vertical` + `flip_horizontal` (either
    /// order), and to `self.bits().reverse_bits()`.
    #[inline]
    #[must_use]
    pub const fn rotate_180(self) -> Self {
        self.flip_vertical().flip_horizontal()
    }

    /// Flip vertically for `Black`, identity for `White` — views the board from
    /// the side-to-move's perspective (e.g. perspective-relative PSTs in eval).
    #[inline]
    #[must_use]
    pub const fn mirror_for(self, color: Color) -> Self {
        match color {
            Color::White => self,
            Color::Black => self.flip_vertical(),
        }
    }

    // ---- Planned for the move-generation change ----
    // Signatures fixed now so downstream code has a stable shape to write against.

    /// Every subset of `self`, via Carry-Rippler (`sub = (sub - self) & self`),
    /// including `EMPTY` and `self`. Walks a magic-bitboard mask's occupancies
    /// during table generation; not meant for boards with many bits set.
    pub fn subsets(self) -> impl Iterator<Item = Bitboard> {
        let mut next: Option<Bitboard> = Some(Bitboard::EMPTY);
        std::iter::from_fn(move || {
            let current = next?;
            next = if current == self {
                None
            } else {
                let bits = self.bits();
                Some(Bitboard::from_bits(
                    current.bits().wrapping_sub(bits) & bits,
                ))
            };
            Some(current)
        })
    }

    /// Every square reachable by repeatedly shifting `self` north, unioned with
    /// `self` (Kogge-Stone fill, not yet occlusion-limited).
    #[inline]
    pub const fn north_fill(self) -> Self {
        let mut x = self.bits();
        x |= x << 8;
        x |= x << 16;
        x |= x << 32;
        Self::from_bits(x)
    }

    #[inline]
    pub const fn south_fill(self) -> Self {
        let mut x = self.bits();
        x |= x >> 8;
        x |= x >> 16;
        x |= x >> 32;
        Self::from_bits(x)
    }

    /// `north_fill` + `south_fill`: every square on any file with a bit set.
    #[inline]
    pub const fn file_fill(self) -> Self {
        self.north_fill().south_fill()
    }

    /// Extends `self` step by step in `dir` through `empty` squares, stopping
    /// (inclusive) at the first non-empty square, or vanishing off the board edge;
    /// same wrap-safety contract as `shift`, which this is built from. One
    /// direction per call: a rook's full attack set is the union of calling this
    /// for North/South/East/West; a bishop's, the four diagonals; a queen's, all
    /// eight.
    #[inline]
    pub const fn occluded_fill(self, empty: Self, dir: Direction) -> Self {
        let mut gen = self;
        let mut frontier = self;
        let mut steps = 0;
        while steps < 7 {
            let new_frontier = frontier.shift(dir).and_not(gen);
            gen = gen.or(new_frontier);
            frontier = new_frontier.and(empty);
            steps += 1;
        }
        gen
    }

    /// Pawns of `color` on `self`, attacking east (white: NE; black: SE).
    #[inline]
    pub const fn pawn_attacks_east(self, color: Color) -> Self {
        match color {
            Color::White => self.shift(Direction::NorthEast),
            Color::Black => self.shift(Direction::SouthEast),
        }
    }

    /// Pawns of `color` on `self`, attacking west (white: NW; black: SW).
    #[inline]
    pub const fn pawn_attacks_west(self, color: Color) -> Self {
        match color {
            Color::White => self.shift(Direction::NorthWest),
            Color::Black => self.shift(Direction::SouthWest),
        }
    }

    /// Single-step forward pushes for pawns of `color` on `self`, onto `empty`
    /// squares only.
    #[inline]
    pub const fn pawn_pushes(self, color: Color, empty: Self) -> Self {
        match color {
            Color::White => self.shift(Direction::North).and(empty),
            Color::Black => self.shift(Direction::South).and(empty),
        }
    }

    /// 8-neighbour expansion: `self` unioned with every adjacent square (king-move
    /// dilation, for king safety / king-ring evaluation).
    #[inline]
    pub const fn dilate(self) -> Self {
        let dilated = self.or(self.shift(Direction::West).or(self.shift(Direction::East)));
        dilated.or(dilated
            .shift(Direction::North)
            .or(dilated.shift(Direction::South)))
    }

    /// Every square a knight on `self` attacks, for every knight in the set at
    /// once. Not a `shift`/fill composition — knight moves are a discontinuous
    /// jump, not a smear or single step — but they do have their own compound
    /// shift-with-masking formula (CPW's "Knight Pattern"), same technique family
    /// as `shift`'s diagonals, just wider file-edge masks since a knight can
    /// cross two files in one move. Also what generates the eventual per-square
    /// `KNIGHT_ATTACKS` lookup table in `move_gen/tables.rs`.
    #[inline]
    pub const fn knight_attacks(self) -> Self {
        let x = self.bits();
        let l1 = (x >> 1) & 0x7F7F7F7F7F7F7F7F;
        let l2 = (x >> 2) & 0x3F3F3F3F3F3F3F3F;
        let r1 = (x << 1) & 0xFEFEFEFEFEFEFEFE;
        let r2 = (x << 2) & 0xFCFCFCFCFCFCFCFC;
        let h1 = l1 | r1;
        let h2 = l2 | r2;
        Self::from_bits((h1 << 16) | (h1 >> 16) | (h2 << 8) | (h2 >> 8))
    }
}

/// Forwards a bitwise-set `std::ops` trait (`&`, `|`, `^`, `-`) to one of the
/// `const fn`s above. `$rhs` is `Bitboard` or `Square` — both convert via
/// `Bitboard::from` (the `Square` case uses our `From<Square>` impl below; the
/// `Bitboard` case uses core's reflexive `From<T> for T`), so one macro arm covers
/// both without a separate hand-written impl per type.
macro_rules! forward_binop {
    ($trait:ident, $method:ident, $inner:ident, $rhs:ty) => {
        impl $trait<$rhs> for Bitboard {
            type Output = Bitboard;
            #[inline]
            fn $method(self, rhs: $rhs) -> Bitboard {
                Bitboard::$inner(self, Bitboard::from(rhs))
            }
        }
    };
}

/// Same as `forward_binop`, for the `*Assign` traits (`&=`, `|=`, `-=`).
macro_rules! forward_assign_op {
    ($trait:ident, $method:ident, $inner:ident, $rhs:ty) => {
        impl $trait<$rhs> for Bitboard {
            #[inline]
            fn $method(&mut self, rhs: $rhs) {
                *self = Bitboard::$inner(*self, Bitboard::from(rhs));
            }
        }
    };
}

/// Shl/Shr take a shift count, not a bitboard-like value, so they pass `rhs`
/// through as-is instead of going through `Bitboard::from` like the macros above.
macro_rules! forward_shift_op {
    ($trait:ident, $method:ident, $inner:ident) => {
        impl $trait<u32> for Bitboard {
            type Output = Bitboard;
            #[inline]
            fn $method(self, rhs: u32) -> Bitboard {
                Bitboard::$inner(self, rhs)
            }
        }
    };
}

forward_binop!(BitAnd, bitand, and, Bitboard);
forward_binop!(BitAnd, bitand, and, Square);
forward_binop!(BitOr, bitor, or, Bitboard);
forward_binop!(BitOr, bitor, or, Square);
forward_binop!(BitXor, bitxor, xor, Bitboard);
forward_binop!(Sub, sub, and_not, Bitboard);
forward_binop!(Sub, sub, and_not, Square);

forward_assign_op!(BitAndAssign, bitand_assign, and, Bitboard);
forward_assign_op!(BitAndAssign, bitand_assign, and, Square);
forward_assign_op!(BitOrAssign, bitor_assign, or, Bitboard);
forward_assign_op!(BitOrAssign, bitor_assign, or, Square);
forward_assign_op!(BitXorAssign, bitxor_assign, xor, Bitboard);
forward_assign_op!(SubAssign, sub_assign, and_not, Bitboard);
forward_assign_op!(SubAssign, sub_assign, and_not, Square);

forward_shift_op!(Shl, shl, shl);
forward_shift_op!(Shr, shr, shr);

impl Not for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn not(self) -> Bitboard {
        Bitboard::not(self)
    }
}

impl From<Square> for Bitboard {
    #[inline]
    fn from(sq: Square) -> Self {
        sq.bitboard()
    }
}

/// A compass direction on the board, used with `Bitboard::shift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl Direction {
    pub const ALL: [Direction; 8] = [
        Direction::North,
        Direction::South,
        Direction::East,
        Direction::West,
        Direction::NorthEast,
        Direction::NorthWest,
        Direction::SouthEast,
        Direction::SouthWest,
    ];

    pub const fn opposite(self) -> Self {
        match self {
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::East => Direction::West,
            Direction::West => Direction::East,
            Direction::NorthEast => Direction::SouthWest,
            Direction::NorthWest => Direction::SouthEast,
            Direction::SouthEast => Direction::NorthWest,
            Direction::SouthWest => Direction::NorthEast,
        }
    }
}

/// Iterates the set squares of a `Bitboard` in ascending (LERF) order via
/// `pop_lsb`.
pub struct BitboardIter(Bitboard);

impl Iterator for BitboardIter {
    type Item = Square;

    #[inline]
    fn next(&mut self) -> Option<Square> {
        self.0.pop_lsb()
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = BitboardIter;

    #[inline]
    fn into_iter(self) -> BitboardIter {
        BitboardIter(self)
    }
}

impl fmt::Debug for Bitboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Bitboard(0x{:016X})", self.bits())?;
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let sq = Square::ALL[rank * 8 + file];
                write!(f, "{} ", if self.contains(sq) { '1' } else { '.' })?;
            }
            writeln!(f)?;
        }
        write!(f, "  a b c d e f g h")
    }
}

impl fmt::Display for Bitboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anchors `rotate_90_cw` to an absolute direction. The group-law property
    /// tests in `tests/bitboard_props.rs` (four cw rotations = identity, cw/ccw
    /// are mutual inverses) only prove the two are *consistent with each other* —
    /// that would hold even if both were secretly counter-clockwise. This pins it
    /// down: a physical clockwise spin (White's side, a1 bottom-left, h8
    /// top-right) sends each corner to the next one going bottom-left -> top-left
    /// -> top-right -> bottom-right -> bottom-left.
    #[test]
    fn rotate_90_cw_matches_known_corner_mapping() {
        assert_eq!(Square::A1.bitboard().rotate_90_cw(), Square::A8.bitboard());
        assert_eq!(Square::A8.bitboard().rotate_90_cw(), Square::H8.bitboard());
        assert_eq!(Square::H8.bitboard().rotate_90_cw(), Square::H1.bitboard());
        assert_eq!(Square::H1.bitboard().rotate_90_cw(), Square::A1.bitboard());
    }

    /// A hand-computed, eyeball-able complement to the property tests in
    /// `tests/bitboard_props.rs`: a center square has all 8 king-move neighbors.
    #[test]
    fn dilate_center_square_covers_all_eight_neighbors() {
        let expected = Bitboard::EMPTY
            .with(Square::E4)
            .with(Square::D3)
            .with(Square::D4)
            .with(Square::D5)
            .with(Square::E3)
            .with(Square::E5)
            .with(Square::F3)
            .with(Square::F4)
            .with(Square::F5);
        assert_eq!(Square::E4.bitboard().dilate(), expected);
    }

    /// A1 has only 3 on-board neighbors (north, east, northeast) — the board-edge
    /// counterpart to the center-square case above.
    #[test]
    fn dilate_corner_square_drops_off_board_neighbors() {
        let expected = Bitboard::EMPTY
            .with(Square::A1)
            .with(Square::A2)
            .with(Square::B1)
            .with(Square::B2);
        assert_eq!(Square::A1.bitboard().dilate(), expected);
    }

    /// A knight on a central square has all 8 legal moves.
    #[test]
    fn knight_attacks_center_square_has_all_eight_moves() {
        let expected = Bitboard::EMPTY
            .with(Square::F6)
            .with(Square::G5)
            .with(Square::G3)
            .with(Square::F2)
            .with(Square::D2)
            .with(Square::C3)
            .with(Square::C5)
            .with(Square::D6);
        assert_eq!(Square::E4.bitboard().knight_attacks(), expected);
    }

    /// A knight in the corner has only 2 legal moves — the classic "knight on
    /// the rim is dim" case.
    #[test]
    fn knight_attacks_corner_square_has_only_two_moves() {
        let expected = Bitboard::EMPTY.with(Square::B3).with(Square::C2);
        assert_eq!(Square::A1.bitboard().knight_attacks(), expected);
    }
}
