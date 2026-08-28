//! Property tests for `eval`: the executable version of the mirror-symmetry
//! and sign-convention contracts `eval_white_pov`/`evaluate` are documented
//! against.
//!
//! There's no perft equivalent here: eval has no published ground truth, so
//! correctness means self-consistency, symmetry, and agreement with an
//! independent naive reference, same discipline as `tests/attacks_props.rs`.
//! `naive_eval_white_pov` below walks the mailbox directly and shares no
//! code with the real bitboard-based implementation.
//!
//! Two tests are deliberately temporary and are called out where they sit:
//! `material_only_score_is_invariant_to_piece_position` holds only while
//! material is the sole eval term, and the `500`/`100` exact-value scenarios
//! stop being exact once piece-square tables (#22) add positional bonuses.

mod common;

use common::{any_board, mirrored};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::{eval_white_pov, evaluate, Score};
use turox_engine::{Color, Piece, Square};

// ---- Independent reference ----

/// Hardcoded independently of `eval::PIECE_VALUES` (which is private
/// anyway): a mailbox walk summing the same spec values `eval_white_pov`
/// documents itself against, not a call into the module under test.
const NAIVE_PIECE_VALUES: [Score; 6] = [100, 320, 330, 500, 900, 0];

fn naive_eval_white_pov(board: &Board) -> Score {
    let mut score: Score = 0;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            let value = NAIVE_PIECE_VALUES[cp.piece() as usize];
            score += match cp.color() {
                Color::White => value,
                Color::Black => -value,
            };
        }
    }
    score
}

proptest! {
    #[test]
    fn eval_white_pov_matches_naive_reference(board in any_board()) {
        prop_assert_eq!(eval_white_pov(&board), naive_eval_white_pov(&board));
    }

    #[test]
    fn evaluate_matches_sign_convention(board in any_board()) {
        let expected = match board.side_to_move() {
            Color::White => eval_white_pov(&board),
            Color::Black => -eval_white_pov(&board),
        };
        prop_assert_eq!(evaluate(&board), expected);
    }

    // The highest-value test in this file: catches a scrambled White/Black
    // lookup on every generated position, not just a hand-written case.
    #[test]
    fn eval_white_pov_is_mirror_antisymmetric(board in any_board()) {
        prop_assert_eq!(eval_white_pov(&board), -eval_white_pov(&mirrored(&board)));
    }

    #[test]
    fn mirrored_round_trips(board in any_board()) {
        prop_assert_eq!(mirrored(&mirrored(&board)), board);
    }

    #[test]
    fn eval_is_invariant_to_move_clocks(
        board in any_board(),
        halfmove_clock in 0u8..=200,
        fullmove_number in 1u16..=500,
    ) {
        let varied = Board::from_parts(
            board,
            board.side_to_move(),
            board.castling_rights(),
            board.en_passant(),
            halfmove_clock,
            fullmove_number,
        );
        prop_assert_eq!(eval_white_pov(&board), eval_white_pov(&varied));
    }

    #[test]
    fn removing_a_black_piece_strictly_increases_white_pov(board in any_board()) {
        let target = Square::ALL.into_iter().find(|&sq| {
            matches!(board.piece_at(sq), Some(cp) if cp.color() == Color::Black && cp.piece() != Piece::King)
        });
        let Some(sq) = target else {
            // any_board() sometimes places nothing beyond the two kings.
            return Ok(());
        };
        let mut reduced = board;
        reduced.remove(sq);
        prop_assert!(eval_white_pov(&reduced) > eval_white_pov(&board));
    }

    // Deliberately temporary: piece-square tables (#22) make this false by
    // design. Delete this test in that PR rather than weakening it.
    #[test]
    fn material_only_score_is_invariant_to_piece_position(board in any_board()) {
        let occupied = board.occupied();
        let from = Square::ALL.into_iter().find(|&sq| {
            matches!(board.piece_at(sq), Some(cp) if cp.piece() != Piece::King)
        });
        let Some(from) = from else {
            return Ok(());
        };
        let Some(to) = Square::ALL.into_iter().find(|&sq| !occupied.contains(sq)) else {
            return Ok(());
        };
        let cp = board.piece_at(from).expect("checked above");
        let mut moved = board;
        moved.remove(from);
        moved.place(to, cp);
        prop_assert_eq!(eval_white_pov(&board), eval_white_pov(&moved));
    }
}

// ---- Concrete scenarios ----

#[test]
fn start_position_is_exactly_zero() {
    let board = Board::start_pos();
    assert_eq!(eval_white_pov(&board), 0);
    assert_eq!(evaluate(&board), 0);
}

// Exact today because material is the only term; becomes approximate once
// piece-square tables (#22) add positional bonuses on top.
#[test]
fn white_up_a_rook_scores_exactly_rook_value() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 500);

    let swapped = mirrored(&board);
    assert_eq!(eval_white_pov(&swapped), -500);
}

// After 1.e4 d5 2.exd5: White is up exactly one pawn's worth of material.
// No move_gen dependency needed; both FENs are hand-authored positions.
#[test]
fn a_pawn_capture_changes_material_by_its_value() {
    let before =
        Board::try_from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2")
            .expect("valid FEN");
    assert_eq!(eval_white_pov(&before), 0);

    let after = Board::try_from_fen("rnbqkbnr/ppp1pppp/8/3P4/8/8/PPPP1PPP/RNBQKBNR b KQkq - 0 2")
        .expect("valid FEN");
    assert_eq!(eval_white_pov(&after), 100);
}
