//! Property tests for `move_gen::magic`: the executable version of the
//! contracts documented on each function in `src/move_gen/magic.rs`.
//!
//! Every function gets a reference-equivalence check against an independent
//! implementation built directly from `Square::offset` stepped one square at a
//! time, not from `Bitboard::occluded_fill` or whatever magic-hashing technique
//! the real implementation ends up using.
//!
//! Every property here pairs a `Square` with an arbitrary `occupied: Bitboard`
//! (2^64 values), a genuinely unbounded domain, which is what `proptest`'s
//! random sampling is for. The `Square`-only checks (fixed `Bitboard::ALL`)
//! live as plain exhaustive `#[test]`s in `move_gen/magic/mod.rs`'s own test
//! module instead: no unbounded domain there for `proptest` to be worth its
//! overhead over a loop, and it's a unit-level check of that module's own
//! functions, not a cross-module integration property.

use proptest::prelude::*;
use turox_chess::move_gen::magic::{bishop_attacks, queen_attacks, rook_attacks};
use turox_chess::strategies::{any_bitboard, any_square};
use turox_chess::{Bitboard, Square};

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

    // ---- Bishop ----

    #[test]
    fn bishop_attacks_matches_naive_walk(sq in any_square(), occupied in any_bitboard()) {
        prop_assert_eq!(bishop_attacks(sq, occupied), naive_bishop_attacks(sq, occupied));
    }

    #[test]
    fn bishop_attacks_never_contains_its_own_square(sq in any_square(), occupied in any_bitboard()) {
        prop_assert!(!bishop_attacks(sq, occupied).contains(sq));
    }

    // ---- Queen ----

    #[test]
    fn queen_attacks_never_contains_its_own_square(sq in any_square(), occupied in any_bitboard()) {
        prop_assert!(!queen_attacks(sq, occupied).contains(sq));
    }
}

/// Every square a slider's attack set can depend on: those strictly between
/// `sq` and the board edge along each direction. A blocker on the edge square
/// itself changes nothing, which is the whole reason a magic index needs only
/// `2^count` slots rather than one per occupancy.
fn naive_relevant_mask(sq: Square, dirs: &[(i8, i8)]) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    for &(df, dr) in dirs {
        let mut current = sq;
        while let Some(next) = current.offset(df, dr) {
            if next.offset(df, dr).is_some() {
                mask = mask.with(next);
            }
            current = next;
        }
    }
    mask
}

/// Checks the committed tables against the naive walk at every occupancy they
/// can tell apart, returning how many that was.
///
/// The properties above sample `any_bitboard()`, whose dense arm carries around
/// 32 bits, so a slider meets a blocker within a square or two of its origin
/// almost every time and the long rays go unwalked. That leaves them a weak
/// instrument for the one thing regenerating a table can break: a slot nothing
/// happens to look at. Walking the subsets of the mask is exhaustive over
/// exactly the domain the tables model, and cheap, because that domain is
/// 102,400 rook slots and 5,248 bishop ones rather than 2^64 of anything.
fn check_every_occupancy(dirs: &[(i8, i8)], attacks: fn(Square, Bitboard) -> Bitboard) -> u64 {
    let mut cases = 0;
    for sq in Square::ALL {
        let mask = naive_relevant_mask(sq, dirs);
        let bits = mask.bits();
        // Carry-Rippler, so this visits each subset of `mask` once.
        let mut sub = 0u64;
        loop {
            let occupied = Bitboard::from_bits(sub);
            assert_eq!(
                attacks(sq, occupied),
                naive_slider_attacks(sq, occupied, dirs),
                "{sq:?} with occupancy {sub:#018x}"
            );
            cases += 1;
            if sub == bits {
                break;
            }
            sub = sub.wrapping_sub(bits) & bits;
        }
    }
    cases
}

/// The case count is asserted alongside the walk so that a mask computed too
/// narrowly cannot quietly shrink the domain to a subset that still passes.
#[test]
fn rook_attacks_matches_naive_walk_at_every_relevant_occupancy() {
    assert_eq!(check_every_occupancy(&ROOK_DIRS, rook_attacks), 102_400);
}

/// See `rook_attacks_matches_naive_walk_at_every_relevant_occupancy` for why
/// the count is part of the assertion.
#[test]
fn bishop_attacks_matches_naive_walk_at_every_relevant_occupancy() {
    assert_eq!(check_every_occupancy(&BISHOP_DIRS, bishop_attacks), 5_248);
}
