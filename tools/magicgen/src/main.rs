//! The `magicgen` binary: searches for a magic multiplier for every square, for
//! rooks and bishops, builds the attack tables those multipliers index, and
//! writes all four into `turox-chess/src/move_gen/magic/`, where the crate
//! includes them at compile time.
//!
//! Nothing checks that the files came from this program. What is checked, on
//! every push, is that they are correct: `turox-chess/tests/magic_props.rs`
//! walks every relevant occupancy of every square through the public lookup
//! and compares it with an independent ray walk. So a regeneration is reviewed
//! by that test passing, not by reading a binary diff.
//!
//! The search runs here rather than as a `const fn` inside the crate because
//! const-eval is far too slow for it: a single worst-case square's table build
//! measured 35.5s there, against a few seconds for all 128 searches here.

use std::path::Path;
use std::process::ExitCode;
use turox_chess::move_gen::magic::{bishop_mask, rook_mask};
use turox_chess::{Bitboard, Direction, Square};
use turox_rng::xorshift64star;

const ROOK_DIRS: [Direction; 4] = [
    Direction::North,
    Direction::South,
    Direction::East,
    Direction::West,
];

const BISHOP_DIRS: [Direction; 4] = [
    Direction::NorthEast,
    Direction::NorthWest,
    Direction::SouthEast,
    Direction::SouthWest,
];

/// `rook_mask` or `bishop_mask`: which piece a search is for.
type MaskFn = fn(Square) -> Bitboard;

/// Fixed PRNG seed, so a regeneration that changes nothing about the search
/// writes the same bytes. Must be nonzero: `xorshift64star` treats 0 as a fixed
/// point. The 64-bit golden-ratio constant, chosen only for being a
/// recognizable, well-mixing nonzero value.
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// How many candidate multipliers one square's search may generate before it
/// gives up.
///
/// Set far above what any square actually spends, because what a square spends
/// is a property of `SEED` and the prefilter rather than anything a reader can
/// bound by inspection: the most expensive of the 128 searches generates a few
/// million candidates, and a reseed redistributes that without changing the
/// total much. The margin is what keeps this a backstop against a broken mask
/// or index rather than a limit a correct regeneration has to be tuned around.
///
/// Generating a candidate costs three `xorshift64star` rounds, a multiply and a
/// popcount, so exhausting the whole budget costs well under a second. The
/// expensive step is verification, and a broken `magic_index` collides on one of
/// its first few subsets, so a failing search reaches this limit without ever
/// paying for a full subset walk.
const MAX_MAGIC_CANDIDATES: u32 = 16_000_000;

/// One square's hash parameters, laid out the way `turox-chess` derives them
/// from the multiplier: `shift` is `64 - mask.count()`, and `offset` is where
/// the square's slice starts in the flat table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "`magic` is the chess-programming term for the multiplier, and the name turox-chess uses for the same field"
)]
struct Magic {
    mask: Bitboard,
    magic: u64,
    shift: u32,
    offset: usize,
}

/// The magic-bitboard hash, local to `m`'s slice. A second copy of the formula
/// `turox-chess` looks up with, since that one is private to the crate. The two
/// cannot disagree unnoticed: tables built with a different formula fail the
/// crate's exhaustive lookup test.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "m.shift always leaves <= 12 significant bits, which fits usize"
)]
const fn magic_index(occupied: Bitboard, m: &Magic) -> usize {
    ((occupied.and(m.mask).bits().wrapping_mul(m.magic)) >> m.shift) as usize
}

/// The actual attack set for a slider on `sq` moving along `dirs`, given a real
/// (not mask-restricted) `occupied`: what each table slot must hold, and what a
/// candidate's hash is verified against.
const fn attacks_for_occupancy(sq: Square, occupied: Bitboard, dirs: [Direction; 4]) -> Bitboard {
    let mut attacks = Bitboard::EMPTY;
    let mut i = 0;
    while i < dirs.len() {
        attacks = attacks.or(sq.bitboard().occluded_fill(occupied.not(), dirs[i]));
        i += 1;
    }
    attacks.without(sq)
}

/// Searches for a magic multiplier for a slider on `sq` with relevant-occupancy
/// `mask`, moving along `dirs`, starting the PRNG from `state`: generate a
/// sparse candidate (AND a few successive `xorshift64star` outputs together,
/// biasing toward sparse, better-hashing multipliers), reject early if
/// `(mask.bits() * magic) & 0xFF00_0000_0000_0000` has fewer than 6 set bits
/// (cheap prefilter before the expensive check: the index is taken from the top
/// of that product, so a sparse top byte means the mask's bits are spread
/// poorly across it), then verify by walking every subset of `mask`
/// (Carry-Rippler) and confirming `magic_index` never collides two subsets whose
/// attack sets genuinely differ. *Constructive* collisions (two occupancies that
/// share a slot and the same attack set) are fine, and in fact required:
/// rejecting those too would make minimal-size magics nearly unfindable.
///
/// Returns the winning `Magic` alongside the PRNG's state after finding it, so
/// `find_all_magics` can thread one continuously-advancing stream across all 64
/// squares instead of restarting every square from the same `state`.
///
/// Gives up and returns `None` after generating `max_candidates` of them. A
/// search that cannot succeed has no other way to end: a wrong `magic_index`
/// leaves every candidate genuinely colliding, and without a budget the loop
/// runs until something outside kills it.
fn find_magic(
    sq: Square,
    mask: Bitboard,
    dirs: [Direction; 4],
    state: u64,
    max_candidates: u32,
) -> Option<(Magic, u64)> {
    // A bishop mask has 5 to 9 bits and a rook mask 10 to 12, which is what
    // holds the subset walk below to at most 4096 steps. The candidate budget
    // cannot stand in for that bound: the walk runs inside the verification of
    // a single candidate, so the budget is never reached. A mask that a broken
    // `Bitboard` or `Square` has widened escapes in two directions at once,
    // since `1 << count` is both the walk's length and the size of the
    // allocation tracking its slots.
    assert!(
        (5..=12).contains(&mask.count()),
        "the mask on {sq:?} has {} bits, outside the 5 to 12 a slider mask can have: a Bitboard or Square primitive is broken",
        mask.count()
    );

    let mut state = state;
    // Candidates *generated*, not candidates verified, so the budget also
    // bounds the generation loop below, whose exit depends on the prefilter
    // accepting a candidate rather than on a fixed trip count.
    let mut generated: u32 = 0;

    'magic_search: loop {
        let magic = loop {
            if generated == max_candidates {
                return None;
            }
            let state1 = xorshift64star(state);
            let state2 = xorshift64star(state1);
            let state3 = xorshift64star(state2);
            state = state3;
            generated += 1;

            let candidate = state1 & state2 & state3;
            if (mask.bits().wrapping_mul(candidate) & 0xFF00_0000_0000_0000).count_ones() >= 6 {
                break candidate;
            }
        };

        let m = Magic {
            mask,
            magic,
            shift: 64 - mask.count(),
            offset: 0,
        };
        // Indexable directly by `magic_index`'s result: `1 << mask.count()` is
        // exactly the number of distinct slots that hash can produce for this
        // mask and shift.
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
                return Some((m, state));
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
}

/// Runs `find_magic` for all 64 squares, threading one PRNG stream from `SEED`
/// across all of them, and assigns each square's `offset` as the running total
/// of `1 << mask.count()` over the squares before it. Returns the first square
/// whose search exhausted its budget, if any.
fn find_all_magics(mask: MaskFn, dirs: [Direction; 4]) -> Result<[Magic; 64], Square> {
    let mut magics = [Magic {
        mask: Bitboard::EMPTY,
        magic: 0,
        shift: 0,
        offset: 0,
    }; 64];
    let mut offset = 0;
    let mut state = SEED;
    for sq in Square::ALL {
        let (mut magic, next_state) =
            find_magic(sq, mask(sq), dirs, state, MAX_MAGIC_CANDIDATES).ok_or(sq)?;
        state = next_state;
        magic.offset = offset;
        offset += 1 << magic.mask.count();
        magics[sq.index()] = magic;
    }
    Ok(magics)
}

/// Builds the flat attack table for `magics`: every subset of every square's
/// mask, written at that square's `offset` plus its hash. Slots no subset
/// reaches stay `Bitboard::EMPTY`, which is never a real answer, since a slider
/// always attacks at least the square that boxes it in.
fn build_table(magics: &[Magic; 64], dirs: [Direction; 4]) -> Vec<Bitboard> {
    let last = &magics[63];
    let mut table = vec![Bitboard::EMPTY; last.offset + (1 << last.mask.count())];
    for sq in Square::ALL {
        let m = &magics[sq.index()];
        let bits = m.mask.bits();
        let mut sub = 0u64;
        loop {
            let occupied = Bitboard::from_bits(sub);
            table[m.offset + magic_index(occupied, m)] = attacks_for_occupancy(sq, occupied, dirs);
            if sub == bits {
                break;
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
    table
}

/// Little-endian, one `u64` per entry: what the crate's `decode` reads back.
fn le_bytes(values: impl Iterator<Item = u64>) -> Vec<u8> {
    values.flat_map(u64::to_le_bytes).collect()
}

/// Searches one piece's magics and returns its two files' contents: the 64
/// multipliers, and the attack table they index.
fn generate(mask: MaskFn, dirs: [Direction; 4]) -> Result<(Vec<u8>, Vec<u8>), Square> {
    let magics = find_all_magics(mask, dirs)?;
    let table = build_table(&magics, dirs);
    Ok((
        le_bytes(magics.iter().map(|m| m.magic)),
        le_bytes(table.iter().map(|b| b.bits())),
    ))
}

fn main() -> ExitCode {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../turox-chess/src/move_gen/magic");
    let pieces: [(&str, MaskFn, [Direction; 4]); 2] = [
        ("rook", rook_mask, ROOK_DIRS),
        ("bishop", bishop_mask, BISHOP_DIRS),
    ];
    for (piece, mask, dirs) in pieces {
        let (multipliers, table) = match generate(mask, dirs) {
            Ok(files) => files,
            Err(sq) => {
                eprintln!(
                    "magicgen: no {piece} magic found for {sq:?} within {MAX_MAGIC_CANDIDATES} candidates: its mask or the index is wrong"
                );
                return ExitCode::FAILURE;
            }
        };
        for (name, bytes) in [
            (format!("{piece}_magics.bin"), multipliers),
            (format!("{piece}_attacks.bin"), table),
        ] {
            let path = out.join(&name);
            if let Err(e) = std::fs::write(&path, bytes) {
                eprintln!("magicgen: writing {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
            println!("wrote {name}");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use turox_chess::strategies::{any_bitboard, any_square};

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

    /// Independent reference for `attacks_for_occupancy`: step one square at a
    /// time along each of `dirs`, stopping (inclusively) at the first occupied
    /// square or the board edge. Built from `Square::offset` rather than
    /// `occluded_fill`, or it would not be independent.
    fn naive_attacks(sq: Square, occupied: Bitboard, dirs: [Direction; 4]) -> Bitboard {
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

    fn any_dirs() -> impl Strategy<Value = [Direction; 4]> {
        prop_oneof![Just(ROOK_DIRS), Just(BISHOP_DIRS)]
    }

    proptest! {
        #[test]
        fn attacks_for_occupancy_matches_naive_walk(
            sq in any_square(),
            occupied in any_bitboard(),
            dirs in any_dirs(),
        ) {
            prop_assert_eq!(
                attacks_for_occupancy(sq, occupied, dirs),
                naive_attacks(sq, occupied, dirs)
            );
        }
    }

    /// A search that cannot succeed must give up and say so, rather than run
    /// forever.
    ///
    /// A budget of ten is far below what rook `a1` needs from `SEED` (about half a
    /// million candidates). Not every square would do: the cheapest bishop squares
    /// find a magic within a handful.
    #[test]
    fn find_magic_gives_up_when_its_candidate_budget_runs_out() {
        assert_eq!(
            find_magic(Square::A1, rook_mask(Square::A1), ROOK_DIRS, SEED, 10),
            None,
            "a budget far below what any square's search needs must return None, not keep searching"
        );
    }

    /// The budget cannot catch a mask that is too wide, because the walk it would
    /// have to bound runs inside a single candidate's verification.
    #[test]
    #[should_panic(expected = "outside the 5 to 12 a slider mask can have")]
    fn find_magic_rejects_a_mask_wider_than_any_slider() {
        let wide = rook_mask(Square::A1).or(bishop_mask(Square::A1));
        let _ = find_magic(Square::A1, wide, ROOK_DIRS, SEED, MAX_MAGIC_CANDIDATES);
    }

    /// The property that matters for correctness: every occupancy subset of the
    /// mask hashes to a slot shared only with subsets of the same attack set.
    /// Checked on the widest and narrowest mask of each piece; the full 64-square
    /// check is the crate's exhaustive lookup test, run against the tables this
    /// program writes.
    ///
    /// Within each width, the square is the one whose search from `SEED` is
    /// cheapest, because this test runs on every push and a fresh search's cost
    /// varies by over a hundredfold between squares of the same width (rook `a1`
    /// generates about 526,000 candidates, rook `h8` about 142,000). A reseed
    /// reshuffles which squares are cheap, so it is worth rechecking these then.
    ///
    /// Also pins the prefilter's direction: the chosen multiplier must have at
    /// least six bits in the top byte of `mask * magic`. The prefilter only affects
    /// which collision-free candidate wins, so nothing else can see it invert.
    #[test]
    fn find_magic_produces_a_collision_free_hash_for_a_few_representative_squares() {
        let rook = |sq| (sq, rook_mask(sq), ROOK_DIRS);
        let bishop = |sq| (sq, bishop_mask(sq), BISHOP_DIRS);
        let cases = [
            rook(Square::H8),   // widest rook mask (12 bits)
            rook(Square::B2),   // narrowest rook mask (10 bits)
            bishop(Square::E4), // widest bishop mask (9 bits)
            bishop(Square::A5), // narrowest bishop mask (5 bits)
        ];
        for (sq, mask, dirs) in cases {
            let (m, _) = find_magic(sq, mask, dirs, SEED, MAX_MAGIC_CANDIDATES)
                .expect("every square has a magic findable inside the budget");

            let high_bits =
                (m.mask.bits().wrapping_mul(m.magic) & 0xFF00_0000_0000_0000).count_ones();
            assert!(
                high_bits >= 6,
                "{sq:?} chose a candidate the prefilter should have rejected: {high_bits} high bits"
            );

            let mut slots: Vec<Option<Bitboard>> = vec![None; 1usize << m.mask.count()];
            let bits = m.mask.bits();
            let mut sub = 0u64;
            loop {
                let occupied = Bitboard::from_bits(sub);
                let attacks = attacks_for_occupancy(sq, occupied, dirs);
                let idx = magic_index(occupied, &m);
                if let Some(existing) = slots[idx] {
                    assert_eq!(
                        existing, attacks,
                        "real collision at {sq:?} slot {idx}: occupancy {sub:#x}"
                    );
                } else {
                    slots[idx] = Some(attacks);
                }
                if sub == bits {
                    break;
                }
                sub = sub.wrapping_sub(bits) & bits;
            }
        }
    }
}
