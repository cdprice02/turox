//! Attack lookups for the non-sliding pieces (knight, king, pawn) and the
//! ray-tracing helpers (`between`, `line`) legal move generation needs for pin and
//! check detection.
//!
//! # Design
//!
//! Every function here is a `const fn` that computes its answer directly from
//! `Bitboard` primitives on each call, rather than indexing into a precomputed
//! static table. For the leaper attacks this is close to free either way:
//! `Bitboard::knight_attacks`, `Bitboard::dilate`, and
//! `Bitboard::pawn_attacks_east`/`pawn_attacks_west` are already O(1) bit tricks
//! (a handful of shifts/masks, no branches, no memory access), so a `[Bitboard;
//! 64]` lookup would trade that for a cache-line load — likely a wash or a
//! pessimization, not a win. `between`/`line` do real work (a bounded ray walk),
//! so a precomputed 64x64 table is a more plausible future win there, but the
//! goal right now is a functional engine, not a fast one — that's a candidate
//! perf pass once something benchmarks-driven (perft, search) can actually show
//! whether it matters, not a guess made in advance.
//!
//! # Public surface
//!
//! - `fn knight_attacks(sq: Square) -> Bitboard`
//! - `fn king_attacks(sq: Square) -> Bitboard` — the 8 neighbors, NOT including
//!   `sq` itself (unlike `Bitboard::dilate`, which includes the seed).
//! - `fn pawn_attacks(color: Color, sq: Square) -> Bitboard` — both capture
//!   squares for a pawn of `color` standing on `sq`.
//! - `fn between(a: Square, b: Square) -> Bitboard` — squares strictly between
//!   `a` and `b` on a shared rank/file/diagonal, or `Bitboard::EMPTY` if they
//!   don't share one (including when `a == b`).
//! - `fn line(a: Square, b: Square) -> Bitboard` — the full rank/file/diagonal
//!   through both `a` and `b`, or `Bitboard::EMPTY` under the same conditions as
//!   `between`.
//!
//! # Implementation notes for `between`/`line`
//!
//! Both share the same first step: classify the relationship between `a` and
//! `b` from their file/rank deltas (`Δfile`, `Δrank`) into one of "same rank",
//! "same file", "same diagonal" (`|Δfile| == |Δrank|`, both nonzero), or
//! "unrelated" — and in the aligned cases, which of the two opposite
//! `Direction`s (e.g. `East` vs `West`) points from `a` toward `b`, from the
//! sign of the delta. That's a positive/negative x four-axis mapping — the same
//! shape that has produced scrambled bugs twice already in `Board::make_move`,
//! so double-check each sign against a concrete example
//! (e.g. `a` above-and-right of `b` should classify as `SouthWest` from `b`'s
//! side / `NorthEast` from `a`'s) rather than trusting it by inspection. Take
//! care that `a == b` (Δfile == Δrank == 0) falls through to "unrelated" rather
//! than satisfying the diagonal check's `abs` equality by accident.
//!
//! Once the direction is known, `Bitboard::occluded_fill` does the actual
//! walking:
//! - `between(a, b)`: fill from `a` in that one direction, treating every
//!   square as passable except `b` (`Bitboard::ALL.without(b)` as the `empty`
//!   argument) so the fill stops exactly at `b`. `occluded_fill` includes both
//!   the seed and the stopping square in its result, so strip `a` and `b`
//!   afterward.
//! - `line(a, b)`: fill from `a` with nothing blocking (`Bitboard::ALL`) in that
//!   direction and its `opposite()`, and union the two — that walks to the
//!   board edge both ways, which is exactly the full line. No need to touch `b`
//!   directly; it's already on the ray by construction.
//!
//! `occluded_fill` loops a fixed 7 steps, which is sufficient: 7 is the maximum
//! possible file/rank/diagonal distance on an 8x8 board, so if `a` and `b` are
//! genuinely aligned the walk is guaranteed to reach `b` within that bound.

use crate::types::bitboard::{Bitboard, Direction};
use crate::types::color::Color;
use crate::types::square::Square;

/// Every square a knight standing on `sq` attacks.
pub const fn knight_attacks(sq: Square) -> Bitboard {
    sq.bitboard().knight_attacks()
}

/// Every square a king standing on `sq` attacks (the 8 neighbors; unlike
/// `Bitboard::dilate`, this does not include `sq` itself).
pub const fn king_attacks(sq: Square) -> Bitboard {
    sq.bitboard().dilate().without(sq)
}

/// Both capture squares for a pawn of `color` standing on `sq`.
pub const fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    let bb = sq.bitboard();
    bb.pawn_attacks_east(color).or(bb.pawn_attacks_west(color))
}

/// The single `Direction` pointing from `a` toward `b` along their shared rank,
/// file, or diagonal, classified from the file/rank deltas. `None` if `a == b`
/// or they share no such line.
const fn ray_direction(a: Square, b: Square) -> Option<Direction> {
    let df = b.file().index() as i8 - a.file().index() as i8;
    let dr = b.rank().index() as i8 - a.rank().index() as i8;
    if df == 0 && dr == 0 {
        None
    } else if dr == 0 {
        Some(if df > 0 {
            Direction::East
        } else {
            Direction::West
        })
    } else if df == 0 {
        Some(if dr > 0 {
            Direction::North
        } else {
            Direction::South
        })
    } else if df == dr {
        Some(if df > 0 {
            Direction::NorthEast
        } else {
            Direction::SouthWest
        })
    } else if df == -dr {
        Some(if df > 0 {
            Direction::SouthEast
        } else {
            Direction::NorthWest
        })
    } else {
        None
    }
}

/// Squares strictly between `a` and `b` on a shared rank, file, or diagonal.
/// `Bitboard::EMPTY` if they don't share one, including when `a == b`.
pub const fn between(a: Square, b: Square) -> Bitboard {
    match ray_direction(a, b) {
        Some(dir) => {
            let empty = Bitboard::ALL.without(b);
            a.bitboard().occluded_fill(empty, dir).without(a).without(b)
        }
        None => Bitboard::EMPTY,
    }
}

/// The full rank, file, or diagonal through both `a` and `b`. `Bitboard::EMPTY`
/// under the same conditions as `between`.
pub const fn line(a: Square, b: Square) -> Bitboard {
    match ray_direction(a, b) {
        Some(dir) => {
            let bb = a.bitboard();
            bb.occluded_fill(Bitboard::ALL, dir)
                .or(bb.occluded_fill(Bitboard::ALL, dir.opposite()))
        }
        None => Bitboard::EMPTY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knight_attacks_center_square_has_all_eight_moves() {
        let expected = Bitboard::EMPTY
            .with(Square::F6)
            .with(Square::G5)
            .with(Square::G3)
            .with(Square::F2)
            .with(Square::D2)
            .with(Square::C3)
            .with(Square::C5)
            .with(Square::D6);
        assert_eq!(knight_attacks(Square::E4), expected);
    }

    #[test]
    fn knight_attacks_corner_square_has_only_two_moves() {
        let expected = Bitboard::EMPTY.with(Square::B3).with(Square::C2);
        assert_eq!(knight_attacks(Square::A1), expected);
    }

    #[test]
    fn king_attacks_corner_square_has_three_moves() {
        let expected = Bitboard::EMPTY
            .with(Square::A2)
            .with(Square::B1)
            .with(Square::B2);
        assert_eq!(king_attacks(Square::A1), expected);
    }

    #[test]
    fn white_pawn_on_e4_attacks_d5_and_f5() {
        let expected = Bitboard::EMPTY.with(Square::D5).with(Square::F5);
        assert_eq!(pawn_attacks(Color::White, Square::E4), expected);
    }

    #[test]
    fn black_pawn_on_e4_attacks_d3_and_f3() {
        let expected = Bitboard::EMPTY.with(Square::D3).with(Square::F3);
        assert_eq!(pawn_attacks(Color::Black, Square::E4), expected);
    }

    #[test]
    fn white_pawn_on_a_file_has_only_one_attack() {
        let expected = Bitboard::EMPTY.with(Square::B5);
        assert_eq!(pawn_attacks(Color::White, Square::A4), expected);
    }

    #[test]
    fn between_on_open_rank_with_rooks_on_e1_and_e8() {
        let expected = Bitboard::EMPTY
            .with(Square::E2)
            .with(Square::E3)
            .with(Square::E4)
            .with(Square::E5)
            .with(Square::E6)
            .with(Square::E7);
        assert_eq!(between(Square::E1, Square::E8), expected);
    }

    #[test]
    fn between_on_main_diagonal() {
        let expected = Bitboard::EMPTY.with(Square::C3).with(Square::D4);
        assert_eq!(between(Square::B2, Square::E5), expected);
    }

    #[test]
    fn between_on_anti_diagonal() {
        let expected = Bitboard::EMPTY.with(Square::D5).with(Square::E4);
        assert_eq!(between(Square::C6, Square::F3), expected);
    }

    #[test]
    fn between_adjacent_squares_is_empty() {
        assert_eq!(between(Square::E4, Square::E5), Bitboard::EMPTY);
    }

    #[test]
    fn between_unaligned_squares_is_empty() {
        // A knight-move apart: no shared rank, file, or diagonal.
        assert_eq!(between(Square::B1, Square::C3), Bitboard::EMPTY);
    }

    #[test]
    fn line_through_e1_e8_is_the_full_e_file() {
        let expected = Bitboard::EMPTY
            .with(Square::E1)
            .with(Square::E2)
            .with(Square::E3)
            .with(Square::E4)
            .with(Square::E5)
            .with(Square::E6)
            .with(Square::E7)
            .with(Square::E8);
        assert_eq!(line(Square::E1, Square::E8), expected);
    }

    #[test]
    fn line_through_a1_h8_is_the_full_main_diagonal() {
        let expected = Bitboard::EMPTY
            .with(Square::A1)
            .with(Square::B2)
            .with(Square::C3)
            .with(Square::D4)
            .with(Square::E5)
            .with(Square::F6)
            .with(Square::G7)
            .with(Square::H8);
        assert_eq!(line(Square::A1, Square::H8), expected);
    }

    #[test]
    fn line_through_a_short_diagonal_does_not_extend_past_the_board() {
        // g1-h2 is only 2 squares long; the line must not "wrap" or invent
        // squares beyond the edge.
        let expected = Bitboard::EMPTY.with(Square::G1).with(Square::H2);
        assert_eq!(line(Square::G1, Square::H2), expected);
    }

    #[test]
    fn line_unaligned_squares_is_empty() {
        assert_eq!(line(Square::B1, Square::C3), Bitboard::EMPTY);
    }
}
