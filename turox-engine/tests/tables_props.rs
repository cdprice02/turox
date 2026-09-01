//! Property tests for `move_gen::tables`: the executable version of the
//! contracts documented on each function in `src/move_gen/tables.rs`.
//!
//! Every function gets a reference-equivalence check against an independent
//! implementation built directly from `Square::offset`/file-rank arithmetic, not
//! from whatever `Bitboard` primitive (`knight_attacks`, `dilate`,
//! `occluded_fill`, ...) the real implementation ends up using.

mod common;

use common::any_square;
use proptest::prelude::*;
use turox_engine::move_gen::tables::{between, king_attacks, knight_attacks, line, pawn_attacks};
use turox_engine::{Bitboard, Color, File, Rank, Square};

fn any_color() -> impl Strategy<Value = Color> {
    prop_oneof![Just(Color::White), Just(Color::Black)]
}

/// Reference definition of `knight_attacks`: every one of the 8 (df, dr) knight
/// deltas that stays on the board.
fn naive_knight_attacks(sq: Square) -> Bitboard {
    const DELTAS: [(i8, i8); 8] = [
        (1, 2),
        (2, 1),
        (2, -1),
        (1, -2),
        (-1, -2),
        (-2, -1),
        (-2, 1),
        (-1, 2),
    ];
    let mut result = Bitboard::EMPTY;
    for (df, dr) in DELTAS {
        if let Some(target) = sq.offset(df, dr) {
            result = result.with(target);
        }
    }
    result
}

/// Reference definition of `king_attacks`: every one of the 8 unit deltas that
/// stays on the board. Deliberately excludes (0, 0), unlike `Bitboard::dilate`,
/// a king does not attack its own square.
fn naive_king_attacks(sq: Square) -> Bitboard {
    let mut result = Bitboard::EMPTY;
    for df in -1i8..=1 {
        for dr in -1i8..=1 {
            if df == 0 && dr == 0 {
                continue;
            }
            if let Some(target) = sq.offset(df, dr) {
                result = result.with(target);
            }
        }
    }
    result
}

/// Reference definition of `pawn_attacks`: the two diagonal-forward deltas for
/// `color`, dropping whichever fall off the board.
fn naive_pawn_attacks(color: Color, sq: Square) -> Bitboard {
    let dr: i8 = match color {
        Color::White => 1,
        Color::Black => -1,
    };
    let mut result = Bitboard::EMPTY;
    for df in [-1i8, 1i8] {
        if let Some(target) = sq.offset(df, dr) {
            result = result.with(target);
        }
    }
    result
}

/// Reference definition of `between`: walk from `a` toward `b` one square at a
/// time along whichever shared rank/file/diagonal they lie on (if any),
/// collecting every square strictly in between. `a == b` and unaligned pairs
/// both fall out of the same "no such axis" check, rather than being
/// special-cased separately.
fn naive_between(a: Square, b: Square) -> Bitboard {
    let (af, ar) = (i32::from(a.file().index()), i32::from(a.rank().index()));
    let (bf, br) = (i32::from(b.file().index()), i32::from(b.rank().index()));
    let (df, dr) = (bf - af, br - ar);
    if (df == 0 && dr == 0) || (df != 0 && dr != 0 && df.abs() != dr.abs()) {
        return Bitboard::EMPTY;
    }
    let (step_f, step_r) = (df.signum(), dr.signum());
    let mut result = Bitboard::EMPTY;
    let (mut f, mut r) = (af + step_f, ar + step_r);
    while (f, r) != (bf, br) {
        let sq = Square::new(
            File::from_index(f as u8).expect("on the shared line, so in bounds"),
            Rank::from_index(r as u8).expect("on the shared line, so in bounds"),
        );
        result = result.with(sq);
        f += step_f;
        r += step_r;
    }
    result
}

/// Reference definition of `line`: every square on the board collinear with `a`
/// and `b` (via the 2D cross product, which is zero exactly for points on the
/// line through `a` and `b`), or `Bitboard::EMPTY` if `a` and `b` don't share a
/// rank/file/diagonal in the first place.
fn naive_line(a: Square, b: Square) -> Bitboard {
    let (af, ar) = (i32::from(a.file().index()), i32::from(a.rank().index()));
    let (bf, br) = (i32::from(b.file().index()), i32::from(b.rank().index()));
    let (df, dr) = (bf - af, br - ar);
    if (df == 0 && dr == 0) || (df != 0 && dr != 0 && df.abs() != dr.abs()) {
        return Bitboard::EMPTY;
    }
    let mut result = Bitboard::EMPTY;
    for sq in Square::ALL {
        let (sf, sr) = (i32::from(sq.file().index()), i32::from(sq.rank().index()));
        let cross = (sf - af) * dr - (sr - ar) * df;
        if cross == 0 {
            result = result.with(sq);
        }
    }
    result
}

proptest! {
    // ---- Knight attacks ----

    #[test]
    fn knight_attacks_matches_naive_deltas(sq in any_square()) {
        prop_assert_eq!(knight_attacks(sq), naive_knight_attacks(sq));
    }

    // ---- King attacks ----

    #[test]
    fn king_attacks_matches_naive_deltas(sq in any_square()) {
        prop_assert_eq!(king_attacks(sq), naive_king_attacks(sq));
    }

    #[test]
    fn king_attacks_never_contains_its_own_square(sq in any_square()) {
        prop_assert!(!king_attacks(sq).contains(sq));
    }

    #[test]
    fn king_attacks_matches_dilate_minus_self(sq in any_square()) {
        prop_assert_eq!(king_attacks(sq), sq.bitboard().dilate().without(sq));
    }

    // ---- Pawn attacks ----

    #[test]
    fn pawn_attacks_matches_naive_deltas(sq in any_square(), color in any_color()) {
        prop_assert_eq!(pawn_attacks(color, sq), naive_pawn_attacks(color, sq));
    }

    #[test]
    fn pawn_attacks_never_contains_its_own_square(sq in any_square(), color in any_color()) {
        prop_assert!(!pawn_attacks(color, sq).contains(sq));
    }

    #[test]
    fn pawn_attacks_has_at_most_two_squares(sq in any_square(), color in any_color()) {
        prop_assert!(pawn_attacks(color, sq).count() <= 2);
    }

    // ---- between ----

    #[test]
    fn between_matches_naive_walk(a in any_square(), b in any_square()) {
        prop_assert_eq!(between(a, b), naive_between(a, b));
    }

    #[test]
    fn between_is_symmetric(a in any_square(), b in any_square()) {
        prop_assert_eq!(between(a, b), between(b, a));
    }

    #[test]
    fn between_never_contains_its_endpoints(a in any_square(), b in any_square()) {
        let bb = between(a, b);
        prop_assert!(!bb.contains(a));
        prop_assert!(!bb.contains(b));
    }

    #[test]
    fn between_own_square_is_empty(a in any_square()) {
        prop_assert_eq!(between(a, a), Bitboard::EMPTY);
    }

    #[test]
    fn between_is_a_subset_of_line(a in any_square(), b in any_square()) {
        prop_assert_eq!(between(a, b).and_not(line(a, b)), Bitboard::EMPTY);
    }

    // ---- line ----

    #[test]
    fn line_matches_naive_collinearity_scan(a in any_square(), b in any_square()) {
        prop_assert_eq!(line(a, b), naive_line(a, b));
    }

    #[test]
    fn line_is_symmetric(a in any_square(), b in any_square()) {
        prop_assert_eq!(line(a, b), line(b, a));
    }

    #[test]
    fn line_own_square_is_empty(a in any_square()) {
        prop_assert_eq!(line(a, a), Bitboard::EMPTY);
    }

    #[test]
    fn line_contains_both_endpoints_when_aligned(a in any_square(), b in any_square()) {
        let bb = line(a, b);
        if !bb.is_empty() {
            prop_assert!(bb.contains(a));
            prop_assert!(bb.contains(b));
        }
    }
}
