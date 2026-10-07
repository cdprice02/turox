//! Concrete tests for `eval::pst`: orientation anchors that pin single squares
//! to their documented values, and structural tests that pin each table's
//! shape.
//!
//! The anchors catch what mirror symmetry cannot, a table that is swapped or
//! reversed consistently. The shape tests survive a retune, and catch a dropped
//! minus sign on any of the 64 squares, where an anchor only covers the square
//! it names.

use turox_chess::{Color, Piece, Square};
use turox_engine::eval::pst::{pst_value, pst_value_eg};

// -20 appears exactly once in the pawn table (d2/e2, the "stop blocking your
// own center pawns" penalty) so it's an unambiguous anchor: getting the
// visual-to-LERF reindex or the Black flip_rank backwards lands on a distinctly
// different number, not a coincidentally-equal one.
#[test]
fn white_pawn_second_rank_penalty_lands_on_the_documented_square() {
    assert_eq!(pst_value(Color::White, Piece::Pawn, Square::D2), -20);
}

#[test]
fn black_pawn_reads_the_same_penalty_on_its_own_mirrored_square() {
    assert_eq!(pst_value(Color::Black, Piece::Pawn, Square::D7), -20);
}

/// The three file-mirror pairs where the queen's table is genuinely asymmetric,
/// as White sees them. This reproduces a known asymmetry in Michniewski's
/// published table, which this engine copies as-is rather than quietly
/// tidying: a table that disagrees with its source is worse than one with a
/// quirk, because the quirk is at least checkable against the source.
///
/// Listed rather than tolerated, so a *fourth* asymmetry appearing (a real
/// typo) fails instead of blending in with these.
const QUEEN_ASYMMETRIC_SQUARES: [Square; 6] = [
    Square::C2,
    Square::F2,
    Square::B3,
    Square::G3,
    Square::A4,
    Square::H4,
];

/// `QUEEN_ASYMMETRIC_SQUARES` as the given colour sees them. Black's table is
/// White's flipped by rank, so its exceptions are too.
fn queen_exceptions(color: Color) -> Vec<Square> {
    QUEEN_ASYMMETRIC_SQUARES
        .iter()
        .map(|&sq| match color {
            Color::White => sq,
            Color::Black => sq.flip_rank(),
        })
        .collect()
}

/// Chess has no left/right preference, so a piece on b1 is worth what the same
/// piece is worth on g1. Every table here holds that exactly, except the
/// queen's three documented pairs.
///
/// Asserted in both directions: symmetry everywhere it should hold, *and* that
/// the exceptions are exactly the expected squares and no others. Only checking
/// the first would let a new asymmetry hide by being added to the exception
/// list; only checking the second would not notice symmetry breaking elsewhere.
#[test]
fn piece_square_tables_are_file_mirror_symmetric_apart_from_the_queens_quirk() {
    for color in [Color::White, Color::Black] {
        let expected: Vec<Square> = queen_exceptions(color);

        for piece in Piece::ALL {
            let mut asymmetric: Vec<Square> = Vec::new();
            for sq in Square::ALL {
                if pst_value(color, piece, sq) != pst_value(color, piece, sq.flip_file()) {
                    asymmetric.push(sq);
                }
            }
            asymmetric.sort_by_key(|sq: &Square| sq.to_u8());

            let mut want = if piece == Piece::Queen {
                expected.clone()
            } else {
                Vec::new()
            };
            want.sort_by_key(|sq: &Square| sq.to_u8());

            assert_eq!(
                asymmetric, want,
                "{color:?} {piece:?}: file-mirror asymmetry does not match the documented set"
            );
        }
    }
}

/// The three exempted pairs get their values pinned, because exempting a square
/// from the symmetry check exempts it from the only thing checking it.
///
/// Mutation testing found exactly that hole: with symmetry alone, deleting the
/// minus from the queen's `h4` left the square asymmetric, so the property
/// still passed, and it was the one surviving mutant of 105 in this file. These
/// six assertions close it. They are the only per-square values pinned here,
/// and they earn it by being the squares symmetry structurally cannot reach.
#[test]
fn the_queens_asymmetric_squares_have_their_documented_values() {
    for (sq, want) in [
        (Square::A4, 0),
        (Square::H4, -5),
        (Square::B3, 5),
        (Square::G3, 0),
        (Square::C2, 5),
        (Square::F2, 0),
    ] {
        assert_eq!(
            pst_value(Color::White, Piece::Queen, sq),
            want,
            "white queen on {sq:?}: this is one of the three asymmetric pairs, so \
             symmetry cannot check it and this assertion is all there is"
        );
        assert_eq!(
            pst_value(Color::Black, Piece::Queen, sq.flip_rank()),
            want,
            "black queen on {:?} must mirror white's {sq:?}",
            sq.flip_rank()
        );
    }
}

/// The endgame tables carry the same symmetry, and the same single exception.
/// Only the king has a distinct endgame table, so this also covers
/// `pst_value_eg` forwarding correctly for everything else.
#[test]
fn endgame_piece_square_tables_are_file_mirror_symmetric_apart_from_the_queens_quirk() {
    for color in [Color::White, Color::Black] {
        let expected: Vec<Square> = queen_exceptions(color);

        for piece in Piece::ALL {
            let mut asymmetric: Vec<Square> = Vec::new();
            for sq in Square::ALL {
                if pst_value_eg(color, piece, sq) != pst_value_eg(color, piece, sq.flip_file()) {
                    asymmetric.push(sq);
                }
            }
            asymmetric.sort_by_key(|sq: &Square| sq.to_u8());

            let mut want = if piece == Piece::Queen {
                expected.clone()
            } else {
                Vec::new()
            };
            want.sort_by_key(|sq: &Square| sq.to_u8());

            assert_eq!(
                asymmetric, want,
                "{color:?} {piece:?} (endgame): file-mirror asymmetry does not match the documented set"
            );
        }
    }
}

/// A pawn cannot stand on its first or last rank: it starts on the second and
/// promotes on reaching the eighth. Those ranks scoring anything other than
/// zero would be a positional preference for a position that cannot occur,
/// which is how an off-by-one-rank table announces itself.
#[test]
fn pawn_tables_score_zero_on_the_ranks_a_pawn_cannot_occupy() {
    for color in [Color::White, Color::Black] {
        for sq in Square::ALL {
            let rank = sq.rank().index();
            if rank == 0 || rank == 7 {
                assert_eq!(
                    pst_value(color, Piece::Pawn, sq),
                    0,
                    "{color:?} pawn on {sq:?} is unreachable and must score 0"
                );
            }
        }
    }
}

/// Only the king's table differs between midgame and endgame; every other
/// piece forwards. Pinning that keeps `pst_value_eg`'s forwarding branch from
/// silently becoming a second copy of the midgame tables.
#[test]
fn only_the_king_has_a_distinct_endgame_table() {
    for color in [Color::White, Color::Black] {
        for piece in Piece::ALL {
            for sq in Square::ALL {
                let mg = pst_value(color, piece, sq);
                let eg = pst_value_eg(color, piece, sq);
                if piece == Piece::King {
                    continue;
                }
                assert_eq!(
                    mg, eg,
                    "{color:?} {piece:?} on {sq:?} must not differ by phase"
                );
            }
        }
    }

    let differs = Square::ALL.iter().any(|&sq| {
        pst_value(Color::White, Piece::King, sq) != pst_value_eg(Color::White, Piece::King, sq)
    });
    assert!(
        differs,
        "the king's endgame table must actually differ from its midgame one"
    );
}
