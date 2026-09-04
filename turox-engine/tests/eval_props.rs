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

/// Independent of `eval::pawn_structure`'s own constants (`pawn_structure`
/// is private to `eval`, unreachable from this integration-test crate
/// anyway): the same `(mg, eg)` values, transcribed again here for the
/// same reason `NAIVE_PIECE_VALUES` isn't shared either.
const NAIVE_DOUBLED_PENALTY: (i32, i32) = (-10, -20);
const NAIVE_ISOLATED_PENALTY: (i32, i32) = (-10, -10);
const NAIVE_PASSED_BONUS: (i32, i32) = (10, 20);

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
/// representation, unlike `eval::phase::Tapered`), material and
/// piece-square terms only: the pre-pawn-structure baseline
/// `eval_white_pov_deviates_from_material_and_pst_only_by_a_bounded_amount`
/// below is checked against, kept separate from
/// `naive_pawn_structure_mg_eg` rather than folded into one function so
/// that property has an independent "pawn structure switched off" total to
/// compare the real, full `eval_white_pov` against.
fn naive_material_and_pst_mg_eg(board: &Board) -> (i32, i32) {
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
    (mg, eg)
}

/// Mailbox-only reference for `color`'s doubled/isolated/passed pawn terms
/// (see `eval::pawn_structure`'s own doc for the exact contract each one
/// is scored against), returned as an unblended `(mg, eg)` pair rather
/// than a single `Score`: `naive_eval_white_pov` folds this into its own
/// running `mg`/`eg` totals before blending once, the same shape the real
/// implementation's `Tapered` accumulator uses, rather than blending this
/// term in isolation and adding two already-blended numbers together
/// (which would round differently).
///
/// Shares no bitboard tricks with `eval::pawn_structure`: files are
/// counted with a plain `[u32; 8]` mailbox scan, and passed/isolated
/// status is decided by scanning every square directly rather than via
/// `Bitboard::front_attack_span`/`file_fill`.
fn naive_pawn_structure_mg_eg(board: &Board, color: Color) -> (i32, i32) {
    let mut file_counts = [0u32; 8];
    let mut own_pawns: Vec<Square> = Vec::new();
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if cp.color() == color && cp.piece() == Piece::Pawn {
                file_counts[sq.file().index()] += 1;
                own_pawns.push(sq);
            }
        }
    }

    let mut mg = 0i32;
    let mut eg = 0i32;

    for &count in &file_counts {
        if count >= 2 {
            let doubled = i32::try_from(count - 1).expect("a file holds at most 8 pawns");
            mg += doubled * NAIVE_DOUBLED_PENALTY.0;
            eg += doubled * NAIVE_DOUBLED_PENALTY.1;
        }
    }

    for sq in own_pawns {
        let file = sq.file().index();
        let west_occupied = file.checked_sub(1).is_some_and(|f| file_counts[f] > 0);
        let east_occupied = file_counts.get(file + 1).is_some_and(|&c| c > 0);
        if !west_occupied && !east_occupied {
            mg += NAIVE_ISOLATED_PENALTY.0;
            eg += NAIVE_ISOLATED_PENALTY.1;
        }

        let blocked = Square::ALL.into_iter().any(|other| {
            let Some(cp) = board.piece_at(other) else {
                return false;
            };
            if cp.color() != color.flip() || cp.piece() != Piece::Pawn {
                return false;
            }
            let file_diff = i32::try_from(other.file().index()).expect("file index fits i32")
                - i32::try_from(file).expect("file index fits i32");
            if !(-1..=1).contains(&file_diff) {
                return false;
            }
            match color {
                Color::White => other.rank() > sq.rank(),
                Color::Black => other.rank() < sq.rank(),
            }
        });
        if !blocked {
            mg += NAIVE_PASSED_BONUS.0;
            eg += NAIVE_PASSED_BONUS.1;
        }
    }

    (mg, eg)
}

/// The total number of pawns (both colors) on `board`: the scale the
/// pawn-structure deviation bound below is measured against.
fn total_pawn_count(board: &Board) -> i32 {
    let mut count = 0i32;
    for sq in Square::ALL {
        if matches!(board.piece_at(sq), Some(cp) if cp.piece() == Piece::Pawn) {
            count += 1;
        }
    }
    count
}

/// Sums a midgame and an endgame total independently (no packed
/// representation, unlike `eval::phase::Tapered`) and blends them with the
/// standard tapered-eval formula, `(mg * (256 - phase) + eg * phase) / 256`:
/// the widely reproduced technique this whole module is built on, described
/// on the chess programming wiki's "Tapered Eval" page. Widened to `i32`
/// for the multiply, since `mg`/`eg` scaled by up to 256 would overflow
/// `Score` (`i16`) well before the division brings the result back down to
/// eval-sized magnitudes.
#[allow(
    clippy::similar_names,
    reason = "mg/eg is the tapered-eval jargon pair this whole module (and eval::phase) is built on; white_mg/white_eg read as a pair for exactly that reason, not a typo risk"
)]
fn naive_eval_white_pov(board: &Board) -> Score {
    let (mut mg, mut eg) = naive_material_and_pst_mg_eg(board);
    let (white_mg, white_eg) = naive_pawn_structure_mg_eg(board, Color::White);
    let (black_mg, black_eg) = naive_pawn_structure_mg_eg(board, Color::Black);
    mg += white_mg - black_mg;
    eg += white_eg - black_eg;
    let phase = naive_game_phase(board);
    let blended = (mg * (256 - phase) + eg * phase) / 256;
    Score::try_from(blended)
        .expect("eval magnitudes stay well under i16::MAX, per eval::Score's own invariant")
}

/// `naive_material_and_pst_mg_eg`, blended the same way
/// `naive_eval_white_pov` blends its own totals: the "pawn structure
/// switched off" baseline `eval_white_pov`'s deviation is measured
/// against below.
fn naive_material_and_pst_white_pov(board: &Board) -> Score {
    let (mg, eg) = naive_material_and_pst_mg_eg(board);
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

    // A loose sanity bound rather than a tight one, but a real structural
    // guarantee, not an arbitrary number: each pawn can contribute at most
    // one `DOUBLED_PENALTY` (eg magnitude 20), one `ISOLATED_PENALTY` (10),
    // and one `PASSED_BONUS` (20) to its own side's total, and
    // `interpolate` can never blend a result outside the span of the `mg`
    // and `eg` totals it's given (`tests/eval_props.rs`'s sibling property
    // in `eval::phase`'s own test module establishes that). So the total
    // pawn-structure swing, combined across both colors, is bounded by 50
    // centipawns per pawn on the board, not merely "some bound found
    // empirically." A doubled- or isolated-counting bug that scales with
    // the number of *pairs* of pawns rather than the number of pawns
    // (quadratic instead of linear) blows past this bound as soon as a
    // board has more than a handful of pawns, which `any_board()`
    // regularly generates.
    #[test]
    fn pawn_structure_contribution_is_bounded_by_pawn_count(board in any_board()) {
        let deviation =
            i32::from(eval_white_pov(&board)) - i32::from(naive_material_and_pst_white_pov(&board));
        let bound = 50 * total_pawn_count(&board);
        prop_assert!(
            deviation.abs() <= bound,
            "pawn-structure deviation {deviation} exceeds the {bound}-centipawn bound for {} pawns",
            total_pawn_count(&board)
        );
    }
}
