//! The magic search, table build, and the tests that keep `magics.rs`'s
//! committed data honest. Entirely `#[cfg(test)]`: every item here either
//! feeds the search (`ROOK_DIRS`, `BISHOP_DIRS`, `SEED`, `relevant_mask`,
//! `attacks_for_occupancy`) or is only called from a test
//! (`find_magic`, `find_all_magics`, `build_table`); the real lookup path in
//! `super` needs none of it, only the already-committed `ROOK_MAGICS`/
//! `BISHOP_MAGICS`. Lives in `src/` rather than `tests/magic_props.rs` because
//! everything it exercises (`Magic`, `magic_index`, and the functions above)
//! is private: `tests/*.rs` compiles as a separate crate that only sees `pub`
//! items, so this code genuinely cannot live anywhere else.
//!
//! The search itself doesn't run as a `const fn`, or at build time: a spike
//! measured a single worst-case square's table build at 35.5s inside
//! const-eval, which doesn't scale to 128 squares inside `cargo build`. It
//! runs here instead, as three `#[test]`s
//! (`find_all_magics_offsets_are_a_correct_prefix_sum_of_popcounts`,
//! `build_table_matches_attacks_for_occupancy_at_every_real_occupancy`, and
//! `regenerating_reproduces_the_committed_magic_data`). Each re-runs the
//! full randomized-candidate search, verifying every candidate against
//! every occupancy subset of the mask, for both piece types across all 64
//! squares, so all three are `#[ignore]`d; run them deliberately with
//! `cargo nextest run --workspace --run-ignored all --release`.

use super::*;
use crate::Direction;
use proptest::prelude::*;
use turox_rng::xorshift64star;

/// The four rook ray directions, as a plain data array rather than a piece
/// distinction the code has to branch on: every generic helper below takes a
/// `[Direction; 4]` (rook's or bishop's) and does the same work either way.
/// Only the regeneration test (`regenerating_reproduces_the_committed_magic_data`
/// and friends) still calls into the search/build machinery this feeds; the
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
/// Not every square's slice is this long: `Magic::offset` plus this square's
/// actual `1 << mask.count_ones()` is what `ROOK_ATTACKS` actually reserves for
/// it; this is only the sum's upper bound, used to size that flat array.
const ROOK_TABLE_SIZE: usize = 102_400;

/// `1 << 9`: the worst-case bishop mask popcount (a bishop on one of the four
/// central squares). Same "sum of actual per-square sizes, not 64× the max"
/// relationship to `BISHOP_ATTACKS` as `ROOK_TABLE_SIZE` has to `ROOK_ATTACKS`.
const BISHOP_TABLE_SIZE: usize = 5_248;

/// Fixed PRNG seed for the magic search, arbitrary but fixed, so the search is
/// byte-for-byte reproducible across runs and platforms (same reasoning
/// `board::zobrist`'s TODO commits to for its own key table). Must be nonzero:
/// `xorshift64star` treats 0 as a fixed point (see `rng`'s module doc). Reuses
/// the same 64-bit golden-ratio constant `benches/bitboard.rs` already seeds its
/// sampling PRNG with (a recognizable, well-mixing nonzero value), not a shared
/// RNG state between the two (each owns its own PRNG stream from here).
/// Regen-test-only, same as `ROOK_DIRS`.
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

/// The relevant occupancy mask for a slider on `sq` moving along `dirs`:
/// squares whose occupancy can actually change the attack set. Generic over
/// `dirs` rather than branching on piece: `relevant_mask(sq, ROOK_DIRS)` and
/// `relevant_mask(sq, BISHOP_DIRS)` are the same code path.
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
/// has fewer than 6 set bits (cheap prefilter before the expensive check: the
/// index is taken from the top of that product, so a sparse top byte means the
/// mask's bits are spread poorly across it), then
/// verify by hand-walking every subset of `mask` (Carry-Rippler, since
/// `Bitboard::subsets()` isn't `const fn`) and confirming `magic_index` never
/// collides two subsets whose `attacks_for_occupancy` results genuinely
/// differ. *Constructive* collisions (two occupancies that hash to the same
/// slot but happen to produce the *same* attack set) are fine, and in fact
/// required: rejecting those too would make minimal-size magics nearly
/// unfindable. Retry with the next candidate only on a real collision.
///
/// Returns the winning `Magic` alongside the PRNG's state after finding it, so
/// `find_all_magics` can thread one continuously-advancing stream across all 64
/// squares instead of restarting every square from the same `state`.
///
/// Gives up and returns `None` after generating `max_candidates` of them. A
/// search that cannot succeed has no other way to end: a wrong `magic_index`
/// leaves every candidate genuinely colliding, and without a budget the loop
/// runs until something outside kills it, which reaches the developer as a
/// test that never finishes rather than one that fails. The PRNG state
/// advances across a failed search just as it does across a successful one, so
/// a caller threading one stream does not retry the same candidates on the
/// next square.
fn find_magic(
    sq: Square,
    dirs: [Direction; 4],
    state: u64,
    max_candidates: u32,
) -> Option<(Magic, u64)> {
    let mask = relevant_mask(sq, dirs);
    // A bishop mask has 5 to 9 bits and a rook mask 10 to 12, which is what
    // holds the subset walk below to at most 4096 steps. The candidate budget
    // cannot stand in for that bound: the walk runs inside the verification of
    // a single candidate, so the budget is never reached. A mask that a broken
    // `Bitboard` or `Square` has widened escapes in two directions at once,
    // since `1 << count` is both the walk's length and the size of the
    // allocation tracking its slots.
    assert!(
        (5..=12).contains(&mask.count()),
        "relevant_mask on {sq:?} along {dirs:?} has {} bits, outside the 5 to 12 a slider mask can have: a Bitboard or Square primitive is broken",
        mask.count()
    );

    let mut state = state;
    // Candidates *generated*, not candidates verified, so the budget also
    // bounds the generation loop below, whose exit depends on the prefilter
    // accepting a candidate rather than on a fixed trip count.
    let mut generated: u32 = 0;

    'magic_search: loop {
        // cheap prefilter for magic
        let magic = loop {
            if generated == max_candidates {
                return None;
            }
            let state1 = xorshift64star(state);
            let state2 = xorshift64star(state1);
            let state3 = xorshift64star(state2);
            state = state3; // keep the rotating state for further iteration
            generated += 1;

            let candidate = state1 & state2 & state3;
            if (mask.bits().wrapping_mul(candidate) & 0xFF00_0000_0000_0000).count_ones() >= 6 {
                break candidate;
            }
        };

        // Carry-Rippler: walk every subset of `mask`, checking that no two
        // subsets with genuinely different attack sets hash to the same slot.
        let m = Magic {
            mask,
            magic,
            shift: 64 - mask.count(),
            offset: 0,
        };
        // Indexable directly by `magic_index`'s result: `1 << mask.count()` is
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
                return Some((m, state));
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
}

/// Runs `find_magic` for all 64 squares along `dirs`, threading one
/// continuously-advancing PRNG stream (seeded from `seed`) across all of them
/// rather than restarting each square from the same state, and assembles the
/// result into a `[Magic; 64]`, including each square's `offset`, a running
/// total of `1 << mask.count_ones()` over the squares before it (so
/// `offset[0] == 0`, and the last square's `offset + (1 << popcount)` is the
/// flat table's real total size, `<=` `ROOK_TABLE_SIZE`/`BISHOP_TABLE_SIZE`).
/// Regen-test-only now that `ROOK_MAGICS`/`BISHOP_MAGICS` are committed; see
/// `regenerating_reproduces_the_committed_magic_data`.
fn find_all_magics(dirs: [Direction; 4], seed: u64) -> [Magic; 64] {
    let mut m: [Magic; 64] = [Magic::default(); 64];
    let mut offset = 0;
    let mut state = seed;
    for sq in Square::ALL {
        let (mut magic, next_state) = find_magic(sq, dirs, state, MAX_MAGIC_CANDIDATES)
            .unwrap_or_else(|| {
                panic!("no magic found for {sq:?} along {dirs:?}: its mask or its index is wrong")
            });
        state = next_state;
        magic.offset = offset;
        offset += 1 << magic.mask.count();
        m[sq.index()] = magic;
    }
    m
}

/// Builds the full flat attack table for `dirs`, given every square's
/// already-found `magics` (with `offset`s already assigned by
/// `find_all_magics`): for each square, hand-walk every subset of its mask
/// (same Carry-Rippler as `find_magic`'s verification step) and write
/// `attacks_for_occupancy(sq, subset, dirs)` into
/// `table[magics[sq].offset + magic_index(subset, &magics[sq])]`. Slots no
/// square's magic ever produces are unused padding: `Bitboard::EMPTY` is a
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
        let m = &magics[sq.index()];
        let bits = m.mask.bits();
        let mut sub = 0u64;
        loop {
            let occupied = Bitboard::from_bits(sub);
            let attacks = attacks_for_occupancy(sq, occupied, dirs);
            let idx = magic_index(occupied, m);
            table[magics[sq.index()].offset + idx] = attacks;
            if sub == bits {
                break;
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
    table
}

fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_u8(i).expect("i in 0..64"))
}

fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

/// Either piece's direction set: every property below holds for both, so
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
/// rather than shared; an independent reference that imported the
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

#[test]
fn relevant_mask_matches_naive_walk() {
    for sq in Square::ALL {
        for dirs in [ROOK_DIRS, BISHOP_DIRS] {
            assert_eq!(
                relevant_mask(sq, dirs),
                naive_mask(sq, dirs),
                "sq={sq:?} dirs={dirs:?}"
            );
        }
    }
}

#[test]
fn relevant_mask_never_contains_its_own_square() {
    for sq in Square::ALL {
        for dirs in [ROOK_DIRS, BISHOP_DIRS] {
            assert!(
                !relevant_mask(sq, dirs).contains(sq),
                "sq={sq:?} dirs={dirs:?}"
            );
        }
    }
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
/// property that benefits from random sampling: the standard published
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
/// running total of `1 << mask.count()` over the squares before it; that's
/// what makes each square's slice land at a distinct, correctly-sized
/// region of the flat table. Checked against `relevant_mask` directly
/// (independent of whatever `find_all_magics` internally does to compute
/// the same masks), and the grand total confirmed to fit the reserved
/// `ROOK_TABLE_SIZE`/`BISHOP_TABLE_SIZE`; the exact totals this project's
/// design research measured (102,400 / 5,248), which is what those two
/// constants were sized from in the first place.
#[test]
#[ignore = "full magic search: many randomized candidate multipliers per \
            square, each verified against every occupancy subset of the \
            mask, for all 64 squares of both piece types; run with \
            --release via --run-ignored all"]
fn find_all_magics_offsets_are_a_correct_prefix_sum_of_popcounts() {
    for (dirs, table_size) in [
        (ROOK_DIRS, ROOK_TABLE_SIZE),
        (BISHOP_DIRS, BISHOP_TABLE_SIZE),
    ] {
        let magics = find_all_magics(dirs, SEED);
        let mut expected_offset = 0;
        for sq in Square::ALL {
            let m = &magics[sq.index()];
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

/// A search that cannot succeed must give up and say so, rather than run
/// forever. Without a budget, a wrong `magic_index` means no candidate ever
/// verifies, and every caller of `find_magic` hangs instead of failing: a real
/// bug in `Bitboard`, `Square` or `Move` reaches the developer as a test gate
/// that never finishes, carrying no indication of which assertion was wrong.
///
/// A budget of ten is far below what rook `a1` needs from `SEED` (about half a
/// million candidates). Not every square would do: the cheapest bishop squares
/// find a magic within a handful.
#[test]
fn find_magic_gives_up_when_its_candidate_budget_runs_out() {
    assert_eq!(
        find_magic(Square::A1, ROOK_DIRS, SEED, 10),
        None,
        "a budget far below what any square's search needs must return None, not keep searching"
    );
}

/// The budget cannot catch a mask that is too wide, because the walk it would
/// have to bound runs inside a single candidate's verification. A direction set
/// no real slider has is the only way to reach that condition from outside, and
/// it stands in for the primitive bug that would otherwise produce it.
#[test]
#[should_panic(expected = "outside the 5 to 12 a slider mask can have")]
fn find_magic_rejects_a_mask_wider_than_any_slider() {
    let dirs = [
        Direction::North,
        Direction::East,
        Direction::NorthEast,
        Direction::North,
    ];
    let _ = find_magic(Square::A1, dirs, SEED, MAX_MAGIC_CANDIDATES);
}

/// The property that actually matters for correctness: `find_magic`'s
/// result must hash every occupancy subset of the mask to a slot, such that
/// two subsets sharing a slot always have the *same* real attack set
/// (constructive collisions, per `find_magic`'s doc, are fine; anything
/// else is a broken magic). Checked on a few squares covering the widest and
/// narrowest mask of each piece, not exhaustively over all 64:
/// `find_all_magics_offsets_are_a_correct_prefix_sum_of_popcounts` plus
/// `build_table_matches_attacks_for_occupancy_at_every_real_occupancy` below
/// cover the full 64-square, every-occupancy case together.
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
    let cases = [
        (Square::H8, ROOK_DIRS),   // widest rook mask (12 bits)
        (Square::B2, ROOK_DIRS),   // narrowest rook mask (10 bits)
        (Square::E4, BISHOP_DIRS), // widest bishop mask (9 bits)
        (Square::A5, BISHOP_DIRS), // narrowest bishop mask (5 bits)
    ];
    for (sq, dirs) in cases {
        let (m, _) = find_magic(sq, dirs, SEED, MAX_MAGIC_CANDIDATES)
            .expect("every square has a magic findable inside the budget");

        let high_bits = (m.mask.bits().wrapping_mul(m.magic) & 0xFF00_0000_0000_0000).count_ones();
        assert!(
            high_bits >= 6,
            "{sq:?} chose a candidate the prefilter should have rejected: {high_bits} high bits"
        );

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
/// only sampled; running it here, at native `#[test]` speed rather than in
/// const-eval, is the entire reason the search runs offline instead of at
/// build time (see the module doc's 35.5s measurement).
#[test]
#[ignore = "full magic search plus a full table build on top of it, both \
            walking every occupancy subset of the mask for all 64 squares \
            of both piece types; run with --release via --run-ignored all"]
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
            let m = &magics[sq.index()];
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

/// Setting this makes `regenerating_reproduces_the_committed_magic_data` write
/// the three artifacts instead of comparing against them.
const REGENERATE_ENV: &str = "TUROX_REGENERATE_MAGICS";

/// `magics.rs` exactly as committed, including the hex grouping `rustfmt` and
/// the `unreadable_literal` lint expect, so a regeneration that changes no
/// multiplier changes no byte.
fn render_magics_source(rook: &[Magic; 64], bishop: &[Magic; 64]) -> String {
    fn grouped_hex(v: u64) -> String {
        format!(
            "0x{:04x}_{:04x}_{:04x}_{:04x}",
            v >> 48,
            (v >> 32) & 0xffff,
            (v >> 16) & 0xffff,
            v & 0xffff
        )
    }
    fn array(name: &str, header: &str, magics: &[Magic; 64]) -> String {
        use std::fmt::Write;
        let mut out = format!("{header}pub(super) const {name}: [Magic; 64] = [\n");
        for m in magics {
            write!(
                out,
                "    Magic {{\n        mask: Bitboard::from_bits({}),\n        magic: {},\n        shift: {},\n        offset: {},\n    }},\n",
                grouped_hex(m.mask.bits()),
                grouped_hex(m.magic),
                m.shift,
                m.offset
            )
            .expect("writing to a String cannot fail");
        }
        out.push_str("];\n");
        out
    }

    let mut out = String::from(
        "//! The committed magic-hash parameters for every square, found by\n\
         //! `regen::find_all_magics` and pinned here so the real lookup path\n\
         //! (`super::rook_attacks`/`bishop_attacks`) never re-runs the search.\n\
         //! `regen::regenerating_reproduces_the_committed_magic_data` re-derives these\n\
         //! from `SEED` and asserts they still match, so this data can't silently\n\
         //! drift from the search that's supposed to produce it. That test also\n\
         //! writes this file, so edit the generator rather than this file.\n\
         \n\
         use super::Magic;\n\
         use crate::types::bitboard::Bitboard;\n\
         \n",
    );
    out.push_str(&array(
        "ROOK_MAGICS",
        "/// Found by `find_all_magics(ROOK_DIRS, SEED)`, see\n\
         /// `regen::regenerating_reproduces_the_committed_magic_data` for the check that\n\
         /// keeps this honest.\n",
        rook,
    ));
    out.push('\n');
    out.push_str(&array(
        "BISHOP_MAGICS",
        "/// Found by `find_all_magics(BISHOP_DIRS, SEED)`, same reproducibility check\n\
         /// as `ROOK_MAGICS`.\n",
        bishop,
    ));
    out
}

/// Little-endian, one `u64` per slot: the layout `decode` reads back.
fn table_bytes(table: &[Bitboard]) -> Vec<u8> {
    table.iter().flat_map(|b| b.bits().to_le_bytes()).collect()
}

/// Keeps the committed data honest, and is also the only thing that writes it.
///
/// Re-runs the search from `SEED`, rebuilds both tables, and renders all three
/// artifacts (`magics.rs` and the two `.bin` tables) exactly as committed. By
/// default it asserts each one matches the committed file byte for byte, which
/// is the full-scale round-trip through `decode` that
/// `decode_reinterprets_little_endian_bytes_as_bitboards` only checks on a toy
/// buffer. With `TUROX_REGENERATE_MAGICS` set it writes them instead.
///
/// One test rather than a separate writer, because an `#[ignore]`d writer would
/// run under `--run-ignored all` and rewrite source files from inside the deep
/// job. Here the deep job only ever compares, and the writer is held to
/// reproducing the existing bytes before it is trusted with new ones.
#[test]
#[ignore = "same full magic search and table build as the other ignored \
            tests here, for both piece types; run with --release via \
            --run-ignored all"]
fn regenerating_reproduces_the_committed_magic_data() {
    let rook_magics = find_all_magics(ROOK_DIRS, SEED);
    let bishop_magics = find_all_magics(BISHOP_DIRS, SEED);
    let rook_table: [Bitboard; ROOK_TABLE_SIZE] = build_table(ROOK_DIRS, &rook_magics);
    let bishop_table: [Bitboard; BISHOP_TABLE_SIZE] = build_table(BISHOP_DIRS, &bishop_magics);

    let artifacts: [(&str, Vec<u8>, &[u8]); 3] = [
        (
            "magics.rs",
            render_magics_source(&rook_magics, &bishop_magics).into_bytes(),
            include_bytes!("magics.rs"),
        ),
        (
            "rook_attacks.bin",
            table_bytes(&rook_table),
            include_bytes!("rook_attacks.bin"),
        ),
        (
            "bishop_attacks.bin",
            table_bytes(&bishop_table),
            include_bytes!("bishop_attacks.bin"),
        ),
    ];

    if std::env::var_os(REGENERATE_ENV).is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/move_gen/magic");
        for (name, rendered, _) in artifacts {
            std::fs::write(dir.join(name), rendered)
                .unwrap_or_else(|e| panic!("writing {name}: {e}"));
        }
        return;
    }

    // The parsed comparison first: on a mismatch it names the square, where
    // the byte comparison below could only say which file.
    assert_eq!(
        rook_magics, ROOK_MAGICS,
        "rook magics no longer reproduce from SEED"
    );
    assert_eq!(
        bishop_magics, BISHOP_MAGICS,
        "bishop magics no longer reproduce from SEED"
    );
    for (name, rendered, committed) in artifacts {
        assert!(
            rendered == committed,
            "{name} no longer matches a fresh regeneration; rerun this test with \
             {REGENERATE_ENV}=1 to rewrite it"
        );
    }
}
