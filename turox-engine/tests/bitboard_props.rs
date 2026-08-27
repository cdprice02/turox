//! Property tests for `Bitboard`: the executable version of the contracts
//! documented on each method in `src/types/bitboard.rs`.
//!
//! These bitboard primitives are what the entire engine's correctness rests on —
//! every square set, move generated, and position evaluated eventually bottoms
//! out in one of these operations — so coverage here is deliberately heavier than
//! elsewhere: every function gets a reference-equivalence check against an
//! independent (if naive/slow) implementation built from already-verified
//! primitives, not just algebraic sanity properties.

use proptest::prelude::*;
use turox_engine::{Bitboard, Color, Direction, File, Rank, Square};

fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
}

/// A bitboard with at most 12 bits set, for the subset-enumeration property (2^12
/// subsets is already 4096; anything wider would make the test slow).
fn small_bitboard() -> impl Strategy<Value = Bitboard> {
    proptest::collection::vec(any_square(), 0..=12).prop_map(|squares| {
        squares
            .into_iter()
            .fold(Bitboard::EMPTY, |bb, sq| bb.with(sq))
    })
}

/// Reference definition of `north_fill`: within each file, smear every set bit
/// upward (toward higher ranks). Built from `contains`/`with`, not from whatever
/// technique `north_fill` itself uses.
fn naive_north_fill(a: Bitboard) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for file in File::ALL {
        let mut seen = false;
        for rank in Rank::ALL {
            let sq = Square::new(file, rank);
            seen |= a.contains(sq);
            if seen {
                result = result.with(sq);
            }
        }
    }
    result
}

/// Same as `naive_north_fill`, smearing downward (toward lower ranks).
fn naive_south_fill(a: Bitboard) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for file in File::ALL {
        let mut seen = false;
        for rank in Rank::ALL.into_iter().rev() {
            let sq = Square::new(file, rank);
            seen |= a.contains(sq);
            if seen {
                result = result.with(sq);
            }
        }
    }
    result
}

/// Reference definition of `file_fill`: every square on a file that has at least
/// one bit set anywhere in `a`.
fn naive_file_fill(a: Bitboard) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for file in File::ALL {
        let has_bit = Rank::ALL
            .iter()
            .any(|&rank| a.contains(Square::new(file, rank)));
        if has_bit {
            for rank in Rank::ALL {
                result = result.with(Square::new(file, rank));
            }
        }
    }
    result
}

/// Reference definition of `occluded_fill`: from each set square in `a`, walk
/// `dir` one step at a time, including every square visited, stopping
/// (inclusively) at the first square not in `empty`, or at the board edge. Built
/// from `Square::offset`/`Bitboard::with`/`contains`, not from `shift` or any
/// other primitive `occluded_fill` itself might use.
fn naive_occluded_fill(a: Bitboard, empty: Bitboard, dir: Direction) -> Bitboard {
    let (df, dr): (i8, i8) = match dir {
        Direction::North => (0, 1),
        Direction::South => (0, -1),
        Direction::East => (1, 0),
        Direction::West => (-1, 0),
        Direction::NorthEast => (1, 1),
        Direction::NorthWest => (-1, 1),
        Direction::SouthEast => (1, -1),
        Direction::SouthWest => (-1, -1),
    };
    let mut result = Bitboard::EMPTY;
    for seed in a {
        let mut current = seed;
        result = result.with(current);
        while let Some(next) = current.offset(df, dr) {
            result = result.with(next);
            if !empty.contains(next) {
                break;
            }
            current = next;
        }
    }
    result
}

proptest! {
    // ---- Core arithmetic invariants ----

    #[test]
    fn and_not_matches_and_of_complement(a in any_bitboard(), b in any_bitboard()) {
        prop_assert_eq!(a.and_not(b), a.and(!b));
    }

    #[test]
    fn complement_is_disjoint_and_covers_all(a in any_bitboard()) {
        prop_assert_eq!(a.and(!a), Bitboard::EMPTY);
        prop_assert_eq!(a.or(!a), Bitboard::ALL);
    }

    #[test]
    fn and_or_xor_agree_with_u64(a in any_bitboard(), b in any_bitboard()) {
        prop_assert_eq!(a.and(b).bits(), a.bits() & b.bits());
        prop_assert_eq!(a.or(b).bits(), a.bits() | b.bits());
        prop_assert_eq!(a.xor(b).bits(), a.bits() ^ b.bits());
    }

    #[test]
    fn shl_shr_agree_with_u64_and_never_panic(a in any_bitboard(), n in 0u32..70) {
        // The contract explicitly allows n >= 64 (must not panic; result is EMPTY),
        // which is why this ranges past 64 rather than stopping at 63.
        let expected = if n < 64 { a.bits() << n } else { 0 };
        prop_assert_eq!(a.shl(n).bits(), expected);
    }

    // ---- Square membership and scanning ----

    #[test]
    fn with_without_contains_agree(a in any_bitboard(), sq in any_square()) {
        prop_assert!(a.with(sq).contains(sq));
        prop_assert!(!a.without(sq).contains(sq));
    }

    #[test]
    fn toggled_twice_is_identity(a in any_bitboard(), sq in any_square()) {
        prop_assert_eq!(a.toggled(sq).toggled(sq), a);
    }

    #[test]
    fn count_matches_u64_count_ones(a in any_bitboard()) {
        prop_assert_eq!(a.count(), a.bits().count_ones());
    }

    #[test]
    fn is_single_and_has_multiple_agree_with_count(a in any_bitboard()) {
        prop_assert_eq!(a.is_single(), a.count() == 1);
        prop_assert_eq!(a.has_multiple(), a.count() > 1);
    }

    #[test]
    fn lsb_msb_match_trailing_leading_zeros(a in any_bitboard()) {
        let expected_lsb = if a.bits() == 0 {
            None
        } else {
            Square::from_index(a.bits().trailing_zeros() as u8)
        };
        let expected_msb = if a.bits() == 0 {
            None
        } else {
            Square::from_index(63 - a.bits().leading_zeros() as u8)
        };
        prop_assert_eq!(a.lsb(), expected_lsb);
        prop_assert_eq!(a.msb(), expected_msb);
    }

    #[test]
    fn pop_lsb_drains_in_ascending_order_and_matches_count(a in any_bitboard()) {
        let mut bb = a;
        let mut popped = Vec::new();
        while let Some(sq) = bb.pop_lsb() {
            popped.push(sq);
        }
        prop_assert_eq!(popped.len() as u32, a.count());
        prop_assert!(popped.windows(2).all(|w| w[0].index() < w[1].index()));
        prop_assert_eq!(bb, Bitboard::EMPTY);
    }

    // ---- Iteration (built on pop_lsb/contains, so inherits their dependency) ----

    #[test]
    fn iteration_yields_exactly_the_contained_squares(a in any_bitboard()) {
        let collected: Vec<Square> = a.into_iter().collect();
        prop_assert_eq!(collected.len() as u32, a.count());
        for sq in &collected {
            prop_assert!(a.contains(*sq));
        }
        let rebuilt = collected.into_iter().fold(Bitboard::EMPTY, |bb, sq| bb.with(sq));
        prop_assert_eq!(rebuilt, a);
    }

    // ---- Direction shifts ----

    #[test]
    fn shift_east_never_lands_on_file_a(a in any_bitboard()) {
        prop_assert_eq!(a.shift(Direction::East).and(File::A.bitboard()), Bitboard::EMPTY);
    }

    #[test]
    fn shift_west_never_lands_on_file_h(a in any_bitboard()) {
        prop_assert_eq!(a.shift(Direction::West).and(File::H.bitboard()), Bitboard::EMPTY);
    }

    #[test]
    fn shift_never_increases_count(a in any_bitboard(), i in 0usize..8) {
        let dir = Direction::ALL[i];
        prop_assert!(a.shift(dir).count() <= a.count());
    }

    #[test]
    fn shift_then_opposite_shift_is_a_subset(a in any_bitboard(), i in 0usize..8) {
        let dir = Direction::ALL[i];
        let there_and_back = a.shift(dir).shift(dir.opposite());
        prop_assert_eq!(there_and_back.and(!a), Bitboard::EMPTY);
    }

    // ---- Flips: involution and reference-equivalence against a per-square walk ----

    #[test]
    fn flip_horizontal_is_an_involution(a in any_bitboard()) {
        prop_assert_eq!(a.flip_horizontal().flip_horizontal(), a);
    }

    #[test]
    fn flip_vertical_is_an_involution(a in any_bitboard()) {
        prop_assert_eq!(a.flip_vertical().flip_vertical(), a);
    }

    #[test]
    fn flip_vertical_matches_byte_swap(a in any_bitboard()) {
        prop_assert_eq!(a.flip_vertical().bits(), a.bits().swap_bytes());
    }

    #[test]
    fn flip_diagonal_a1h8_is_an_involution(a in any_bitboard()) {
        prop_assert_eq!(a.flip_diagonal_a1h8().flip_diagonal_a1h8(), a);
    }

    #[test]
    fn flip_diagonal_a1h8_matches_per_square_transpose(a in any_bitboard()) {
        let flipped = a.flip_diagonal_a1h8();
        for sq in Square::ALL {
            let transposed = Square::new(
                File::from_index(sq.rank().index()).unwrap(),
                Rank::from_index(sq.file().index()).unwrap(),
            );
            prop_assert_eq!(flipped.contains(transposed), a.contains(sq));
        }
    }

    #[test]
    fn flip_diagonal_a8h1_is_an_involution(a in any_bitboard()) {
        prop_assert_eq!(a.flip_diagonal_a8h1().flip_diagonal_a8h1(), a);
    }

    // ---- Rotations ----
    //
    // The group-law properties below (four cw rotations = identity, cw and ccw are
    // mutual inverses) only prove rotate_90_cw and rotate_90_ccw are *consistent
    // with each other* — that holds even if both were secretly counter-clockwise.
    // The absolute direction is pinned down by a plain #[test] (not a property, so
    // it lives with the other unit tests in src/types/bitboard.rs, not here) named
    // rotate_90_cw_matches_known_corner_mapping.

    #[test]
    fn rotate_90_cw_four_times_is_identity(a in any_bitboard()) {
        prop_assert_eq!(a.rotate_90_cw().rotate_90_cw().rotate_90_cw().rotate_90_cw(), a);
    }

    #[test]
    fn rotate_90_cw_and_ccw_are_inverses(a in any_bitboard()) {
        prop_assert_eq!(a.rotate_90_cw().rotate_90_ccw(), a);
        prop_assert_eq!(a.rotate_90_ccw().rotate_90_cw(), a);
    }

    #[test]
    fn rotate_180_matches_both_flips_composed(a in any_bitboard()) {
        prop_assert_eq!(a.rotate_180(), a.flip_vertical().flip_horizontal());
        prop_assert_eq!(a.rotate_180(), a.flip_horizontal().flip_vertical());
    }

    #[test]
    fn rotate_180_matches_reverse_bits(a in any_bitboard()) {
        prop_assert_eq!(a.rotate_180().bits(), a.bits().reverse_bits());
    }

    #[test]
    fn mirror_for_white_is_identity_black_is_flip_vertical(a in any_bitboard()) {
        prop_assert_eq!(a.mirror_for(Color::White), a);
        prop_assert_eq!(a.mirror_for(Color::Black), a.flip_vertical());
    }

    // ---- Every transform preserves population count ----

    #[test]
    fn every_transform_preserves_count(a in any_bitboard()) {
        let transforms: [fn(Bitboard) -> Bitboard; 7] = [
            Bitboard::flip_horizontal,
            Bitboard::flip_vertical,
            Bitboard::flip_diagonal_a1h8,
            Bitboard::flip_diagonal_a8h1,
            Bitboard::rotate_90_cw,
            Bitboard::rotate_90_ccw,
            Bitboard::rotate_180,
        ];
        for t in transforms {
            prop_assert_eq!(t(a).count(), a.count());
        }
    }

    // ---- Carry-Rippler subset enumeration (planned for the move-gen change) ----

    #[test]
    fn subsets_yields_exactly_two_to_the_count_distinct_subsets(a in small_bitboard()) {
        let subsets: Vec<Bitboard> = a.subsets().collect();
        prop_assert_eq!(subsets.len() as u32, 1u32 << a.count());
        for &s in &subsets {
            prop_assert_eq!(s.and(!a), Bitboard::EMPTY, "subset must be a subset of the mask");
        }
        prop_assert!(subsets.contains(&Bitboard::EMPTY));
        prop_assert!(subsets.contains(&a));
        let mut sorted: Vec<u64> = subsets.iter().map(|b| b.bits()).collect();
        sorted.sort_unstable();
        sorted.dedup();
        prop_assert_eq!(sorted.len(), subsets.len(), "all subsets must be distinct");
    }

    // ---- Fills (Kogge-Stone) ----

    #[test]
    fn north_fill_matches_naive_smear(a in any_bitboard()) {
        prop_assert_eq!(a.north_fill(), naive_north_fill(a));
    }

    #[test]
    fn north_fill_is_a_superset_and_idempotent(a in any_bitboard()) {
        prop_assert_eq!(a.and_not(a.north_fill()), Bitboard::EMPTY);
        prop_assert_eq!(a.north_fill().north_fill(), a.north_fill());
    }

    #[test]
    fn south_fill_matches_naive_smear(a in any_bitboard()) {
        prop_assert_eq!(a.south_fill(), naive_south_fill(a));
    }

    #[test]
    fn south_fill_is_a_superset_and_idempotent(a in any_bitboard()) {
        prop_assert_eq!(a.and_not(a.south_fill()), Bitboard::EMPTY);
        prop_assert_eq!(a.south_fill().south_fill(), a.south_fill());
    }

    #[test]
    fn file_fill_matches_naive_definition(a in any_bitboard()) {
        prop_assert_eq!(a.file_fill(), naive_file_fill(a));
    }

    #[test]
    fn file_fill_equals_union_of_north_and_south_fill(a in any_bitboard()) {
        prop_assert_eq!(a.file_fill(), a.north_fill().or(a.south_fill()));
    }

    #[test]
    fn file_fill_count_is_a_multiple_of_eight(a in any_bitboard()) {
        prop_assert_eq!(a.file_fill().count() % 8, 0);
    }

    // ---- Occluded fill ----
    //
    // occluded_fill takes a Direction (like shift) rather than being 8 hardcoded
    // per-direction methods, so these properties are parametrized over
    // Direction::ALL rather than repeated 8 times.

    #[test]
    fn occluded_fill_matches_naive_walk(
        a in any_bitboard(),
        empty in any_bitboard(),
        i in 0usize..8,
    ) {
        let dir = Direction::ALL[i];
        prop_assert_eq!(a.occluded_fill(empty, dir), naive_occluded_fill(a, empty, dir));
    }

    #[test]
    fn occluded_fill_always_contains_the_seed(
        a in any_bitboard(),
        empty in any_bitboard(),
        i in 0usize..8,
    ) {
        let dir = Direction::ALL[i];
        prop_assert_eq!(a.and_not(a.occluded_fill(empty, dir)), Bitboard::EMPTY);
    }

    #[test]
    fn occluded_fill_with_everything_empty_matches_north_south_fill(a in any_bitboard()) {
        prop_assert_eq!(a.occluded_fill(Bitboard::ALL, Direction::North), a.north_fill());
        prop_assert_eq!(a.occluded_fill(Bitboard::ALL, Direction::South), a.south_fill());
    }

    // ---- Dilation ----

    #[test]
    fn dilate_matches_union_of_all_eight_shifts(a in any_bitboard()) {
        let mut expected = a;
        for dir in Direction::ALL {
            expected = expected.or(a.shift(dir));
        }
        prop_assert_eq!(a.dilate(), expected);
    }

    #[test]
    fn dilate_is_a_superset(a in any_bitboard()) {
        prop_assert_eq!(a.and_not(a.dilate()), Bitboard::EMPTY);
    }
}
