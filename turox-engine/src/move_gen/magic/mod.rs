//! Sliding-piece (bishop, rook, queen) attack generation via magic bitboards.
//!
//! # Design
//!
//! For a slider on `sq`, only the occupancy of squares it could actually be
//! blocked by matters — its *relevant occupancy mask*. Magic bitboards hash an
//! actual board's occupancy (restricted to that mask) down to a small index via
//! `(magic.wrapping_mul(occupied_bits)) >> shift`, precomputed to land on a
//! distinct slot per distinct occupancy pattern, so a lookup replaces a ray walk.
//!
//! `relevant_mask`/`attacks_for_occupancy` are generic over `[Direction; 4]`
//! rather than branching on piece — `relevant_mask(sq, ROOK_DIRS)` and
//! `relevant_mask(sq, BISHOP_DIRS)` are the same function. The mask drops each
//! ray's terminal square by shifting the full unblocked ray one step further
//! (off the board) and back, rather than reasoning per square about which edge
//! a ray dies on — a rook standing on `FILE_A` has its entire north/south mask
//! living on `FILE_A`, so subtracting that edge outright would wipe out real
//! blocker squares, not just the terminus (the same {axis}×{sign} shape that's
//! bitten `Board::make_move` twice):
//!
//! ```text
//! mask_dir = occluded_fill(sq.bitboard(), ALL, dir)  // sq + full ray to the edge
//!              .shift(dir)                           // terminus drops off-board
//!              .shift(dir.opposite())                // rest slides back
//! mask = union over dirs of mask_dir, minus sq
//! ```
//!
//! Magics are found by search, not hardcoded — a fixed-seed PRNG
//! (`crate::rng::xorshift64star`) generates sparse candidates, verified by
//! hand-walking every occupancy subset of the mask (Carry-Rippler) and
//! confirming the hash is collision-free. *Constructive* collisions, where two
//! occupancies happen to produce the *same* attack set, are fine and in fact
//! required — rejecting those makes minimal-size magics nearly unfindable.
//!
//! The search runs offline in a `#[test]` (`regen::regenerating_reproduces_
//! the_committed_magic_data`), not `const fn`: a spike measured a single
//! worst-case square's table build at 35.5s inside const-eval, which doesn't
//! scale to 128 squares inside `cargo build`. That test commits the winning
//! `Magic` data (`magics.rs`) plus the built tables' raw bytes as
//! `rook_attacks.bin`/`bishop_attacks.bin`, and re-runs the search each time to
//! keep the commits honest rather than copy-pasted. `decode` turns those bytes
//! back into real `[Bitboard; N]` `static` data at compile time — cheap
//! (`u64::from_le_bytes` per entry) compared to computing the tables
//! themselves, which is why decoding stays `const fn` while the search and
//! table build don't.

use crate::types::bitboard::Bitboard;
use crate::types::square::Square;

mod magics;
#[cfg(test)]
mod regen;

use magics::{BISHOP_MAGICS, ROOK_MAGICS};

/// `1 << 12`: the worst-case rook mask popcount across all 64 squares (e.g. a
/// rook on `a1`: 6 file squares + 6 rank squares, each excluding its far edge).
/// Not every square's slice is this long — `Magic::offset` plus this square's
/// actual `1 << mask.count_ones()` is what `ROOK_ATTACKS` actually reserves for
/// it — this is only the sum's upper bound, used to size that flat array.
const ROOK_TABLE_SIZE: usize = 102_400;

/// `1 << 9`: the worst-case bishop mask popcount (a bishop on one of the four
/// central squares). Same "sum of actual per-square sizes, not 64× the max"
/// relationship to `BISHOP_ATTACKS` as `ROOK_TABLE_SIZE` has to `ROOK_ATTACKS`.
const BISHOP_TABLE_SIZE: usize = 5_248;

/// One square's precomputed magic-hash parameters: where its relevant occupancy
/// bits live (`mask`), the multiplier that hashes them collision-free
/// (`magic`), how far to shift the product down to an index (`shift`), and
/// where its slice starts in the flat `ROOK_ATTACKS`/`BISHOP_ATTACKS` array
/// (`offset`). One array of 64 of these per piece type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Magic {
    mask: Bitboard,
    magic: u64,
    shift: u32,
    offset: usize,
}

/// The magic-bitboard hash, *local to `m`'s own slice* — the caller adds
/// `m.offset` to get an actual `ROOK_ATTACKS`/`BISHOP_ATTACKS` index. Restrict
/// `occupied` to `m.mask`'s bits, multiply by `m.magic`, keep the top
/// `64 - m.shift` bits: `((occupied & m.mask).bits().wrapping_mul(m.magic)) >>
/// m.shift`, as a `usize`. This is the one piece of this file that runs on
/// every real move-generation lookup, not just at table-build time.
const fn magic_index(occupied: Bitboard, m: &Magic) -> usize {
    (((occupied.and(m.mask)).bits().wrapping_mul(m.magic)) >> m.shift) as usize
}

/// Reinterprets `bytes` (tightly packed little-endian `u64`s, `N * 8` bytes
/// long — what `rook_attacks.bin`/`bishop_attacks.bin` hold) as `[Bitboard;
/// N]`. Panics (via the `bytes[...]` index) if `bytes.len() < N * 8`; the two
/// committed `.bin` files are always exactly `N * 8` for their respective `N`,
/// so this only fires if they and the `N` this is called with ever drift apart.
const fn decode<const N: usize>(bytes: &[u8]) -> [Bitboard; N] {
    let mut table = [Bitboard::EMPTY; N];
    let mut i = 0;
    while i < N {
        let mut b = [0u8; 8];
        let mut j = 0;
        while j < 8 {
            b[j] = bytes[i * 8 + j];
            j += 1;
        }
        table[i] = Bitboard::from_bits(u64::from_le_bytes(b));
        i += 1;
    }
    table
}

/// The flat rook attack table, decoded from the committed `rook_attacks.bin` —
/// see `ROOK_MAGICS`'s doc and `decode`'s doc for how it got there. `static`,
/// not `const`: at 800 KB, a `const` risks the compiler duplicating the whole
/// array at every reference site instead of storing it once.
static ROOK_ATTACKS: [Bitboard; ROOK_TABLE_SIZE] = decode(include_bytes!("rook_attacks.bin"));

/// The flat bishop attack table, decoded from `bishop_attacks.bin`. Same
/// `static`-not-`const` reasoning as `ROOK_ATTACKS`.
static BISHOP_ATTACKS: [Bitboard; BISHOP_TABLE_SIZE] = decode(include_bytes!("bishop_attacks.bin"));

/// Every square a rook standing on `sq` attacks, given `occupied` (both empty
/// and enemy/friendly squares — this module doesn't know about color). Stops
/// at, and includes, the first occupied square in each of the four directions.
pub const fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let m = &ROOK_MAGICS[sq.index() as usize];
    ROOK_ATTACKS[m.offset + magic_index(occupied, m)]
}

/// Every square a bishop standing on `sq` attacks, given `occupied`. Same
/// blocked-and-inclusive contract as `rook_attacks`.
pub const fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let m = &BISHOP_MAGICS[sq.index() as usize];
    BISHOP_ATTACKS[m.offset + magic_index(occupied, m)]
}

/// Every square a queen standing on `sq` attacks: the union of `rook_attacks`
/// and `bishop_attacks`.
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
    /// same file. This is the concrete case the mask gotcha documented above
    /// would get wrong: if `rook_mask` incorrectly subtracted all of `FILE_A`
    /// (rather than just the far edge per direction), the north/south blocker
    /// on `A6` would fall outside the mask, and the magic hash would collapse
    /// this occupancy together with a different one that doesn't have that
    /// blocker — returning attacks as if `A6` weren't there.
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
    fn decode_reinterprets_little_endian_bytes_as_bitboards() {
        let values: [u64; 3] = [0, 0xFF, 0x8000_0000_0000_0001];
        let mut bytes = Vec::new();
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let table: [Bitboard; 3] = decode(&bytes);
        for (i, v) in values.into_iter().enumerate() {
            assert_eq!(table[i], Bitboard::from_bits(v), "mismatch at index {i}");
        }
    }
}
