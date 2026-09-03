//! Property tests for `eval`: the executable version of the mirror-symmetry
//! and sign-convention contracts `eval_white_pov`/`evaluate` are documented
//! against, extended here to cover piece-square tables (`eval::pst`) and
//! the midgame/endgame tapering blend (`eval::phase`).
//!
//! There's no perft equivalent here: eval has no published ground truth, so
//! correctness means self-consistency, symmetry, and agreement with an
//! independent naive reference, same discipline as `tests/attacks_props.rs`.
//! `naive_eval_white_pov` below walks the mailbox directly and shares no
//! material-summing code with the real bitboard-based implementation, but it
//! *does* call the real `pst::pst_value`/`pst::pst_value_eg` rather than
//! hand-duplicating the 384-entry tables: unlike the six `PIECE_VALUES`,
//! independently transcribing tables that size invites a copy-paste error
//! that would fail this test for a reason that has nothing to do with
//! `eval_white_pov`'s own correctness. Independence for PST is enforced
//! instead by the orientation anchors in `tests/eval.rs`, which pin specific
//! squares to specific values without depending on the shape of the whole
//! table; that file also has the rest of this module's concrete scenario
//! tests. This one is proptest only.
//!
//! The phase computation here (`naive_game_phase`) is its own mailbox walk
//! too, deliberately not a call into `eval::phase::game_phase`: `phase` is a
//! private module, unreachable from here (this is a separate integration-test
//! crate) and, more importantly, the point of an independent reference is
//! that it doesn't share code with the logic under test. Both
//! `naive_game_phase` and the blend at the bottom of `naive_eval_white_pov`
//! reproduce the same formula `eval::phase` documents itself against, not a
//! call into it.

mod common;

use common::{any_board, mirrored};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::pst::{pst_value, pst_value_eg};
use turox_engine::eval::{eval_white_pov, evaluate, Score};
use turox_engine::{Color, Piece, Square};

// ---- Independent reference ----

/// Hardcoded independently of `eval::PIECE_VALUES` (which is private
/// anyway): a mailbox walk summing the same spec values `eval_white_pov`
/// documents itself against, not a call into the module under test.
const NAIVE_PIECE_VALUES: [Score; 6] = [100, 320, 330, 500, 900, 0];

/// Independent of `eval::phase::PHASE_WEIGHT`/`TOTAL_PHASE`: the same
/// standard tapered-eval weighting, transcribed again here rather than
/// shared, for the same reason `NAIVE_PIECE_VALUES` isn't shared either.
const NAIVE_PHASE_WEIGHT: [i32; 6] = [0, 1, 1, 2, 4, 0];
const NAIVE_TOTAL_PHASE: i32 = 24;

/// Mailbox walk over `board.piece_at`, reproducing `eval::phase::game_phase`
/// without calling it: sums `NAIVE_PHASE_WEIGHT` for every non-pawn,
/// non-king piece found, subtracts from `NAIVE_TOTAL_PHASE`, clamps to
/// non-negative (`any_board()` can place more non-pawn material than
/// `NAIVE_TOTAL_PHASE` accounts for), and scales to `0..=256`.
fn naive_game_phase(board: &Board) -> i32 {
    let mut phase = NAIVE_TOTAL_PHASE;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if !matches!(cp.piece(), Piece::Pawn | Piece::King) {
                phase -= NAIVE_PHASE_WEIGHT[cp.piece().index()];
            }
        }
    }
    let phase = phase.max(0);
    (phase * 256 + NAIVE_TOTAL_PHASE / 2) / NAIVE_TOTAL_PHASE
}

/// Sums a midgame and an endgame total independently (no packed
/// representation, unlike `eval::phase::Tapered`) and blends them with the
/// standard tapered-eval formula, `(mg * (256 - phase) + eg * phase) / 256`:
/// the widely reproduced technique this whole module is built on, described
/// on the chess programming wiki's "Tapered Eval" page. Widened to `i32`
/// for the multiply, since `mg`/`eg` scaled by up to 256 would overflow
/// `Score` (`i16`) well before the division brings the result back down to
/// eval-sized magnitudes.
fn naive_eval_white_pov(board: &Board) -> Score {
    let mut mg: i32 = 0;
    let mut eg: i32 = 0;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            let material = i32::from(NAIVE_PIECE_VALUES[cp.piece().index()]);
            let mg_positional = i32::from(pst_value(cp.color(), cp.piece(), sq));
            let eg_positional = i32::from(pst_value_eg(cp.color(), cp.piece(), sq));
            let sign = match cp.color() {
                Color::White => 1,
                Color::Black => -1,
            };
            mg += sign * (material + mg_positional);
            eg += sign * (material + eg_positional);
        }
    }
    let phase = naive_game_phase(board);
    let blended = (mg * (256 - phase) + eg * phase) / 256;
    Score::try_from(blended)
        .expect("eval magnitudes stay well under i16::MAX, per eval::Score's own invariant")
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
