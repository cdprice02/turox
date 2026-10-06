//! Sliding-piece (bishop, rook, queen) attack generation via magic bitboards.
//!
//! A slider's attack set only depends on the occupancy of squares it could actually be
//! blocked by (its *relevant occupancy mask*), so `magic_index` hashes a real occupancy,
//! restricted to that mask, down to a small table index: a lookup instead of a ray walk
//! on every call.
//!
//! Two things here are found by search rather than derived, and are included as
//! data: the multipliers (`rook_magics.bin`/`bishop_magics.bin`) and the attack
//! tables they index (`rook_attacks.bin`/`bishop_attacks.bin`). The `magicgen`
//! tool writes all four. Everything else about a square's hash follows from its
//! mask, and is computed here at compile time.

use crate::types::bitboard::{Bitboard, Direction};
use crate::types::square::Square;

/// The four rook ray directions, as data rather than a piece distinction to
/// branch on: `relevant_mask` and `magics` do the same work for either set.
const ROOK_DIRS: [Direction; 4] = [
    Direction::North,
    Direction::South,
    Direction::East,
    Direction::West,
];

/// The four bishop ray directions.
const BISHOP_DIRS: [Direction; 4] = [
    Direction::NorthEast,
    Direction::NorthWest,
    Direction::SouthEast,
    Direction::SouthWest,
];

/// The relevant occupancy mask for a slider on `sq` moving along `dirs`:
/// squares whose occupancy can actually change the attack set.
///
/// Drops each ray's terminal square by shifting the full unblocked ray one
/// step further (off the board) and back, rather than reasoning per square
/// about which edge a ray dies on: `occluded_fill(sq.bitboard(), ALL,
/// dir).shift(dir).shift(dir.opposite())`. A rook standing on `FILE_A` has its
/// entire north/south mask living on `FILE_A`, so subtracting that edge
/// outright would wipe out real blocker squares, not just the terminus.
const fn relevant_mask(sq: Square, dirs: [Direction; 4]) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    let mut i = 0;
    while i < dirs.len() {
        let dir = dirs[i];
        mask = mask.or(sq
            .bitboard()
            .occluded_fill(Bitboard::ALL, dir)
            .shift(dir)
            .shift(dir.opposite()));
        i += 1;
    }
    mask.without(sq)
}

/// The squares whose occupancy can change a rook's attacks from `sq`: its rank
/// and file, minus `sq` and each ray's edge square, since a blocker on the edge
/// stops nothing that the edge would not stop anyway.
///
/// Public so the generator that searches for the magic multipliers, which
/// lives outside this crate, hashes exactly the mask the lookup uses: the
/// table layout depends on it, and two definitions could drift.
#[must_use]
pub const fn rook_mask(sq: Square) -> Bitboard {
    relevant_mask(sq, ROOK_DIRS)
}

/// The bishop counterpart of [`rook_mask`], over both diagonals.
#[must_use]
pub const fn bishop_mask(sq: Square) -> Bitboard {
    relevant_mask(sq, BISHOP_DIRS)
}

/// The sum of `1 << rook_mask(sq).count()` over all 64 squares: one slot per
/// relevant occupancy, so the flat table wastes nothing between slices.
/// `magics` checks it against the masks at compile time.
const ROOK_TABLE_SIZE: usize = 102_400;

/// The bishop counterpart of `ROOK_TABLE_SIZE`.
const BISHOP_TABLE_SIZE: usize = 5_248;

/// One square's magic-hash parameters: where its relevant occupancy bits live
/// (`mask`), the multiplier that hashes them collision-free (`magic`), how far
/// to shift the product down to an index (`shift`), and where its slice starts
/// in the flat `ROOK_ATTACKS`/`BISHOP_ATTACKS` array (`offset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "`magic` is the actual chess-programming term for this field; a generic name like `multiplier` would read worse to this module's audience"
)]
struct Magic {
    /// The relevant-occupancy mask for this square: the squares a slider could
    /// be blocked by, with the board edges excluded, since a piece on the edge
    /// blocks nothing beyond itself.
    mask: Bitboard,
    /// The one field found by search rather than derived from `mask`.
    magic: u64,
    /// `64 - mask.count()`, so the index keeps exactly as many bits as the mask
    /// has.
    shift: u32,
    /// Where this square's slice begins in the shared attack table, since all
    /// 64 squares live in one array rather than 64 separate ones.
    offset: usize,
}

/// The magic-bitboard hash, *local to `m`'s own slice*. The caller adds
/// `m.offset` to get an actual `ROOK_ATTACKS`/`BISHOP_ATTACKS` index. Restrict
/// `occupied` to `m.mask`'s bits, multiply by `m.magic`, keep the top
/// `64 - m.shift` bits. This is the one piece of this file that runs on
/// every real move-generation lookup.
// `m.shift` always leaves at most 12 significant bits (the widest rook/bishop
// mask), which fits `usize` on every platform this engine targets; there is
// no const-stable `TryFrom<u64> for usize` to reach for instead
// (rust-lang/rust#143874), and this is the hottest line in the engine, so a
// runtime-checked fallback doesn't belong here even once one exists.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "m.shift always leaves <= 12 significant bits, which fits usize; no const-stable TryFrom<u64> for usize exists yet (rust-lang/rust#143874)"
)]
const fn magic_index(occupied: Bitboard, m: &Magic) -> usize {
    (((occupied.and(m.mask)).bits().wrapping_mul(m.magic)) >> m.shift) as usize
}

/// Reinterprets `bytes`, tightly packed little-endian `u64`s, as `[u64; N]`.
///
/// Requires the length to be exactly `N * 8`, so a file the generator wrote
/// for a different table size fails the build rather than being silently
/// truncated or read past.
const fn decode<const N: usize>(bytes: &[u8]) -> [u64; N] {
    assert!(
        bytes.len() == N * 8,
        "a magic data file's length does not match the table it is decoded into"
    );
    let mut values = [0u64; N];
    let mut i = 0;
    while i < N {
        let mut b = [0u8; 8];
        let mut j = 0;
        while j < 8 {
            b[j] = bytes[i * 8 + j];
            j += 1;
        }
        values[i] = u64::from_le_bytes(b);
        i += 1;
    }
    values
}

/// [`decode`], as the attack table's `Bitboard`s.
const fn decode_table<const N: usize>(bytes: &[u8]) -> [Bitboard; N] {
    let values: [u64; N] = decode(bytes);
    let mut table = [Bitboard::EMPTY; N];
    let mut i = 0;
    while i < N {
        table[i] = Bitboard::from_bits(values[i]);
        i += 1;
    }
    table
}

/// Every square's `Magic`, from its multiplier in `multipliers` and everything
/// else derived from its mask: the shift from the mask's width, and the offset
/// as a running total of the slice sizes before it.
///
/// Fails the build unless those slices add up to exactly `table_size`, which
/// is what ties the masks here to the table the generator wrote: a mask that
/// disagreed with the generator's would shift every later offset.
const fn magics(multipliers: &[u8], dirs: [Direction; 4], table_size: usize) -> [Magic; 64] {
    let multipliers: [u64; 64] = decode(multipliers);
    let mut magics = [Magic {
        mask: Bitboard::EMPTY,
        magic: 0,
        shift: 0,
        offset: 0,
    }; 64];
    let mut offset = 0;
    let mut i = 0;
    while i < Square::ALL.len() {
        let mask = relevant_mask(Square::ALL[i], dirs);
        magics[i] = Magic {
            mask,
            magic: multipliers[i],
            shift: 64 - mask.count(),
            offset,
        };
        offset += 1 << mask.count();
        i += 1;
    }
    assert!(
        offset == table_size,
        "the masks' slices do not add up to the attack table's size"
    );
    magics
}

/// Every square's rook hash parameters, indexed by `Square::index`.
const ROOK_MAGICS: [Magic; 64] = magics(
    include_bytes!("rook_magics.bin"),
    ROOK_DIRS,
    ROOK_TABLE_SIZE,
);

/// Every square's bishop hash parameters, indexed by `Square::index`.
const BISHOP_MAGICS: [Magic; 64] = magics(
    include_bytes!("bishop_magics.bin"),
    BISHOP_DIRS,
    BISHOP_TABLE_SIZE,
);

/// The flat rook attack table. `static`, not `const`: at 800 KB, a `const`
/// risks the compiler duplicating the whole array at every reference site
/// instead of storing it once.
static ROOK_ATTACKS: [Bitboard; ROOK_TABLE_SIZE] = decode_table(include_bytes!("rook_attacks.bin"));

/// The flat bishop attack table. Same `static`-not-`const` reasoning as
/// `ROOK_ATTACKS`.
static BISHOP_ATTACKS: [Bitboard; BISHOP_TABLE_SIZE] =
    decode_table(include_bytes!("bishop_attacks.bin"));

/// Every square a rook standing on `sq` attacks, given `occupied` (both empty and
/// enemy/friendly squares; this module doesn't know about color).
///
/// Stops at, and includes, the first occupied square in each of the four directions.
#[must_use]
pub const fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let m = &ROOK_MAGICS[sq.index()];
    ROOK_ATTACKS[m.offset + magic_index(occupied, m)]
}

/// Every square a bishop standing on `sq` attacks, given `occupied`. Same
/// blocked-and-inclusive contract as `rook_attacks`.
#[must_use]
pub const fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let m = &BISHOP_MAGICS[sq.index()];
    BISHOP_ATTACKS[m.offset + magic_index(occupied, m)]
}

/// Every square a queen standing on `sq` attacks: the union of `rook_attacks`
/// and `bishop_attacks`.
#[must_use]
pub const fn queen_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    rook_attacks(sq, occupied).or(bishop_attacks(sq, occupied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rook_on_d4_open_board_covers_full_rank_and_file() {
        let expected = Bitboard::EMPTY
            .with(Square::A4)
            .with(Square::B4)
            .with(Square::C4)
            .with(Square::E4)
            .with(Square::F4)
            .with(Square::G4)
            .with(Square::H4)
            .with(Square::D1)
            .with(Square::D2)
            .with(Square::D3)
            .with(Square::D5)
            .with(Square::D6)
            .with(Square::D7)
            .with(Square::D8);
        assert_eq!(rook_attacks(Square::D4, Bitboard::EMPTY), expected);
    }

    #[test]
    fn rook_on_a1_corner_open_board() {
        let expected = Bitboard::EMPTY
            .with(Square::B1)
            .with(Square::C1)
            .with(Square::D1)
            .with(Square::E1)
            .with(Square::F1)
            .with(Square::G1)
            .with(Square::H1)
            .with(Square::A2)
            .with(Square::A3)
            .with(Square::A4)
            .with(Square::A5)
            .with(Square::A6)
            .with(Square::A7)
            .with(Square::A8);
        assert_eq!(rook_attacks(Square::A1, Bitboard::EMPTY), expected);
    }

    /// Rook standing on the a-file itself, blocked by a piece further up the
    /// same file. This is the concrete case `relevant_mask`'s mask
    /// gotcha would get wrong: if `rook_mask` incorrectly subtracted all of `FILE_A`
    /// (rather than just the far edge per direction), the north/south blocker
    /// on `A6` would fall outside the mask, and the magic hash would collapse
    /// this occupancy together with a different one that doesn't have that
    /// blocker: returning attacks as if `A6` weren't there.
    #[test]
    fn rook_on_a_file_is_blocked_by_a_piece_further_up_the_same_file() {
        let occupied = Bitboard::EMPTY.with(Square::A6);
        let expected = Bitboard::EMPTY
            .with(Square::A2)
            .with(Square::A3)
            .with(Square::A1)
            .with(Square::A5)
            .with(Square::A6)
            .with(Square::B4)
            .with(Square::C4)
            .with(Square::D4)
            .with(Square::E4)
            .with(Square::F4)
            .with(Square::G4)
            .with(Square::H4);
        assert_eq!(rook_attacks(Square::A4, occupied), expected);
    }

    #[test]
    fn rook_is_blocked_by_the_first_piece_in_each_direction() {
        // Rook on e4, boxed in by pieces on e6 (north) and b4 (west); south and
        // east stay open to the edge.
        let occupied = Bitboard::EMPTY.with(Square::E6).with(Square::B4);
        let expected = Bitboard::EMPTY
            .with(Square::E5)
            .with(Square::E6)
            .with(Square::E3)
            .with(Square::E2)
            .with(Square::E1)
            .with(Square::D4)
            .with(Square::C4)
            .with(Square::B4)
            .with(Square::F4)
            .with(Square::G4)
            .with(Square::H4);
        assert_eq!(rook_attacks(Square::E4, occupied), expected);
    }

    #[test]
    fn bishop_on_d4_open_board_covers_both_diagonals() {
        let expected = Bitboard::EMPTY
            .with(Square::A1)
            .with(Square::B2)
            .with(Square::C3)
            .with(Square::E5)
            .with(Square::F6)
            .with(Square::G7)
            .with(Square::H8)
            .with(Square::A7)
            .with(Square::B6)
            .with(Square::C5)
            .with(Square::E3)
            .with(Square::F2)
            .with(Square::G1);
        assert_eq!(bishop_attacks(Square::D4, Bitboard::EMPTY), expected);
    }

    #[test]
    fn bishop_on_a1_corner_only_has_the_one_diagonal() {
        let expected = Bitboard::EMPTY
            .with(Square::B2)
            .with(Square::C3)
            .with(Square::D4)
            .with(Square::E5)
            .with(Square::F6)
            .with(Square::G7)
            .with(Square::H8);
        assert_eq!(bishop_attacks(Square::A1, Bitboard::EMPTY), expected);
    }

    #[test]
    fn bishop_is_blocked_by_the_first_piece_on_a_diagonal() {
        // Bishop on d4, blocked by a piece on f6 partway up the a1-h8-ward
        // diagonal; the other three diagonals stay open to the edge.
        let occupied = Bitboard::EMPTY.with(Square::F6);
        let expected = Bitboard::EMPTY
            .with(Square::A1)
            .with(Square::B2)
            .with(Square::C3)
            .with(Square::E5)
            .with(Square::F6)
            .with(Square::A7)
            .with(Square::B6)
            .with(Square::C5)
            .with(Square::E3)
            .with(Square::F2)
            .with(Square::G1);
        assert_eq!(bishop_attacks(Square::D4, occupied), expected);
    }

    #[test]
    fn queen_on_d4_open_board_is_rook_and_bishop_combined() {
        let expected = rook_attacks(Square::D4, Bitboard::EMPTY)
            .or(bishop_attacks(Square::D4, Bitboard::EMPTY));
        assert_eq!(queen_attacks(Square::D4, Bitboard::EMPTY), expected);
    }

    #[test]
    fn queen_is_blocked_independently_on_each_of_its_eight_directions() {
        let occupied = Bitboard::EMPTY.with(Square::E6).with(Square::F6);
        let expected = rook_attacks(Square::D4, occupied).or(bishop_attacks(Square::D4, occupied));
        assert_eq!(queen_attacks(Square::D4, occupied), expected);
    }

    #[test]
    fn rook_attacks_on_fully_occupied_board_has_at_most_four_squares() {
        for sq in Square::ALL {
            assert!(rook_attacks(sq, Bitboard::ALL).count() <= 4, "sq={sq:?}");
        }
    }

    #[test]
    fn bishop_attacks_on_fully_occupied_board_has_at_most_four_squares() {
        for sq in Square::ALL {
            assert!(bishop_attacks(sq, Bitboard::ALL).count() <= 4, "sq={sq:?}");
        }
    }

    #[test]
    fn decode_reinterprets_little_endian_bytes_as_bitboards() {
        let values: [u64; 3] = [0, 0xFF, 0x8000_0000_0000_0001];
        let mut bytes = Vec::new();
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let table: [Bitboard; 3] = decode_table(&bytes);
        for (i, v) in values.into_iter().enumerate() {
            assert_eq!(table[i], Bitboard::from_bits(v), "mismatch at index {i}");
        }
    }
}
