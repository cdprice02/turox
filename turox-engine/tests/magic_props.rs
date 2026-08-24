//! Property tests for `move_gen::magic`: the executable version of the
//! contracts documented on each function in `src/move_gen/magic.rs`.
//!
//! Every function gets a reference-equivalence check against an independent
//! implementation built directly from `Square::offset` stepped one square at a
//! time, not from `Bitboard::occluded_fill` or whatever magic-hashing technique
//! the real implementation ends up using.

use proptest::prelude::*;
use turox_engine::move_gen::magic::{bishop_attacks, queen_attacks, rook_attacks};
use turox_engine::{Bitboard, Square};

fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
}

fn any_bitboard() -> impl Strategy<Value = Bitboard> {
    any::<u64>().prop_map(Bitboard::from_bits)
}

const ROOK_DIRS: [(i8, i8); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Reference definition shared by rook/bishop: from `sq`, step one square at a
/// time along each of `dirs`, including every square visited, stopping
/// (inclusively) at the first occupied square or the board edge.
fn naive_slider_attacks(sq: Square, occupied: Bitboard, dirs: &[(i8, i8)]) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for &(df, dr) in dirs {
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

fn naive_rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    naive_slider_attacks(sq, occupied, &ROOK_DIRS)
}

fn naive_bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    naive_slider_attacks(sq, occupied, &BISHOP_DIRS)
}

proptest! {
    // ---- Rook ----

    #[test]
    fn rook_attacks_matches_naive_walk(sq in any_square(), occupied in any_bitboard()) {
        prop_assert_eq!(rook_attacks(sq, occupied), naive_rook_attacks(sq, occupied));
    }

    #[test]
    fn rook_attacks_never_contains_its_own_square(sq in any_square(), occupied in any_bitboard()) {
        prop_assert!(!rook_attacks(sq, occupied).contains(sq));
    }

    #[test]
    fn rook_attacks_on_fully_occupied_board_has_at_most_four_squares(sq in any_square()) {
        prop_assert!(rook_attacks(sq, Bitboard::ALL).count() <= 4);
    }

    // ---- Bishop ----

    #[test]
    fn bishop_attacks_matches_naive_walk(sq in any_square(), occupied in any_bitboard()) {
        prop_assert_eq!(bishop_attacks(sq, occupied), naive_bishop_attacks(sq, occupied));
    }

    #[test]
    fn bishop_attacks_never_contains_its_own_square(sq in any_square(), occupied in any_bitboard()) {
        prop_assert!(!bishop_attacks(sq, occupied).contains(sq));
    }

    #[test]
    fn bishop_attacks_on_fully_occupied_board_has_at_most_four_squares(sq in any_square()) {
        prop_assert!(bishop_attacks(sq, Bitboard::ALL).count() <= 4);
    }

    // ---- Queen ----

    #[test]
    fn queen_attacks_equals_union_of_rook_and_bishop(sq in any_square(), occupied in any_bitboard()) {
        prop_assert_eq!(
            queen_attacks(sq, occupied),
            rook_attacks(sq, occupied) | bishop_attacks(sq, occupied)
        );
    }

    #[test]
    fn queen_attacks_never_contains_its_own_square(sq in any_square(), occupied in any_bitboard()) {
        prop_assert!(!queen_attacks(sq, occupied).contains(sq));
    }
}
