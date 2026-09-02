//! Concrete scenario tests for `eval`'s public API.
//!
//! `tests/eval_props.rs` has the mirror-symmetry/naive-reference coverage;
//! these are hand-picked positions that pin exact numbers down, which the
//! symmetry properties alone can't: a swapped-but-internally-consistent PST
//! table could still pass every property in that file while producing an
//! engine that develops backwards.

mod common;

use common::mirrored;
use turox_engine::board::Board;
use turox_engine::eval::pst::pst_value;
use turox_engine::eval::{eval_white_pov, evaluate};
use turox_engine::{Color, Piece, Square};

// Not just an empirical check: `eval_white_pov_is_mirror_antisymmetric` (in
// `tests/eval_props.rs`) guarantees `eval_white_pov(b) == -eval_white_pov(mirrored(b))`
// for any board, and the start position is its own mirror (White's setup is
// exactly Black's, rank-flipped and color-swapped). So eval_white_pov(start) ==
// -eval_white_pov(mirrored(start)) == -eval_white_pov(start), which forces
// eval_white_pov(start) == 0 regardless of what's in PST: true for any
// self-mirror-symmetric position, not a coincidence about these particular
// table values.
#[test]
fn start_position_is_exactly_zero() {
    let board = Board::start_pos();
    assert_eq!(eval_white_pov(&board), 0);
    assert_eq!(evaluate(&board), 0);
}

// Stays exact rather than approximate with PST folded in: a1 (rook), e1
// (king), and e8 (king, read via flip_rank as e1) all carry a PST value of
// 0 in the tables above, so this position's PST term is 0 - 0 = 0 and the
// score is still pure material.
#[test]
fn white_up_a_rook_scores_exactly_rook_value() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 500);

    let swapped = mirrored(&board);
    assert_eq!(eval_white_pov(&swapped), -500);
}

// Material is unchanged (one White pawn, relocated); the score changes by
// exactly the pawn's own PST delta, d2 (-20) to d4 (+20) = 40, isolating
// the positional term from the material term.
#[test]
fn a_central_pawn_push_changes_pst_but_not_material() {
    let before = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&before), 80); // 100 material + (0 + -20) PST

    let after = Board::try_from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&after), 120); // 100 material + (0 + 20) PST
}

// ---- PST orientation anchors ----
//
// The one thing the symmetric-position tests in `tests/eval_props.rs` can't
// catch on their own: a swapped-but-still-internally-consistent table (or a
// reindexing that reverses the wrong axis) can still pass every property
// there while producing an engine that develops backwards. -20 appears
// exactly once in the pawn table (d2/e2, the "stop blocking your own center
// pawns" penalty) so it's an unambiguous anchor: getting the visual-to-LERF
// reindex or the Black flip_rank backwards lands on a distinctly different
// number, not a coincidentally-equal one.

#[test]
fn white_pawn_second_rank_penalty_lands_on_the_documented_square() {
    assert_eq!(pst_value(Color::White, Piece::Pawn, Square::D2), -20);
}

#[test]
fn black_pawn_reads_the_same_penalty_on_its_own_mirrored_square() {
    assert_eq!(pst_value(Color::Black, Piece::Pawn, Square::D7), -20);
}
