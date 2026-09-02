//! Property tests for `eval`: the executable version of the mirror-symmetry
//! and sign-convention contracts `eval_white_pov`/`evaluate` are documented
//! against, extended here to cover piece-square tables (`eval::pst`).
//!
//! There's no perft equivalent here: eval has no published ground truth, so
//! correctness means self-consistency, symmetry, and agreement with an
//! independent naive reference, same discipline as `tests/attacks_props.rs`.
//! `naive_eval_white_pov` below walks the mailbox directly and shares no
//! material-summing code with the real bitboard-based implementation, but it
//! *does* call the real `pst::pst_value` rather than hand-duplicating the
//! 384-entry table: unlike the six `PIECE_VALUES`, independently
//! transcribing a table that size invites a copy-paste error that would fail
//! this test for a reason that has nothing to do with `eval_white_pov`'s own
//! correctness. Independence for PST is enforced instead by the orientation
//! anchors in `tests/eval.rs`, which pin specific squares to specific values
//! without depending on the shape of the whole table; that file also has the
//! rest of this module's concrete scenario tests. This one is proptest only.

mod common;

use common::{any_board, mirrored};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::pst::pst_value;
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
            let material = NAIVE_PIECE_VALUES[cp.piece().index()];
            let positional = pst_value(cp.color(), cp.piece(), sq);
            score += match cp.color() {
                Color::White => material + positional,
                Color::Black => -(material + positional),
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
    // lookup on every generated position, not just a hand-written case. With
    // PST folded in, this covers a color-flip bug in `pst_value` just as
    // much as one in the material sum.
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

    // Holds given these specific table magnitudes (PST entries stay within
    // roughly +/-50, well under any piece's material value), not as a
    // structural guarantee independent of the data the way the material-only
    // version of this property was. Worth re-checking if the tables are
    // ever retuned with more extreme values.
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
}
