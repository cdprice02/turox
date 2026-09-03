//! Attack lookups for the non-sliding pieces (knight, king, pawn) and the
//! ray-tracing helpers (`between`, `line`) legal move generation needs for pin
//! and check detection.
//!
//! Every function here is a `const fn` that computes its answer directly on
//! each call rather than indexing into a precomputed static table: the leaper
//! attacks are already O(1) bit tricks, so a `[Bitboard; 64]` lookup would
//! trade that for a cache-line load, likely a wash, not a win. `between`/
//! `line` do more real work (a bounded ray walk), so a precomputed 64x64
//! table is a more plausible future win there, but that's a candidate perf
//! pass for once something benchmarks-driven can show it matters, not a guess
//! made in advance.
//!
//! The module is named for that possible future, not the present: every
//! function here takes a fixed, small input (`Square`, or a `Square` pair),
//! the same shape `move_gen::magic`'s own committed `[Magic; 64]` tables key
//! off, so swapping any of these to a real lookup table later is a
//! same-signature change, not a redesign. Named ahead of the swap rather than
//! after it, on purpose.

use crate::types::bitboard::{Bitboard, Direction};
use crate::types::color::Color;
use crate::types::square::Square;

/// Every square a knight standing on `sq` attacks.
///
/// Not a `shift`/fill composition: knight moves are a discontinuous jump, not a smear or
/// single step, but they have their own compound shift-with-masking formula, same
/// technique family as `Bitboard::shift`'s diagonals, just wider file-edge masks since a
/// knight can cross two files in one move.
#[must_use]
pub const fn knight_attacks(sq: Square) -> Bitboard {
    let x = sq.bitboard().bits();
    let l1 = (x >> 1) & 0x7F7F_7F7F_7F7F_7F7F;
    let l2 = (x >> 2) & 0x3F3F_3F3F_3F3F_3F3F;
    let r1 = (x << 1) & 0xFEFE_FEFE_FEFE_FEFE;
    let r2 = (x << 2) & 0xFCFC_FCFC_FCFC_FCFC;
    let h1 = l1 | r1;
    let h2 = l2 | r2;
    Bitboard::from_bits((h1 << 16) | (h1 >> 16) | (h2 << 8) | (h2 >> 8))
}

/// Every square a king standing on `sq` attacks (the 8 neighbors; unlike
/// `Bitboard::dilate`, this does not include `sq` itself).
#[must_use]
pub const fn king_attacks(sq: Square) -> Bitboard {
    sq.bitboard().dilate().without(sq)
}

/// Pawns of `color` on `bb`, attacking east (white: NE; black: SE).
const fn pawn_attacks_east(bb: Bitboard, color: Color) -> Bitboard {
    match color {
        Color::White => bb.shift(Direction::NorthEast),
        Color::Black => bb.shift(Direction::SouthEast),
    }
}

/// Pawns of `color` on `bb`, attacking west (white: NW; black: SW).
const fn pawn_attacks_west(bb: Bitboard, color: Color) -> Bitboard {
    match color {
        Color::White => bb.shift(Direction::NorthWest),
        Color::Black => bb.shift(Direction::SouthWest),
    }
}

/// Both capture squares for a pawn of `color` standing on `sq`.
#[must_use]
pub const fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    let bb = sq.bitboard();
    pawn_attacks_east(bb, color).or(pawn_attacks_west(bb, color))
}

/// The single `Direction` pointing from `a` toward `b` along their shared rank,
/// file, or diagonal, classified from the file/rank deltas into "same rank",
/// "same file", "same diagonal" (`|Δfile| == |Δrank|`, both nonzero), or
/// "unrelated". `None` if `a == b` (which falls through to "unrelated" rather
/// than satisfying the diagonal check's `abs` equality by accident) or they
/// share no such line.
const fn ray_direction(a: Square, b: Square) -> Option<Direction> {
    let df = b.file().to_u8().cast_signed() - a.file().to_u8().cast_signed();
    let dr = b.rank().to_u8().cast_signed() - a.rank().to_u8().cast_signed();
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
///
/// `Bitboard::EMPTY` if they don't share one, including when `a == b`. Fills
/// from `a` toward `b` via `occluded_fill`, treating every square but `b` as
/// passable so the fill stops exactly at `b`; both endpoints get stripped
/// afterward, since `occluded_fill` includes its seed and stopping square.
#[must_use]
pub const fn between(a: Square, b: Square) -> Bitboard {
    match ray_direction(a, b) {
        Some(dir) => {
            let empty = Bitboard::ALL.without(b);
            a.bitboard().occluded_fill(empty, dir).without(a).without(b)
        }
        None => Bitboard::EMPTY,
    }
}

/// The full rank, file, or diagonal through both `a` and `b`.
///
/// `Bitboard::EMPTY` under the same conditions as `between`. Fills from `a` with nothing
/// blocking, in both the direction toward `b` and its `opposite()`, walking to the board
/// edge both ways; no need to touch `b` directly, since it's already on the ray by
/// construction.
#[must_use]
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

    // ---- Exhaustive checks against an independent reference ----
    //
    // The concrete examples above are readable, pinned anchors; these check
    // every function against a definition built directly from
    // `Square::offset`/file-rank arithmetic, not from whatever `Bitboard`
    // primitive (`dilate`, `occluded_fill`, ...) the real implementation
    // uses, over every one of `Square`'s 64 values (or 64*64 pairs, for the
    // two-square functions).
    use crate::types::square::{File, Rank};

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
        let (af, ar) = (i32::from(a.file().to_u8()), i32::from(a.rank().to_u8()));
        let (bf, br) = (i32::from(b.file().to_u8()), i32::from(b.rank().to_u8()));
        let (df, dr) = (bf - af, br - ar);
        if (df == 0 && dr == 0) || (df != 0 && dr != 0 && df.abs() != dr.abs()) {
            return Bitboard::EMPTY;
        }
        let (step_f, step_r) = (df.signum(), dr.signum());
        let mut result = Bitboard::EMPTY;
        let (mut f, mut r) = (af + step_f, ar + step_r);
        while (f, r) != (bf, br) {
            let sq = Square::new(
                File::from_u8(u8::try_from(f).expect("on the shared line, so in bounds"))
                    .expect("on the shared line, so in bounds"),
                Rank::from_u8(u8::try_from(r).expect("on the shared line, so in bounds"))
                    .expect("on the shared line, so in bounds"),
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
        let (af, ar) = (
            a.file().to_u8().cast_signed(),
            a.rank().to_u8().cast_signed(),
        );
        let (bf, br) = (
            b.file().to_u8().cast_signed(),
            b.rank().to_u8().cast_signed(),
        );
        let (df, dr) = (bf - af, br - ar);
        if (df == 0 && dr == 0) || (df != 0 && dr != 0 && df.abs() != dr.abs()) {
            return Bitboard::EMPTY;
        }
        let mut result = Bitboard::EMPTY;
        for sq in Square::ALL {
            let (sf, sr) = (
                sq.file().to_u8().cast_signed(),
                sq.rank().to_u8().cast_signed(),
            );
            let cross = (sf - af) * dr - (sr - ar) * df;
            if cross == 0 {
                result = result.with(sq);
            }
        }
        result
    }

    #[test]
    fn knight_attacks_matches_naive_deltas() {
        for sq in Square::ALL {
            assert_eq!(knight_attacks(sq), naive_knight_attacks(sq), "sq={sq:?}");
        }
    }

    #[test]
    fn king_attacks_matches_naive_deltas() {
        for sq in Square::ALL {
            assert_eq!(king_attacks(sq), naive_king_attacks(sq), "sq={sq:?}");
        }
    }

    #[test]
    fn king_attacks_never_contains_its_own_square() {
        for sq in Square::ALL {
            assert!(!king_attacks(sq).contains(sq), "sq={sq:?}");
        }
    }

    #[test]
    fn king_attacks_matches_dilate_minus_self() {
        for sq in Square::ALL {
            assert_eq!(
                king_attacks(sq),
                sq.bitboard().dilate().without(sq),
                "sq={sq:?}"
            );
        }
    }

    #[test]
    fn pawn_attacks_matches_naive_deltas() {
        for sq in Square::ALL {
            for color in Color::ALL {
                assert_eq!(
                    pawn_attacks(color, sq),
                    naive_pawn_attacks(color, sq),
                    "sq={sq:?} color={color:?}"
                );
            }
        }
    }

    #[test]
    fn pawn_attacks_never_contains_its_own_square() {
        for sq in Square::ALL {
            for color in Color::ALL {
                assert!(
                    !pawn_attacks(color, sq).contains(sq),
                    "sq={sq:?} color={color:?}"
                );
            }
        }
    }

    #[test]
    fn pawn_attacks_has_at_most_two_squares() {
        for sq in Square::ALL {
            for color in Color::ALL {
                assert!(
                    pawn_attacks(color, sq).count() <= 2,
                    "sq={sq:?} color={color:?}"
                );
            }
        }
    }

    #[test]
    fn between_matches_naive_walk() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(between(a, b), naive_between(a, b), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn between_is_symmetric() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(between(a, b), between(b, a), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn between_never_contains_its_endpoints() {
        for a in Square::ALL {
            for b in Square::ALL {
                let bb = between(a, b);
                assert!(!bb.contains(a), "a={a:?} b={b:?}");
                assert!(!bb.contains(b), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn between_own_square_is_empty() {
        for a in Square::ALL {
            assert_eq!(between(a, a), Bitboard::EMPTY, "a={a:?}");
        }
    }

    #[test]
    fn between_is_a_subset_of_line() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(
                    between(a, b).and_not(line(a, b)),
                    Bitboard::EMPTY,
                    "a={a:?} b={b:?}"
                );
            }
        }
    }

    #[test]
    fn line_matches_naive_collinearity_scan() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(line(a, b), naive_line(a, b), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn line_is_symmetric() {
        for a in Square::ALL {
            for b in Square::ALL {
                assert_eq!(line(a, b), line(b, a), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn line_own_square_is_empty() {
        for a in Square::ALL {
            assert_eq!(line(a, a), Bitboard::EMPTY, "a={a:?}");
        }
    }

    #[test]
    fn line_contains_both_endpoints_when_aligned() {
        for a in Square::ALL {
            for b in Square::ALL {
                let bb = line(a, b);
                if !bb.is_empty() {
                    assert!(bb.contains(a), "a={a:?} b={b:?}");
                    assert!(bb.contains(b), "a={a:?} b={b:?}");
                }
            }
        }
    }
}
