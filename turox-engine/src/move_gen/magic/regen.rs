//! The magic search, table build, and the tests that keep `magics.rs`'s
//! committed data honest. Entirely `#[cfg(test)]`: every item here either
//! feeds the search (`ROOK_DIRS`, `BISHOP_DIRS`, `SEED`, `relevant_mask`,
//! `attacks_for_occupancy`) or is only called from a test
//! (`find_magic`, `find_all_magics`, `build_table`) — the real lookup path in
//! `super` needs none of it, only the already-committed `ROOK_MAGICS`/
//! `BISHOP_MAGICS`. Lives in `src/` rather than `tests/magic_props.rs` because
//! everything it exercises (`Magic`, `magic_index`, and the functions above)
//! is private: `tests/*.rs` compiles as a separate crate that only sees `pub`
//! items, so this code genuinely cannot live anywhere else.
//!
//! The search itself doesn't run as a `const fn`, or at build time: a spike
//! measured a single worst-case square's table build at 35.5s inside
//! const-eval, which doesn't scale to 128 squares inside `cargo build`. It
//! runs here instead, as a normal `#[test]`
//! (`regenerating_reproduces_the_committed_magic_data`), which re-derives
//! `ROOK_MAGICS`/`BISHOP_MAGICS` from `SEED` on every run and asserts they
//! still match what's committed in `magics.rs`.

use super::*;
use crate::rng::xorshift64star;
use crate::Direction;
use proptest::prelude::*;

/// The four rook ray directions, as a plain data array rather than a piece
/// distinction the code has to branch on — every generic helper below takes a
/// `[Direction; 4]` (rook's or bishop's) and does the same work either way.
/// Only the regeneration test (`regenerating_reproduces_the_committed_magic_data`
/// and friends) still calls into the search/build machinery this feeds — the
/// real lookup path below only needs the already-committed `ROOK_MAGICS`.
const ROOK_DIRS: [Direction; 4] = [
    Direction::North,
    Direction::South,
    Direction::East,
    Direction::West,
];

/// The four bishop ray directions. Same regen-test-only status as `ROOK_DIRS`.
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
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// The relevant occupancy mask for a slider on `sq` moving along `dirs`:
/// squares whose occupancy can actually change the attack set. Generic over
/// `dirs` rather than branching on piece — `relevant_mask(sq, ROOK_DIRS)` and
/// `relevant_mask(sq, BISHOP_DIRS)` are the same code path.
///
/// Drops each ray's terminal square by shifting the full unblocked ray one
/// step further (off the board) and back, rather than reasoning per square
/// about which edge a ray dies on: `occluded_fill(sq.bitboard(), ALL,
/// dir).shift(dir).shift(dir.opposite())`. A rook standing on `FILE_A` has its
/// entire north/south mask living on `FILE_A`, so subtracting that edge
/// outright would wipe out real blocker squares, not just the terminus — the
/// same {axis}×{sign} shape that's bitten `Board::make_move` twice.
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

/// Searches for a magic multiplier for a slider on `sq` along `dirs`, starting
/// the PRNG from `state`: generate a sparse candidate (AND a few successive
/// `xorshift64star` outputs together, biasing toward sparse, better-hashing
/// multipliers), reject early if `(mask.bits() * magic) & 0xFF00_0000_0000_0000`
/// has fewer than 6 set bits (cheap prefilter before the expensive check), then
/// verify by hand-walking every subset of `mask` (Carry-Rippler, since
/// `Bitboard::subsets()` isn't `const fn`) and confirming `magic_index` never
/// collides two subsets whose `attacks_for_occupancy` results genuinely
/// differ. *Constructive* collisions — two occupancies that hash to the same
/// slot but happen to produce the *same* attack set — are fine, and in fact
/// required: rejecting those too would make minimal-size magics nearly
/// unfindable. Retry with the next candidate only on a real collision.
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
/// `regenerating_reproduces_the_committed_magic_data`.
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
/// same reason `find_magic` isn't: table build is search-adjacent work, not
/// the cheap byte-reinterpretation `decode` does. Regen-test-only, same as
/// `find_all_magics`.
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
fn naive_attacks_for_occupancy(sq: Square, occupied: Bitboard, dirs: [Direction; 4]) -> Bitboard {
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
/// (constructive collisions — see `find_magic`'s doc — are fine; anything
/// else is a broken magic). Checked on a few squares chosen to cover both the
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
/// const-eval, is the entire reason the search runs offline instead of at
/// build time (see the module doc's 35.5s measurement).
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
    let rebuilt_bishop: [Bitboard; BISHOP_TABLE_SIZE] = build_table(BISHOP_DIRS, &BISHOP_MAGICS);
    assert_eq!(
        rebuilt_rook, ROOK_ATTACKS,
        "rook_attacks.bin no longer matches a fresh build"
    );
    assert_eq!(
        rebuilt_bishop, BISHOP_ATTACKS,
        "bishop_attacks.bin no longer matches a fresh build"
    );
}
