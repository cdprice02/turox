//! King-safety evaluation: the pawn-only half (#57). Shelter, storm, and
//! open/semi-open files near the king, each its own scoring function in the
//! same shape `pawn_structure` uses for doubled/isolated/passed pawns.
//!
//! Every constant in this module packs a zero `eg`: #83 found the endgame
//! king PST already pays +40 for a centralized king, and a king-safety term
//! active in the endgame lane too would fight that exactly where it
//! matters most, a king still under attack in a position that reads as
//! partway toward the endgame on `game_phase`'s blend.

use crate::board::Board;
use crate::eval::phase::{pack, Tapered};
use crate::move_gen::attacks::king_square;
use crate::types::{Bitboard, Color, Piece, Square};
use crate::Direction;

/// Penalty per zone file (`sq`'s own file and each file adjacent to it,
/// clamped at the board edge) with no friendly pawn anywhere on it: nothing
/// left on that file to block a rook or queen running straight at the king.
const SHELTER_PENALTY: Tapered = pack(-15, 0);

/// Additional penalty per zone file with no pawn of *either* color on it: a
/// fully open file is worse than merely missing friendly cover
/// (`SHELTER_PENALTY` already charges for that part on its own), since it
/// also has no enemy pawn left to block a rook or queen from running all
/// the way down it. Stacks on top of `SHELTER_PENALTY` rather than
/// replacing it: a fully open file is strictly more dangerous than a
/// semi-open one, not a different category of danger.
const OPEN_FILE_PENALTY: Tapered = pack(-25, 0);

/// `king_sq`'s own file plus one file adjacent to it on either side, each
/// the full 8-square file. Always 3 entries, `const fn`-friendly (a fixed
/// size a `while` loop can index): a corner king's missing third file
/// comes back as `Bitboard::EMPTY` from `shift`'s own wrap-safety (an
/// a-file king's `shift(West)` lands off the board rather than wrapping to
/// the h-file) rather than being dropped, since there's no allocation-free
/// way for a `const fn` to return a variably-sized collection.
///
/// Every caller must check `is_empty()` on each entry before testing its
/// own predicate against it: `Bitboard::EMPTY.and(pawns)` is trivially
/// empty regardless of `pawns`, which reads as "a real, pawnless file"
/// instead of "no such file" if left unguarded, the same corner-king bug
/// `shelter_penalty` hit once already.
const fn zone_files(king_sq: Square) -> [Bitboard; 3] {
    let king_file = king_sq.file().bitboard();
    [
        king_file.shift(Direction::West),
        king_file,
        king_file.shift(Direction::East),
    ]
}

/// `SHELTER_PENALTY` once for every one of `king_sq`'s zone files with no
/// bit of `pawns` on it, checked by presence anywhere on the file rather
/// than distance from the king: a pushed-but-not-traded shield pawn still
/// counts as cover for this first pass, sizing refinements are a tuning
/// question for later.
const fn shelter_penalty(pawns: Bitboard, king_sq: Square) -> Tapered {
    let files = zone_files(king_sq);
    let mut missing: u32 = 0;
    let mut i = 0;
    while i < files.len() {
        let f = files[i];
        if !f.is_empty() && f.and(pawns).is_empty() {
            missing += 1;
        }
        i += 1;
    }
    missing.cast_signed() * SHELTER_PENALTY
}

/// `OPEN_FILE_PENALTY` once for every one of `king_sq`'s zone files with no
/// bit of `pawns` *and* no bit of `enemy_pawns` on it: a file a pawn of
/// either color still occupies isn't open, even if that pawn belongs to
/// the attacker rather than the defender (that half-open case is already
/// priced by `shelter_penalty`, on its own).
const fn open_file_penalty(pawns: Bitboard, enemy_pawns: Bitboard, king_sq: Square) -> Tapered {
    let occupied = pawns.or(enemy_pawns);
    let files = zone_files(king_sq);
    let mut open: u32 = 0;
    let mut i = 0;
    while i < files.len() {
        let f = files[i];
        if !f.is_empty() && f.and(occupied).is_empty() {
            open += 1;
        }
        i += 1;
    }
    open.cast_signed() * OPEN_FILE_PENALTY
}

/// Penalty per enemy pawn found in `king_sq`'s storm zone (see
/// `storm_zone`): an advancing pawn threatening to crack the shelter open,
/// distinct from `SHELTER_PENALTY`/`OPEN_FILE_PENALTY`, which only look at
/// whether pawns are *missing*, not whether the enemy's are closing in.
const STORM_PENALTY: Tapered = pack(-10, 0);

/// How many ranks deep `storm_zone` reaches in front of the king: a pawn
/// still this close to its own back rank hasn't threatened anything yet,
/// every real game's pawns start there. First-pass placeholder, same as
/// every other magnitude in this module, tunable once #39's SPRT harness
/// can measure it against real games rather than reasoning about it.
const STORM_RANGE: u8 = 3;

/// The three-file, `STORM_RANGE`-rank cone strictly ahead of `king_sq`,
/// from `color`'s own forward direction: the region an enemy pawn has to
/// reach before it counts as storming rather than just existing somewhere
/// on the board.
///
/// Built the same way `pawn_structure::isolani`/`zone_files` stay
/// edge-safe: `shift(Direction::East)`/`shift(Direction::West)` widen the
/// king's own square to three files *before* pushing forward, and every
/// step forward is a single wrap-safe `shift`, not a fill, so the cone
/// stops after `STORM_RANGE` ranks instead of running to the board edge
/// the way `front_attack_span` does.
///
/// Takes `color` explicitly rather than reading it off a `Board`: this is
/// the one function in the module where getting White and Black's forward
/// direction backwards would silently look correct (empty zone either way
/// on a fresh board), the exact `{Color}x{direction}` shape this repo's
/// history says to test explicitly rather than trust by inspection.
const fn storm_zone(king_sq: Square, color: Color) -> Bitboard {
    let forward = color.forward();
    let mut zone = king_sq.bitboard().shift(forward);
    zone = zone
        .or(zone.shift(Direction::West))
        .or(zone.shift(Direction::East));
    let mut i = 1;
    while i < STORM_RANGE {
        zone = zone.or(zone.shift(forward));
        i += 1;
    }
    zone
}

/// `STORM_PENALTY` once for every bit of `enemy_pawns` inside
/// `storm_zone(king_sq, color)`.
const fn storm_penalty(enemy_pawns: Bitboard, king_sq: Square, color: Color) -> Tapered {
    storm_zone(king_sq, color)
        .and(enemy_pawns)
        .count()
        .cast_signed()
        * STORM_PENALTY
}

/// `color`'s total king-safety contribution: shelter, open-file, and storm
/// penalties summed over `color`'s actual king. `0` if `color` has no king
/// (unreachable in a real game, but `any_board()`-style proptest input can
/// place none): there's no king to be unsafe, and every term above needs a
/// real `king_sq` to measure from.
#[must_use]
pub const fn king_safety_score(board: &Board, color: Color) -> Tapered {
    let Some(king_sq) = king_square(board, color) else {
        return 0;
    };
    let pawns = board.pieces(color, Piece::Pawn);
    let enemy_pawns = board.pieces(color.flip(), Piece::Pawn);
    shelter_penalty(pawns, king_sq)
        + open_file_penalty(pawns, enemy_pawns, king_sq)
        + storm_penalty(enemy_pawns, king_sq, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- zone_files ----

    #[test]
    fn zone_files_has_no_empty_entries_for_a_non_edge_king() {
        assert!(zone_files(Square::G1).iter().all(|f| !f.is_empty()));
    }

    #[test]
    fn zone_files_has_exactly_one_empty_entry_for_an_a_file_king() {
        assert_eq!(
            zone_files(Square::A1)
                .iter()
                .filter(|f| f.is_empty())
                .count(),
            1
        );
    }

    #[test]
    fn zone_files_has_exactly_one_empty_entry_for_an_h_file_king() {
        assert_eq!(
            zone_files(Square::H1)
                .iter()
                .filter(|f| f.is_empty())
                .count(),
            1
        );
    }

    #[test]
    fn shelter_penalty_is_zero_with_a_pawn_on_every_zone_file() {
        let king_sq = Square::G1;
        let pawns = Bitboard::EMPTY
            .with(Square::F2)
            .with(Square::G2)
            .with(Square::H2);
        assert_eq!(shelter_penalty(pawns, king_sq), 0);
    }

    #[test]
    fn shelter_penalty_counts_one_file_with_no_pawn_anywhere_on_it() {
        let king_sq = Square::G1;
        // g-pawn captured or traded away entirely; f and h still covered.
        let pawns = Bitboard::EMPTY.with(Square::F2).with(Square::H2);
        assert_eq!(shelter_penalty(pawns, king_sq), SHELTER_PENALTY);
    }

    #[test]
    fn shelter_penalty_does_not_care_where_on_the_file_the_pawn_sits() {
        // g-pawn pushed to g4: still on the g-file, so this file isn't
        // "missing" even though the pawn is no longer close to the king.
        let king_sq = Square::G1;
        let pawns = Bitboard::EMPTY
            .with(Square::F2)
            .with(Square::G4)
            .with(Square::H2);
        assert_eq!(shelter_penalty(pawns, king_sq), 0);
    }

    #[test]
    fn shelter_penalty_counts_all_three_zone_files_on_a_bare_board() {
        let king_sq = Square::G1;
        assert_eq!(
            shelter_penalty(Bitboard::EMPTY, king_sq),
            SHELTER_PENALTY + SHELTER_PENALTY + SHELTER_PENALTY
        );
    }

    // The two edge cases this repo's {Color}x{direction} bug history says
    // to write explicitly: a corner king has only two zone files, and a
    // wraparound bug would silently manufacture a third by reading the far
    // side of the board as "adjacent".
    #[test]
    fn shelter_penalty_does_not_wrap_past_the_a_file_for_a_corner_king() {
        let king_sq = Square::A1;
        assert_eq!(
            shelter_penalty(Bitboard::EMPTY, king_sq),
            SHELTER_PENALTY + SHELTER_PENALTY
        );
    }

    #[test]
    fn shelter_penalty_does_not_wrap_past_the_h_file_for_a_corner_king() {
        let king_sq = Square::H1;
        assert_eq!(
            shelter_penalty(Bitboard::EMPTY, king_sq),
            SHELTER_PENALTY + SHELTER_PENALTY
        );
    }

    // ---- open_file_penalty ----

    #[test]
    fn open_file_penalty_is_zero_when_every_zone_file_has_a_pawn_of_either_color() {
        let king_sq = Square::G1;
        // f and h covered by White, g covered only by Black: no file is
        // fully empty, so nothing here is open regardless of whose pawn
        // it is.
        let pawns = Bitboard::EMPTY.with(Square::F2).with(Square::H2);
        let enemy_pawns = Bitboard::EMPTY.with(Square::G6);
        assert_eq!(open_file_penalty(pawns, enemy_pawns, king_sq), 0);
    }

    #[test]
    fn open_file_penalty_counts_one_file_with_no_pawn_of_either_color() {
        let king_sq = Square::G1;
        // g-file: no White pawn, no Black pawn either, fully open.
        let pawns = Bitboard::EMPTY.with(Square::F2).with(Square::H2);
        let enemy_pawns = Bitboard::EMPTY.with(Square::A7);
        assert_eq!(
            open_file_penalty(pawns, enemy_pawns, king_sq),
            OPEN_FILE_PENALTY
        );
    }

    #[test]
    fn open_file_penalty_does_not_count_a_semi_open_file_held_only_by_the_enemy() {
        // g-file has a Black pawn and no White pawn: half-open, not open.
        // `shelter_penalty` prices the missing friendly cover on its own;
        // this term must not charge for it again.
        let king_sq = Square::G1;
        let pawns = Bitboard::EMPTY.with(Square::F2).with(Square::H2);
        let enemy_pawns = Bitboard::EMPTY.with(Square::G7);
        assert_eq!(open_file_penalty(pawns, enemy_pawns, king_sq), 0);
    }

    #[test]
    fn open_file_penalty_counts_all_three_zone_files_on_a_bare_board() {
        let king_sq = Square::G1;
        assert_eq!(
            open_file_penalty(Bitboard::EMPTY, Bitboard::EMPTY, king_sq),
            OPEN_FILE_PENALTY + OPEN_FILE_PENALTY + OPEN_FILE_PENALTY
        );
    }

    #[test]
    fn open_file_penalty_does_not_wrap_past_the_a_file_for_a_corner_king() {
        let king_sq = Square::A1;
        assert_eq!(
            open_file_penalty(Bitboard::EMPTY, Bitboard::EMPTY, king_sq),
            OPEN_FILE_PENALTY + OPEN_FILE_PENALTY
        );
    }

    #[test]
    fn open_file_penalty_does_not_wrap_past_the_h_file_for_a_corner_king() {
        let king_sq = Square::H1;
        assert_eq!(
            open_file_penalty(Bitboard::EMPTY, Bitboard::EMPTY, king_sq),
            OPEN_FILE_PENALTY + OPEN_FILE_PENALTY
        );
    }

    // ---- storm_zone ----

    #[test]
    fn storm_zone_is_nine_squares_for_a_non_edge_king() {
        // 3 files (f, g, h) x STORM_RANGE (3) ranks ahead.
        assert_eq!(storm_zone(Square::G1, Color::White).count(), 9);
    }

    #[test]
    fn storm_zone_is_six_squares_for_a_corner_king() {
        // 2 files (a, b) x STORM_RANGE (3) ranks ahead.
        assert_eq!(storm_zone(Square::A1, Color::White).count(), 6);
    }

    #[test]
    fn storm_zone_excludes_the_kings_own_square() {
        let zone = storm_zone(Square::G1, Color::White);
        assert!(zone.and(Square::G1.bitboard()).is_empty());
    }

    // The direction check this repo's history says to write explicitly:
    // Black's cone reaches down the board, not up it, so it must not
    // overlap the ranks White's cone from the mirrored square would cover.
    #[test]
    fn storm_zone_reaches_downward_for_black() {
        let zone = storm_zone(Square::G8, Color::Black);
        assert!(!zone.and(Square::G7.bitboard()).is_empty());
        assert!(zone.and(Square::G4.bitboard()).is_empty());
    }

    // ---- storm_penalty ----

    #[test]
    fn storm_penalty_is_zero_for_a_pawn_still_on_its_home_rank() {
        let king_sq = Square::G1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::G7);
        assert_eq!(storm_penalty(enemy_pawns, king_sq, Color::White), 0);
    }

    #[test]
    fn storm_penalty_counts_a_pawn_at_the_edge_of_the_cone() {
        // g4 is exactly STORM_RANGE (3) ranks ahead of g1.
        let king_sq = Square::G1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::G4);
        assert_eq!(
            storm_penalty(enemy_pawns, king_sq, Color::White),
            STORM_PENALTY
        );
    }

    #[test]
    fn storm_penalty_excludes_a_pawn_one_rank_beyond_the_cone() {
        let king_sq = Square::G1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::G5);
        assert_eq!(storm_penalty(enemy_pawns, king_sq, Color::White), 0);
    }

    #[test]
    fn storm_penalty_counts_multiple_pawns_in_the_cone() {
        let king_sq = Square::G1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::F3).with(Square::G3);
        assert_eq!(
            storm_penalty(enemy_pawns, king_sq, Color::White),
            STORM_PENALTY + STORM_PENALTY
        );
    }

    #[test]
    fn storm_penalty_ignores_a_pawn_outside_the_zone_files() {
        // e-file is two files from g, outside the f/g/h zone.
        let king_sq = Square::G1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::E2);
        assert_eq!(storm_penalty(enemy_pawns, king_sq, Color::White), 0);
    }

    #[test]
    fn storm_penalty_counts_an_advancing_pawn_for_black_too() {
        // g5 is exactly STORM_RANGE ranks below g8, White advancing on a
        // Black king; must not be hardcoded to White's own direction.
        let king_sq = Square::G8;
        let enemy_pawns = Bitboard::EMPTY.with(Square::G5);
        assert_eq!(
            storm_penalty(enemy_pawns, king_sq, Color::Black),
            STORM_PENALTY
        );
    }

    #[test]
    fn storm_penalty_does_not_wrap_past_the_a_file_for_a_corner_king() {
        // If the west shift wrapped instead of falling off the board, a
        // corner king's cone would spuriously pick up the h-file.
        let king_sq = Square::A1;
        let enemy_pawns = Bitboard::EMPTY.with(Square::H3);
        assert_eq!(storm_penalty(enemy_pawns, king_sq, Color::White), 0);
    }
}
