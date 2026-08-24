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
//! The search runs offline in a `#[test]`
//! (`regenerating_reproduces_the_committed_magic_data`), not `const fn`: a
//! spike measured a single worst-case square's table build at 35.5s inside
//! const-eval, which doesn't scale to 128 squares inside `cargo build`. That
//! test commits the winning `Magic` data below plus the built tables' raw bytes
//! as `rook_attacks.bin`/`bishop_attacks.bin`, and re-runs the search each time
//! to keep the commits honest rather than copy-pasted. `decode` turns those
//! bytes back into real `[Bitboard; N]` `static` data at compile time — cheap
//! (`u64::from_le_bytes` per entry) compared to computing the tables
//! themselves, which is why decoding stays `const fn` while the search and
//! table build don't.
//!
//! # Public surface
//!
//! - `fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard`
//! - `fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard`
//! - `fn queen_attacks(sq: Square, occupied: Bitboard) -> Bitboard` — the union
//!   of the two above.

use crate::rng::xorshift64star;
use crate::types::bitboard::Bitboard;
use crate::types::square::Square;
use crate::Direction;

/// The four rook ray directions, as a plain data array rather than a piece
/// distinction the code has to branch on — every generic helper below takes a
/// `[Direction; 4]` (rook's or bishop's) and does the same work either way.
/// Only the regeneration test (`tests::regenerating_reproduces_the_committed_magic_data`
/// and friends) still calls into the search/build machinery this feeds — the
/// real lookup path below only needs the already-committed `ROOK_MAGICS`.
#[allow(dead_code)]
const ROOK_DIRS: [Direction; 4] = [
    Direction::North,
    Direction::South,
    Direction::East,
    Direction::West,
];

/// The four bishop ray directions. Same regen-test-only status as `ROOK_DIRS`.
#[allow(dead_code)]
const BISHOP_DIRS: [Direction; 4] = [
    Direction::NorthEast,
    Direction::NorthWest,
    Direction::SouthEast,
    Direction::SouthWest,
];

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

/// Fixed PRNG seed for the magic search — arbitrary but fixed, so the search is
/// byte-for-byte reproducible across runs and platforms (same reasoning
/// `board::zobrist`'s TODO commits to for its own key table). Must be nonzero:
/// `xorshift64star` treats 0 as a fixed point (see `rng`'s module doc). Reuses
/// the same 64-bit golden-ratio constant `benches/bitboard.rs` already seeds its
/// sampling PRNG with — a recognizable, well-mixing nonzero value, not a shared
/// RNG state between the two (each owns its own PRNG stream from here).
/// Regen-test-only, same as `ROOK_DIRS`.
#[allow(dead_code)]
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

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

/// Found by `find_all_magics(ROOK_DIRS, SEED)` — see
/// `tests::regenerating_reproduces_the_committed_magic_data` for the check that
/// keeps this honest.
const ROOK_MAGICS: [Magic; 64] = [
    Magic {
        mask: Bitboard::from_bits(0x000101010101017e),
        magic: 0x1180002240028050,
        shift: 52,
        offset: 0,
    },
    Magic {
        mask: Bitboard::from_bits(0x000202020202027c),
        magic: 0x80400020001000c0,
        shift: 53,
        offset: 4096,
    },
    Magic {
        mask: Bitboard::from_bits(0x000404040404047a),
        magic: 0x8100200100409029,
        shift: 53,
        offset: 6144,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008080808080876),
        magic: 0x0080080050008004,
        shift: 53,
        offset: 8192,
    },
    Magic {
        mask: Bitboard::from_bits(0x001010101010106e),
        magic: 0x8a00116002000408,
        shift: 53,
        offset: 10240,
    },
    Magic {
        mask: Bitboard::from_bits(0x002020202020205e),
        magic: 0x4300040013002886,
        shift: 53,
        offset: 12288,
    },
    Magic {
        mask: Bitboard::from_bits(0x004040404040403e),
        magic: 0x090001002c008200,
        shift: 53,
        offset: 14336,
    },
    Magic {
        mask: Bitboard::from_bits(0x008080808080807e),
        magic: 0x0100024180650002,
        shift: 52,
        offset: 16384,
    },
    Magic {
        mask: Bitboard::from_bits(0x0001010101017e00),
        magic: 0x0000800081400122,
        shift: 53,
        offset: 20480,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002020202027c00),
        magic: 0x4a0080200140008a,
        shift: 54,
        offset: 22528,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004040404047a00),
        magic: 0x400300200010410a,
        shift: 54,
        offset: 23552,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008080808087600),
        magic: 0x2000801001080080,
        shift: 54,
        offset: 24576,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010101010106e00),
        magic: 0x0044800400080080,
        shift: 54,
        offset: 25600,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020202020205e00),
        magic: 0x4188801401800200,
        shift: 54,
        offset: 26624,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040404040403e00),
        magic: 0xa011004200940100,
        shift: 54,
        offset: 27648,
    },
    Magic {
        mask: Bitboard::from_bits(0x0080808080807e00),
        magic: 0x0000803444800100,
        shift: 53,
        offset: 28672,
    },
    Magic {
        mask: Bitboard::from_bits(0x00010101017e0100),
        magic: 0x8005208004400081,
        shift: 53,
        offset: 30720,
    },
    Magic {
        mask: Bitboard::from_bits(0x00020202027c0200),
        magic: 0x1030104002402000,
        shift: 54,
        offset: 32768,
    },
    Magic {
        mask: Bitboard::from_bits(0x00040404047a0400),
        magic: 0x0810008020008050,
        shift: 54,
        offset: 33792,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008080808760800),
        magic: 0x0200090030010021,
        shift: 54,
        offset: 34816,
    },
    Magic {
        mask: Bitboard::from_bits(0x00101010106e1000),
        magic: 0x0084808004001800,
        shift: 54,
        offset: 35840,
    },
    Magic {
        mask: Bitboard::from_bits(0x00202020205e2000),
        magic: 0x220080801a000400,
        shift: 54,
        offset: 36864,
    },
    Magic {
        mask: Bitboard::from_bits(0x00404040403e4000),
        magic: 0x2b0054000510080a,
        shift: 54,
        offset: 37888,
    },
    Magic {
        mask: Bitboard::from_bits(0x00808080807e8000),
        magic: 0x00000600088404c1,
        shift: 53,
        offset: 38912,
    },
    Magic {
        mask: Bitboard::from_bits(0x000101017e010100),
        magic: 0x0080004840022000,
        shift: 53,
        offset: 40960,
    },
    Magic {
        mask: Bitboard::from_bits(0x000202027c020200),
        magic: 0x00300040400c2000,
        shift: 54,
        offset: 43008,
    },
    Magic {
        mask: Bitboard::from_bits(0x000404047a040400),
        magic: 0x0400410100200610,
        shift: 54,
        offset: 44032,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008080876080800),
        magic: 0x0008008080500008,
        shift: 54,
        offset: 45056,
    },
    Magic {
        mask: Bitboard::from_bits(0x001010106e101000),
        magic: 0x0028440080800800,
        shift: 54,
        offset: 46080,
    },
    Magic {
        mask: Bitboard::from_bits(0x002020205e202000),
        magic: 0x0002001a00080431,
        shift: 54,
        offset: 47104,
    },
    Magic {
        mask: Bitboard::from_bits(0x004040403e404000),
        magic: 0x2002008200012418,
        shift: 54,
        offset: 48128,
    },
    Magic {
        mask: Bitboard::from_bits(0x008080807e808000),
        magic: 0x5400884200008401,
        shift: 53,
        offset: 49152,
    },
    Magic {
        mask: Bitboard::from_bits(0x0001017e01010100),
        magic: 0x018121c001800880,
        shift: 53,
        offset: 51200,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002027c02020200),
        magic: 0x8004200080804000,
        shift: 54,
        offset: 53248,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004047a04040400),
        magic: 0x0004134101002000,
        shift: 54,
        offset: 54272,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008087608080800),
        magic: 0x20010088a1001003,
        shift: 54,
        offset: 55296,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010106e10101000),
        magic: 0x6000e40801001100,
        shift: 54,
        offset: 56320,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020205e20202000),
        magic: 0x1008402008011004,
        shift: 54,
        offset: 57344,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040403e40404000),
        magic: 0x052006a804001011,
        shift: 54,
        offset: 58368,
    },
    Magic {
        mask: Bitboard::from_bits(0x0080807e80808000),
        magic: 0x20e0008402002343,
        shift: 53,
        offset: 59392,
    },
    Magic {
        mask: Bitboard::from_bits(0x00017e0101010100),
        magic: 0x10002082c0008002,
        shift: 53,
        offset: 61440,
    },
    Magic {
        mask: Bitboard::from_bits(0x00027c0202020200),
        magic: 0x000a0500408e0020,
        shift: 54,
        offset: 63488,
    },
    Magic {
        mask: Bitboard::from_bits(0x00047a0404040400),
        magic: 0x0084220080420034,
        shift: 54,
        offset: 64512,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008760808080800),
        magic: 0x0084210090010028,
        shift: 54,
        offset: 65536,
    },
    Magic {
        mask: Bitboard::from_bits(0x00106e1010101000),
        magic: 0x420201a004120008,
        shift: 54,
        offset: 66560,
    },
    Magic {
        mask: Bitboard::from_bits(0x00205e2020202000),
        magic: 0x0042010804060010,
        shift: 54,
        offset: 67584,
    },
    Magic {
        mask: Bitboard::from_bits(0x00403e4040404000),
        magic: 0x00c1000200410004,
        shift: 54,
        offset: 68608,
    },
    Magic {
        mask: Bitboard::from_bits(0x00807e8080808000),
        magic: 0x0012040c85ca0011,
        shift: 53,
        offset: 69632,
    },
    Magic {
        mask: Bitboard::from_bits(0x007e010101010100),
        magic: 0x10514002e0800280,
        shift: 53,
        offset: 71680,
    },
    Magic {
        mask: Bitboard::from_bits(0x007c020202020200),
        magic: 0x0601024008208100,
        shift: 54,
        offset: 73728,
    },
    Magic {
        mask: Bitboard::from_bits(0x007a040404040400),
        magic: 0x010250814204a200,
        shift: 54,
        offset: 74752,
    },
    Magic {
        mask: Bitboard::from_bits(0x0076080808080800),
        magic: 0x12012010000d0100,
        shift: 54,
        offset: 75776,
    },
    Magic {
        mask: Bitboard::from_bits(0x006e101010101000),
        magic: 0x04000d8801005100,
        shift: 54,
        offset: 76800,
    },
    Magic {
        mask: Bitboard::from_bits(0x005e202020202000),
        magic: 0x0800405020042801,
        shift: 54,
        offset: 77824,
    },
    Magic {
        mask: Bitboard::from_bits(0x003e404040404000),
        magic: 0x2000800200410080,
        shift: 54,
        offset: 78848,
    },
    Magic {
        mask: Bitboard::from_bits(0x007e808080808000),
        magic: 0x2810140044812200,
        shift: 53,
        offset: 79872,
    },
    Magic {
        mask: Bitboard::from_bits(0x7e01010101010100),
        magic: 0x24008001004099a1,
        shift: 52,
        offset: 81920,
    },
    Magic {
        mask: Bitboard::from_bits(0x7c02020202020200),
        magic: 0x0058c00100208011,
        shift: 53,
        offset: 86016,
    },
    Magic {
        mask: Bitboard::from_bits(0x7a04040404040400),
        magic: 0x8018384020010111,
        shift: 53,
        offset: 88064,
    },
    Magic {
        mask: Bitboard::from_bits(0x7608080808080800),
        magic: 0x000200102004400a,
        shift: 53,
        offset: 90112,
    },
    Magic {
        mask: Bitboard::from_bits(0x6e10101010101000),
        magic: 0x0009000408003083,
        shift: 53,
        offset: 92160,
    },
    Magic {
        mask: Bitboard::from_bits(0x5e20202020202000),
        magic: 0x0002000821041002,
        shift: 53,
        offset: 94208,
    },
    Magic {
        mask: Bitboard::from_bits(0x3e40404040404000),
        magic: 0x006088110a00904c,
        shift: 53,
        offset: 96256,
    },
    Magic {
        mask: Bitboard::from_bits(0x7e80808080808000),
        magic: 0x0504430050840022,
        shift: 52,
        offset: 98304,
    },
];

/// Found by `find_all_magics(BISHOP_DIRS, SEED)` — same reproducibility check
/// as `ROOK_MAGICS`.
const BISHOP_MAGICS: [Magic; 64] = [
    Magic {
        mask: Bitboard::from_bits(0x0040201008040200),
        magic: 0x0608208480820080,
        shift: 58,
        offset: 0,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000402010080400),
        magic: 0x00880a084210a020,
        shift: 59,
        offset: 64,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000004020100a00),
        magic: 0x8958020401220514,
        shift: 59,
        offset: 96,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000000040221400),
        magic: 0x1108214240240014,
        shift: 59,
        offset: 128,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000000002442800),
        magic: 0x040a021106158084,
        shift: 59,
        offset: 160,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000000204085000),
        magic: 0x00a2882008806404,
        shift: 59,
        offset: 192,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000020408102000),
        magic: 0x200e8a8420201504,
        shift: 59,
        offset: 224,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002040810204000),
        magic: 0x0000440402c21002,
        shift: 58,
        offset: 256,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020100804020000),
        magic: 0x0011122a08080080,
        shift: 59,
        offset: 320,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040201008040000),
        magic: 0x0000080800919204,
        shift: 59,
        offset: 352,
    },
    Magic {
        mask: Bitboard::from_bits(0x00004020100a0000),
        magic: 0x0280081811408000,
        shift: 59,
        offset: 384,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000004022140000),
        magic: 0x0001040400810080,
        shift: 59,
        offset: 416,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000000244280000),
        magic: 0x10000a0210000021,
        shift: 59,
        offset: 448,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000020408500000),
        magic: 0x1061420212200424,
        shift: 59,
        offset: 480,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002040810200000),
        magic: 0x1040108e94202000,
        shift: 59,
        offset: 512,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004081020400000),
        magic: 0x090020c404410808,
        shift: 59,
        offset: 544,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010080402000200),
        magic: 0x0241019802880208,
        shift: 59,
        offset: 576,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020100804000400),
        magic: 0x0128402c28028426,
        shift: 59,
        offset: 608,
    },
    Magic {
        mask: Bitboard::from_bits(0x004020100a000a00),
        magic: 0x0228001000801551,
        shift: 57,
        offset: 640,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000402214001400),
        magic: 0x000800042a002041,
        shift: 57,
        offset: 768,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000024428002800),
        magic: 0x0818200402180080,
        shift: 57,
        offset: 896,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002040850005000),
        magic: 0x0490480202062004,
        shift: 57,
        offset: 1024,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004081020002000),
        magic: 0x1c03c00411084800,
        shift: 59,
        offset: 1152,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008102040004000),
        magic: 0x20020800a2020254,
        shift: 59,
        offset: 1184,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008040200020400),
        magic: 0x1850088010208540,
        shift: 59,
        offset: 1216,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010080400040800),
        magic: 0x008d200010820220,
        shift: 59,
        offset: 1248,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020100a000a1000),
        magic: 0x2009410408120400,
        shift: 57,
        offset: 1280,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040221400142200),
        magic: 0x200200800800800a,
        shift: 55,
        offset: 1408,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002442800284400),
        magic: 0x8100840060802000,
        shift: 55,
        offset: 1920,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004085000500800),
        magic: 0x0408020001220101,
        shift: 57,
        offset: 2432,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008102000201000),
        magic: 0x8004040103012181,
        shift: 59,
        offset: 2560,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010204000402000),
        magic: 0x2012006002110100,
        shift: 59,
        offset: 2592,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004020002040800),
        magic: 0x10024840a8600200,
        shift: 59,
        offset: 2624,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008040004081000),
        magic: 0x0004041440021004,
        shift: 59,
        offset: 2656,
    },
    Magic {
        mask: Bitboard::from_bits(0x00100a000a102000),
        magic: 0x8004007201040400,
        shift: 57,
        offset: 2688,
    },
    Magic {
        mask: Bitboard::from_bits(0x0022140014224000),
        magic: 0x4204200800008820,
        shift: 55,
        offset: 2816,
    },
    Magic {
        mask: Bitboard::from_bits(0x0044280028440200),
        magic: 0x10060484000e0020,
        shift: 55,
        offset: 3328,
    },
    Magic {
        mask: Bitboard::from_bits(0x0008500050080400),
        magic: 0x0620024380410480,
        shift: 57,
        offset: 3840,
    },
    Magic {
        mask: Bitboard::from_bits(0x0010200020100800),
        magic: 0x0090008080010401,
        shift: 59,
        offset: 3968,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020400040201000),
        magic: 0x0008050102214440,
        shift: 59,
        offset: 4000,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002000204081000),
        magic: 0x1408010c10882000,
        shift: 59,
        offset: 4032,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004000408102000),
        magic: 0x00104c240c502000,
        shift: 59,
        offset: 4064,
    },
    Magic {
        mask: Bitboard::from_bits(0x000a000a10204000),
        magic: 0x0908820801040a01,
        shift: 57,
        offset: 4096,
    },
    Magic {
        mask: Bitboard::from_bits(0x0014001422400000),
        magic: 0x8100004208004a80,
        shift: 57,
        offset: 4224,
    },
    Magic {
        mask: Bitboard::from_bits(0x0028002844020000),
        magic: 0x0f20200c20600400,
        shift: 57,
        offset: 4352,
    },
    Magic {
        mask: Bitboard::from_bits(0x0050005008040200),
        magic: 0x01200c20a0600200,
        shift: 57,
        offset: 4480,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020002010080400),
        magic: 0x008a420202100410,
        shift: 59,
        offset: 4608,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040004020100800),
        magic: 0x200a080101110220,
        shift: 59,
        offset: 4640,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000020408102000),
        magic: 0x0324048410080004,
        shift: 59,
        offset: 4672,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000040810204000),
        magic: 0x04060200840c4466,
        shift: 59,
        offset: 4704,
    },
    Magic {
        mask: Bitboard::from_bits(0x00000a1020400000),
        magic: 0x080000c200900008,
        shift: 59,
        offset: 4736,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000142240000000),
        magic: 0x1122800142060441,
        shift: 59,
        offset: 4768,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000284402000000),
        magic: 0x0001001022021602,
        shift: 59,
        offset: 4800,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000500804020000),
        magic: 0x0400042014c30000,
        shift: 59,
        offset: 4832,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000201008040200),
        magic: 0x8210241002a20200,
        shift: 59,
        offset: 4864,
    },
    Magic {
        mask: Bitboard::from_bits(0x0000402010080400),
        magic: 0x0022080204044068,
        shift: 59,
        offset: 4896,
    },
    Magic {
        mask: Bitboard::from_bits(0x0002040810204000),
        magic: 0x0030a2021ea0080c,
        shift: 58,
        offset: 4928,
    },
    Magic {
        mask: Bitboard::from_bits(0x0004081020400000),
        magic: 0x0420808082903030,
        shift: 59,
        offset: 4992,
    },
    Magic {
        mask: Bitboard::from_bits(0x000a102040000000),
        magic: 0x450a802040c41001,
        shift: 59,
        offset: 5024,
    },
    Magic {
        mask: Bitboard::from_bits(0x0014224000000000),
        magic: 0x2002000006209801,
        shift: 59,
        offset: 5056,
    },
    Magic {
        mask: Bitboard::from_bits(0x0028440200000000),
        magic: 0x0000400004218200,
        shift: 59,
        offset: 5088,
    },
    Magic {
        mask: Bitboard::from_bits(0x0050080402000000),
        magic: 0x400008120a100900,
        shift: 59,
        offset: 5120,
    },
    Magic {
        mask: Bitboard::from_bits(0x0020100804020000),
        magic: 0x7147101030008181,
        shift: 59,
        offset: 5152,
    },
    Magic {
        mask: Bitboard::from_bits(0x0040201008040200),
        magic: 0x8404200404009832,
        shift: 58,
        offset: 5184,
    },
];

/// The relevant occupancy mask for a slider on `sq` moving along `dirs`:
/// squares whose occupancy can actually change the attack set. See the module
/// doc's "Masks and attacks-for-occupancy are generic over direction, not
/// piece" section for the shift-then-shift-back-then-subtract-`sq` derivation —
/// same formula for `relevant_mask(sq, ROOK_DIRS)` and
/// `relevant_mask(sq, BISHOP_DIRS)`, no piece-specific branch.
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

/// The actual attack set for a slider on `sq` moving along `dirs`, given a real
/// (not mask-restricted) `occupied`. Ground truth for both the magic search
/// (verifying a candidate's hash is collision-free) and the table build (what
/// actually goes in each slot): union `occluded_fill` over each direction,
/// strip `sq`.
const fn attacks_for_occupancy(sq: Square, occupied: Bitboard, dirs: [Direction; 4]) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    let mut i = 0;
    while i < dirs.len() {
        let dir = dirs[i];
        mask = mask.or(sq.bitboard().occluded_fill(occupied.not(), dir));
        i += 1;
    }
    mask.without(sq)
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

/// Searches for a magic multiplier for a slider on `sq` along `dirs`, starting
/// the PRNG from `state`: generate a sparse candidate (AND a few successive
/// `xorshift64star` outputs together, biasing toward sparse, better-hashing
/// multipliers), reject early if `(mask.bits() * magic) & 0xFF00_0000_0000_0000`
/// has fewer than 6 set bits (cheap prefilter before the expensive check), then
/// verify by hand-walking every subset of `mask` (Carry-Rippler, since
/// `Bitboard::subsets()` isn't `const fn`) and confirming `magic_index` never
/// collides two subsets whose `attacks_for_occupancy` results genuinely differ.
/// Retry with the next candidate on any real collision.
///
/// Returns the winning `Magic` alongside the PRNG's state after finding it, so
/// `find_all_magics` can thread one continuously-advancing stream across all 64
/// squares instead of restarting every square from the same `state`.
fn find_magic(sq: Square, dirs: [Direction; 4], state: u64) -> (Magic, u64) {
    let mask = relevant_mask(sq, dirs);

    // cheap prefilter for magic
    let mut state = state;
    let mut magic;

    'magic_search: loop {
        loop {
            let state1 = xorshift64star(state);
            let state2 = xorshift64star(state1);
            let state3 = xorshift64star(state2);
            state = state3; // keep the rotating state for further iteration

            magic = state1 & state2 & state3;
            if (mask.bits().wrapping_mul(magic) & 0xFF00_0000_0000_0000).count_ones() < 6 {
                break;
            }
        }
        let magic = magic; // immut-ify magic

        // Carry-Rippler: walk every subset of `mask`, checking that no two
        // subsets with genuinely different attack sets hash to the same slot.
        let m = Magic {
            mask,
            magic,
            shift: 64 - mask.count(),
            offset: 0,
        };
        // Indexable directly by `magic_index`'s result — `1 << mask.count()` is
        // exactly the number of distinct slots that hash can ever produce for
        // this mask/shift, so no hashing is needed to track occupied slots.
        let mut slots: Vec<Option<Bitboard>> = vec![None; 1usize << mask.count()];
        let bits = mask.bits();
        let mut sub = 0u64;
        loop {
            let occupied = Bitboard::from_bits(sub);
            let attacks = attacks_for_occupancy(sq, occupied, dirs);
            let idx = magic_index(occupied, &m);
            match slots[idx] {
                Some(existing) if existing != attacks => {
                    continue 'magic_search;
                }
                _ => {
                    slots[idx] = Some(attacks);
                }
            }
            if sub == bits {
                return (m, state);
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
}

/// Runs `find_magic` for all 64 squares along `dirs`, threading one
/// continuously-advancing PRNG stream (seeded from `seed`) across all of them
/// rather than restarting each square from the same state, and assembles the
/// result into a `[Magic; 64]` — including each square's `offset`, a running
/// total of `1 << mask.count_ones()` over the squares before it (so
/// `offset[0] == 0`, and the last square's `offset + (1 << popcount)` is the
/// flat table's real total size, `<=` `ROOK_TABLE_SIZE`/`BISHOP_TABLE_SIZE`).
/// Regen-test-only now that `ROOK_MAGICS`/`BISHOP_MAGICS` are committed — see
/// `tests::regenerating_reproduces_the_committed_magic_data`.
#[allow(dead_code)]
fn find_all_magics(dirs: [Direction; 4], seed: u64) -> [Magic; 64] {
    let mut m: [Magic; 64] = [Magic::default(); 64];
    let mut offset = 0;
    let mut state = seed;
    for sq in Square::ALL {
        let (mut magic, next_state) = find_magic(sq, dirs, state);
        state = next_state;
        magic.offset = offset;
        offset += 1 << magic.mask.count();
        m[sq.index() as usize] = magic;
    }
    m
}

/// Builds the full flat attack table for `dirs`, given every square's
/// already-found `magics` (with `offset`s already assigned by
/// `find_all_magics`): for each square, hand-walk every subset of its mask
/// (same Carry-Rippler as `find_magic`'s verification step) and write
/// `attacks_for_occupancy(sq, subset, dirs)` into
/// `table[magics[sq].offset + magic_index(subset, &magics[sq])]`. Slots no
/// square's magic ever produces are unused padding — `Bitboard::EMPTY` is a
/// safe sentinel for them, since a slider on a real board always attacks at
/// least one square (even fully boxed in, it attacks whatever boxed it in), so
/// `EMPTY` is never a real answer to collide with. `N` is `ROOK_TABLE_SIZE`/
/// `BISHOP_TABLE_SIZE`; the returned array's unused tail beyond the real total
/// (see `find_all_magics`'s doc) stays `EMPTY` too. Not `const fn`, for the
/// same reason `find_magic` isn't. Regen-test-only, same as `find_all_magics`.
#[allow(dead_code)]
fn build_table<const N: usize>(dirs: [Direction; 4], magics: &[Magic; 64]) -> [Bitboard; N] {
    let mut table = [Bitboard::EMPTY; N];
    for sq in Square::ALL {
        let m = &magics[sq.index() as usize];
        let bits = m.mask.bits();
        let mut sub = 0u64;
        loop {
            let occupied = Bitboard::from_bits(sub);
            let attacks = attacks_for_occupancy(sq, occupied, dirs);
            let idx = magic_index(occupied, m);
            table[magics[sq.index() as usize].offset + idx] = attacks;
            if sub == bits {
                break;
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
    table
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
    use proptest::prelude::*;

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

    // ---- Everything below lives here rather than in tests/magic_props.rs
    // because everything it exercises (`relevant_mask`, `attacks_for_occupancy`,
    // `magic_index`, `find_magic`, `find_all_magics`, `build_table`, `Magic`) is
    // private: tests/*.rs compiles as a separate crate that only sees `pub`
    // items, so these genuinely cannot live anywhere else.

    fn any_square() -> impl Strategy<Value = Square> {
        (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
    }

    fn any_bitboard() -> impl Strategy<Value = Bitboard> {
        any::<u64>().prop_map(Bitboard::from_bits)
    }

    /// Either piece's direction set — every property below holds for both, so
    /// this is what makes each one a single check instead of two.
    fn any_dirs() -> impl Strategy<Value = [Direction; 4]> {
        prop_oneof![Just(ROOK_DIRS), Just(BISHOP_DIRS)]
    }

    fn direction_delta(dir: Direction) -> (i8, i8) {
        match dir {
            Direction::North => (0, 1),
            Direction::South => (0, -1),
            Direction::East => (1, 0),
            Direction::West => (-1, 0),
            Direction::NorthEast => (1, 1),
            Direction::NorthWest => (-1, 1),
            Direction::SouthEast => (1, -1),
            Direction::SouthWest => (-1, -1),
        }
    }

    /// Independent reference for `relevant_mask`: walk one square at a time via
    /// `Square::offset` until stepping off the board, keeping every square
    /// visited except the last (the terminus `relevant_mask` deliberately
    /// excludes). Built from `Square::offset`, not `occluded_fill`/`shift`,
    /// unlike `relevant_mask` itself.
    fn naive_mask(sq: Square, dirs: [Direction; 4]) -> Bitboard {
        let mut result = Bitboard::EMPTY;
        for dir in dirs {
            let (df, dr) = direction_delta(dir);
            let mut current = sq;
            let mut ray = Vec::new();
            while let Some(next) = current.offset(df, dr) {
                ray.push(next);
                current = next;
            }
            ray.pop(); // drop the terminus
            for s in ray {
                result = result.with(s);
            }
        }
        result
    }

    /// Independent reference for `attacks_for_occupancy`: step one square at a
    /// time along each of `dirs`, including every square visited, stopping
    /// (inclusively) at the first occupied square or the board edge. Same
    /// shape as `tests/magic_props.rs`'s `naive_slider_attacks`, duplicated
    /// rather than shared — an independent reference that imported the
    /// production code's own helper wouldn't be independent.
    fn naive_attacks_for_occupancy(
        sq: Square,
        occupied: Bitboard,
        dirs: [Direction; 4],
    ) -> Bitboard {
        let mut result = Bitboard::EMPTY;
        for dir in dirs {
            let (df, dr) = direction_delta(dir);
            let mut current = sq;
            while let Some(next) = current.offset(df, dr) {
                result = result.with(next);
                if occupied.contains(next) {
                    break;
                }
                current = next;
            }
        }
        result
    }

    proptest! {
        #[test]
        fn relevant_mask_matches_naive_walk(sq in any_square(), dirs in any_dirs()) {
            prop_assert_eq!(relevant_mask(sq, dirs), naive_mask(sq, dirs));
        }

        #[test]
        fn relevant_mask_never_contains_its_own_square(sq in any_square(), dirs in any_dirs()) {
            prop_assert!(!relevant_mask(sq, dirs).contains(sq));
        }

        #[test]
        fn attacks_for_occupancy_matches_naive_walk(
            sq in any_square(),
            occupied in any_bitboard(),
            dirs in any_dirs(),
        ) {
            prop_assert_eq!(
                attacks_for_occupancy(sq, occupied, dirs),
                naive_attacks_for_occupancy(sq, occupied, dirs)
            );
        }

        #[test]
        fn attacks_for_occupancy_never_contains_its_own_square(
            sq in any_square(),
            occupied in any_bitboard(),
            dirs in any_dirs(),
        ) {
            prop_assert!(!attacks_for_occupancy(sq, occupied, dirs).contains(sq));
        }
    }

    /// Popcount bounds are exhaustive over all 64 squares (not proptest-random)
    /// because they're a known, fixed set of facts about the whole board, not a
    /// property that benefits from random sampling — the standard published
    /// numbers for magic-bitboard masks, and the same ones this project's own
    /// design research measured directly: rook masks are 10 bits interior, 12
    /// at worst (e.g. a corner); bishop masks are 5 at a corner, 9 at the four
    /// central squares.
    #[test]
    fn relevant_mask_popcount_matches_known_bounds() {
        for sq in Square::ALL {
            let rook_bits = relevant_mask(sq, ROOK_DIRS).count();
            assert!(
                (10..=12).contains(&rook_bits),
                "rook mask on {sq:?} has {rook_bits} bits, expected 10..=12"
            );
            let bishop_bits = relevant_mask(sq, BISHOP_DIRS).count();
            assert!(
                (5..=9).contains(&bishop_bits),
                "bishop mask on {sq:?} has {bishop_bits} bits, expected 5..=9"
            );
        }
    }

    /// `find_all_magics` must assign every square an `offset` that's the
    /// running total of `1 << mask.count()` over the squares before it — that's
    /// what makes each square's slice land at a distinct, correctly-sized
    /// region of the flat table. Checked against `relevant_mask` directly
    /// (independent of whatever `find_all_magics` internally does to compute
    /// the same masks), and the grand total confirmed to fit the reserved
    /// `ROOK_TABLE_SIZE`/`BISHOP_TABLE_SIZE` — the exact totals this project's
    /// design research measured (102,400 / 5,248), which is what those two
    /// constants were sized from in the first place.
    #[test]
    fn find_all_magics_offsets_are_a_correct_prefix_sum_of_popcounts() {
        for (dirs, table_size) in [
            (ROOK_DIRS, ROOK_TABLE_SIZE),
            (BISHOP_DIRS, BISHOP_TABLE_SIZE),
        ] {
            let magics = find_all_magics(dirs, SEED);
            let mut expected_offset = 0;
            for sq in Square::ALL {
                let m = &magics[sq.index() as usize];
                assert_eq!(m.mask, relevant_mask(sq, dirs), "mask mismatch at {sq:?}");
                assert_eq!(m.offset, expected_offset, "offset mismatch at {sq:?}");
                expected_offset += 1 << m.mask.count();
            }
            assert!(
                expected_offset <= table_size,
                "total table entries {expected_offset} exceeds reserved size {table_size}"
            );
        }
    }

    /// The property that actually matters for correctness: `find_magic`'s
    /// result must hash every occupancy subset of the mask to a slot, such that
    /// two subsets sharing a slot always have the *same* real attack set
    /// (constructive collisions — see the module doc — are fine; anything else
    /// is a broken magic). Checked on a few squares chosen to cover both the
    /// worst-case (12-bit rook, 9-bit bishop) and a typical interior mask, not
    /// exhaustively over all 64 — `find_all_magics_offsets_are_a_correct_prefix_sum_of_popcounts`
    /// plus `build_table_matches_attacks_for_occupancy_at_every_real_occupancy`
    /// below cover the full 64-square, every-occupancy case together.
    #[test]
    fn find_magic_produces_a_collision_free_hash_for_a_few_representative_squares() {
        let cases = [
            (Square::A1, ROOK_DIRS),   // worst-case rook mask (12 bits)
            (Square::D4, ROOK_DIRS),   // typical interior rook mask (10 bits)
            (Square::H8, BISHOP_DIRS), // corner bishop (5 bits)
            (Square::D4, BISHOP_DIRS), // worst-case bishop mask (9 bits)
        ];
        for (sq, dirs) in cases {
            let (m, _) = find_magic(sq, dirs, SEED);

            // Carry-Rippler over every subset of mask.
            let mut slots: Vec<Option<Bitboard>> = vec![None; 1usize << m.mask.count()];
            let mask_bits = m.mask.bits();
            let mut sub = 0u64;
            loop {
                let occ = Bitboard::from_bits(sub);
                let attacks = attacks_for_occupancy(sq, occ, dirs);
                let idx = magic_index(occ, &m);
                if let Some(existing) = slots[idx] {
                    assert_eq!(
                        existing, attacks,
                        "real collision at {sq:?} slot {idx}: occupancy {sub:#x}"
                    );
                } else {
                    slots[idx] = Some(attacks);
                }
                if sub == mask_bits {
                    break;
                }
                sub = sub.wrapping_sub(mask_bits) & mask_bits;
            }
        }
    }

    /// End-to-end: every real occupancy of every square, looked up through the
    /// actual built table via `magic_index` + `offset`, matches
    /// `attacks_for_occupancy`'s ground truth directly. This is the full
    /// 102,400-lookup rook check `find_magic_produces_a_collision_free_hash_for_a_few_representative_squares`
    /// only sampled — running it here, at native `#[test]` speed rather than in
    /// const-eval, is the entire reason the search moved offline (see the
    /// module doc's "Magic search runs offline" section).
    #[test]
    fn build_table_matches_attacks_for_occupancy_at_every_real_occupancy() {
        for (dirs, table_size) in [
            (ROOK_DIRS, ROOK_TABLE_SIZE),
            (BISHOP_DIRS, BISHOP_TABLE_SIZE),
        ] {
            let magics = find_all_magics(dirs, SEED);
            let table: Vec<Bitboard> = match table_size {
                ROOK_TABLE_SIZE => build_table::<ROOK_TABLE_SIZE>(dirs, &magics).to_vec(),
                _ => build_table::<BISHOP_TABLE_SIZE>(dirs, &magics).to_vec(),
            };
            for sq in Square::ALL {
                let m = &magics[sq.index() as usize];
                let mask_bits = m.mask.bits();
                let mut sub = 0u64;
                loop {
                    let occ = Bitboard::from_bits(sub);
                    let expected = attacks_for_occupancy(sq, occ, dirs);
                    let idx = m.offset + magic_index(occ, m);
                    assert_eq!(
                        table[idx], expected,
                        "table mismatch at {sq:?}, occupancy {sub:#x}"
                    );
                    if sub == mask_bits {
                        break;
                    }
                    sub = sub.wrapping_sub(mask_bits) & mask_bits;
                }
            }
        }
    }

    /// Keeps the committed data honest: re-runs the search from `SEED` and
    /// confirms it reproduces `ROOK_MAGICS`/`BISHOP_MAGICS` exactly, then
    /// rebuilds each table from scratch and confirms it matches
    /// `ROOK_ATTACKS`/`BISHOP_ATTACKS` (decoded from the committed `.bin`
    /// files) byte for byte — the real, full-scale round-trip through `decode`,
    /// not just the toy buffer in
    /// `decode_reinterprets_little_endian_bytes_as_bitboards`. If this ever
    /// fails, the committed data and the search/build code have drifted apart.
    #[test]
    fn regenerating_reproduces_the_committed_magic_data() {
        assert_eq!(
            find_all_magics(ROOK_DIRS, SEED),
            ROOK_MAGICS,
            "rook magics no longer reproduce from SEED"
        );
        assert_eq!(
            find_all_magics(BISHOP_DIRS, SEED),
            BISHOP_MAGICS,
            "bishop magics no longer reproduce from SEED"
        );

        let rebuilt_rook: [Bitboard; ROOK_TABLE_SIZE] = build_table(ROOK_DIRS, &ROOK_MAGICS);
        let rebuilt_bishop: [Bitboard; BISHOP_TABLE_SIZE] =
            build_table(BISHOP_DIRS, &BISHOP_MAGICS);
        assert_eq!(
            rebuilt_rook, ROOK_ATTACKS,
            "rook_attacks.bin no longer matches a fresh build"
        );
        assert_eq!(
            rebuilt_bishop, BISHOP_ATTACKS,
            "bishop_attacks.bin no longer matches a fresh build"
        );
    }
}
