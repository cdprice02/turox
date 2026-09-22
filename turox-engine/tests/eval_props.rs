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

#![expect(
    clippy::expect_used,
    reason = "`clippy.toml`'s allow-expect-in-tests reaches `#[test]` functions and `#[cfg(test)]` modules, but not plain helpers in an integration test or bench, where a failed fixture should abort the run"
)]

mod common;

use common::{any_board, mirrored};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::endgame_scale::{self, ScaleFactor};
use turox_engine::eval::pst::{pst_value, pst_value_eg};
use turox_engine::eval::{eval_white_pov, evaluate, weights, Score};
use turox_engine::{Color, Piece, Square};

// ---- Independent reference ----
//
// Independent in the way that matters: every function below walks
// `board.piece_at` square by square and recomputes the term from scratch,
// sharing none of the bitboard tricks, file masks, or packed accumulator the
// real implementation uses. That walk is the thing being checked.
//
// The magnitudes are *not* independent, deliberately. They are imported from
// `eval::weights`, the same place the implementation reads them. A tuned
// constant has no independent truth to check against, so a second copy only
// ever tested that the two copies matched, and made every retune a two-place
// edit that failed loudly when someone updated one. What the tables and
// weights themselves are worth is pinned separately, by structure rather than
// by transcription; see `tests/eval.rs`.

/// The mg/eg halves as plain `i32`, which is what the naive walks accumulate
/// in. `eval::weights` stores them as `(Score, Score)` because that is what
/// the implementation packs from.
fn mg_eg(w: (Score, Score)) -> (i32, i32) {
    (i32::from(w.0), i32::from(w.1))
}

/// The real scale factor for `board`.
///
/// Deliberately not a naive reimplementation. The version this replaced walked
/// the same early returns in the same order with the same names, which is
/// sharing reasoning rather than checking it: it could catch a typo and not a
/// logic error, and `CONTEXT.md` rules that out for a naive reference.
///
/// The properties below are not testing the scale factor; they bound pawn
/// structure and king safety. The factor is a common multiplier that has to
/// match on both sides of a deviation for those bounds to mean anything, so
/// sharing it is what makes them correct. What the factor itself does is pinned
/// by concrete positions in `tests/eval.rs`.
fn real_scale_factor(board: &Board) -> ScaleFactor {
    endgame_scale::scale_factor(board)
}

/// Whether `board`'s material triggers any scaling at all.
fn triggers_endgame_scale(board: &Board) -> bool {
    !real_scale_factor(board).is_one()
}

/// Mailbox walk over `board.piece_at`, reproducing `eval::phase::game_phase`
/// without calling it: sums `weights::PHASE_WEIGHT` for every non-pawn,
/// non-king piece found, subtracts that from `weights::TOTAL_PHASE`, clamps
/// to non-negative (`any_board()` can place more non-pawn material than a
/// real game has, which `TOTAL_PHASE` does not account for), and scales the
/// result to `0..=256`.
fn naive_game_phase(board: &Board) -> i32 {
    let mut phase = i32::try_from(weights::TOTAL_PHASE).expect("24 fits i32");
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if !matches!(cp.piece(), Piece::Pawn | Piece::King) {
                phase -= i32::try_from(weights::PHASE_WEIGHT[cp.piece().index()])
                    .expect("phase weights are small");
            }
        }
    }
    let phase = phase.max(0);
    (phase * 256 + i32::try_from(weights::TOTAL_PHASE).expect("24 fits i32") / 2)
        / i32::try_from(weights::TOTAL_PHASE).expect("24 fits i32")
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
            let material = i32::from(weights::PIECE_VALUES[cp.piece().index()]);
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
            mg += doubled * mg_eg(weights::DOUBLED_PENALTY).0;
            eg += doubled * mg_eg(weights::DOUBLED_PENALTY).1;
        }
    }

    for sq in own_pawns {
        let file = sq.file().index();
        let west_occupied = file.checked_sub(1).is_some_and(|f| file_counts[f] > 0);
        let east_occupied = file_counts.get(file + 1).is_some_and(|&c| c > 0);
        if !west_occupied && !east_occupied {
            mg += mg_eg(weights::ISOLATED_PENALTY).0;
            eg += mg_eg(weights::ISOLATED_PENALTY).1;
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
            mg += mg_eg(weights::PASSED_BONUS).0;
            eg += mg_eg(weights::PASSED_BONUS).1;
        }
    }

    (mg, eg)
}

/// Mailbox-only reference for `color`'s king-safety contribution: shelter,
/// open-file, and storm penalties (see `eval::king_safety`'s own doc for
/// the exact contract each is scored against), returned as an unblended
/// `(mg, eg)` pair for the same reason `naive_pawn_structure_mg_eg` is.
/// `eg` is always `0`: every real king-safety term lives in the midgame
/// lane only, so this doesn't even track a separate endgame total.
///
/// Shares no bitboard tricks with `eval::king_safety`: zone files and the
/// storm cone are both plain index arithmetic over `File`/`Rank` indices,
/// not `Bitboard::shift`/`file_fill`.
fn naive_king_safety_mg_eg(board: &Board, color: Color) -> (i32, i32) {
    let Some(king_sq) = Square::ALL
        .into_iter()
        .find(|&sq| matches!(board.piece_at(sq), Some(cp) if cp.color() == color && cp.piece() == Piece::King))
    else {
        return (0, 0);
    };

    let mut own_pawn_files = [false; 8];
    let mut enemy_pawn_files = [false; 8];
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if cp.piece() == Piece::Pawn {
                if cp.color() == color {
                    own_pawn_files[sq.file().index()] = true;
                } else {
                    enemy_pawn_files[sq.file().index()] = true;
                }
            }
        }
    }

    let king_file = king_sq.file().index();
    let zone_files: Vec<usize> = [
        king_file.checked_sub(1),
        Some(king_file),
        king_file.checked_add(1),
    ]
    .into_iter()
    .flatten()
    .filter(|&f| f < 8)
    .collect();

    let mut mg = 0i32;
    for &file in &zone_files {
        if !own_pawn_files[file] {
            mg += i32::from(weights::SHELTER_PENALTY.0);
        }
        if !own_pawn_files[file] && !enemy_pawn_files[file] {
            mg += i32::from(weights::OPEN_FILE_PENALTY.0);
        }
    }

    let king_rank = i32::try_from(king_sq.rank().index()).expect("rank index fits i32");
    for sq in Square::ALL {
        let Some(cp) = board.piece_at(sq) else {
            continue;
        };
        if cp.piece() != Piece::Pawn
            || cp.color() == color
            || !zone_files.contains(&sq.file().index())
        {
            continue;
        }
        let sq_rank = i32::try_from(sq.rank().index()).expect("rank index fits i32");
        let ranks_ahead = match color {
            Color::White => sq_rank - king_rank,
            Color::Black => king_rank - sq_rank,
        };
        let storm_range =
            i32::try_from(usize::from(weights::STORM_RANGE)).expect("STORM_RANGE fits i32");
        if (1..=storm_range).contains(&ranks_ahead) {
            mg += i32::from(weights::STORM_PENALTY.0);
        }
    }

    (mg, 0)
}

/// Mailbox-only reference for `color`'s bishop-pair contribution:
/// `weights::BISHOP_PAIR_BONUS` if `color` has two or more bishops on the
/// board, zero otherwise. Flat across both phases, so this returns the
/// same `mg`/`eg` pair whenever it applies, for the same reason
/// `naive_king_safety_mg_eg` always returns `0` for its own `eg` half.
fn naive_bishop_pair_mg_eg(board: &Board, color: Color) -> (i32, i32) {
    let count = Square::ALL
        .into_iter()
        .filter(|&sq| {
            matches!(board.piece_at(sq), Some(cp) if cp.color() == color && cp.piece() == Piece::Bishop)
        })
        .count();
    if count >= 2 {
        mg_eg(weights::BISHOP_PAIR_BONUS)
    } else {
        (0, 0)
    }
}

/// Mailbox-only reference for `color`'s rook-file contribution:
/// `weights::ROOK_OPEN_FILE_BONUS` for each of `color`'s rooks on a file
/// with no pawn of either colour, `weights::ROOK_SEMI_OPEN_FILE_BONUS` for
/// each on a file with no friendly pawn but at least one enemy pawn, zero
/// for every other rook. Flat across both phases, same reasoning as
/// `naive_bishop_pair_mg_eg`.
fn naive_rook_files_mg_eg(board: &Board, color: Color) -> (i32, i32) {
    let mut own_pawn_files = [false; 8];
    let mut enemy_pawn_files = [false; 8];
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            if cp.piece() == Piece::Pawn {
                if cp.color() == color {
                    own_pawn_files[sq.file().index()] = true;
                } else {
                    enemy_pawn_files[sq.file().index()] = true;
                }
            }
        }
    }

    let mut total = 0i32;
    for sq in Square::ALL {
        let Some(cp) = board.piece_at(sq) else {
            continue;
        };
        if cp.piece() != Piece::Rook || cp.color() != color {
            continue;
        }
        let file = sq.file().index();
        if !own_pawn_files[file] && !enemy_pawn_files[file] {
            total += i32::from(weights::ROOK_OPEN_FILE_BONUS.0);
        } else if !own_pawn_files[file] {
            total += i32::from(weights::ROOK_SEMI_OPEN_FILE_BONUS.0);
        }
    }
    (total, total)
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

/// The total number of rooks (both colors) on `board`: the scale the
/// rook-file deviation bound below is measured against, the same
/// discipline `total_pawn_count` uses for pawn structure.
fn total_rook_count(board: &Board) -> i32 {
    let mut count = 0i32;
    for sq in Square::ALL {
        if matches!(board.piece_at(sq), Some(cp) if cp.piece() == Piece::Rook) {
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
#[expect(
    clippy::similar_names,
    reason = "mg/eg is the tapered-eval jargon pair this whole module (and eval::phase) is built on; white_mg/white_eg read as a pair for exactly that reason, not a typo risk"
)]
fn naive_eval_white_pov(board: &Board) -> Score {
    let (mut mg, mut eg) = naive_material_and_pst_mg_eg(board);
    let (white_mg, white_eg) = naive_pawn_structure_mg_eg(board, Color::White);
    let (black_mg, black_eg) = naive_pawn_structure_mg_eg(board, Color::Black);
    mg += white_mg - black_mg;
    eg += white_eg - black_eg;
    let (white_ks_mg, white_ks_eg) = naive_king_safety_mg_eg(board, Color::White);
    let (black_ks_mg, black_ks_eg) = naive_king_safety_mg_eg(board, Color::Black);
    mg += white_ks_mg - black_ks_mg;
    eg += white_ks_eg - black_ks_eg;
    let (white_bp_mg, white_bp_eg) = naive_bishop_pair_mg_eg(board, Color::White);
    let (black_bp_mg, black_bp_eg) = naive_bishop_pair_mg_eg(board, Color::Black);
    mg += white_bp_mg - black_bp_mg;
    eg += white_bp_eg - black_bp_eg;
    let (white_rf_mg, white_rf_eg) = naive_rook_files_mg_eg(board, Color::White);
    let (black_rf_mg, black_rf_eg) = naive_rook_files_mg_eg(board, Color::Black);
    mg += white_rf_mg - black_rf_mg;
    eg += white_rf_eg - black_rf_eg;
    real_scale_factor(board).apply(blend(mg, eg, naive_game_phase(board)))
}

/// The standard tapered-eval blend, `(mg * (256 - phase) + eg * phase) /
/// 256`, factored out of `naive_eval_white_pov` and every "one term
/// switched off" baseline below it: each of those differs from the others
/// only in which `mg_eg` terms it sums before reaching this, not in how
/// the blend itself works, so sharing this one formula keeps that the only
/// difference visible at each call site.
fn blend(mg: i32, eg: i32, phase: i32) -> Score {
    let blended = (mg * (256 - phase) + eg * phase) / 256;
    Score::try_from(blended)
        .expect("eval magnitudes stay well under i16::MAX, per eval::Score's own invariant")
}

/// A term this reference can leave out, so a property can measure what the
/// real evaluation adds for it.
///
/// One parameterised baseline rather than one function per term: each new
/// evaluation term otherwise needs its own near-identical copy of the walk
/// below, and the queued terms would have turned two of those into six.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OmittedTerm {
    /// Leaves out pawn structure, so the deviation isolates it.
    PawnStructure,
    /// Leaves out king safety, so the deviation isolates that instead.
    KingSafety,
    /// Leaves out the bishop pair, so the deviation isolates that instead.
    BishopPair,
    /// Leaves out rook files, so the deviation isolates that instead.
    RookFiles,
}

/// Material and PST, plus every term except `omit`, White-relative.
///
/// `eval_white_pov`'s deviation from this is the omitted term's own
/// contribution and nothing else, which is what lets the bounds below be
/// stated per term rather than as one combined fudge factor.
#[expect(
    clippy::similar_names,
    reason = "mg/eg is the tapered-eval jargon pair this whole module (and eval::phase) is built on; white_mg/white_eg read as a pair for exactly that reason, not a typo risk"
)]
fn naive_white_pov_omitting(board: &Board, omit: OmittedTerm) -> Score {
    let (mut mg, mut eg) = naive_material_and_pst_mg_eg(board);

    if omit != OmittedTerm::PawnStructure {
        let (white_mg, white_eg) = naive_pawn_structure_mg_eg(board, Color::White);
        let (black_mg, black_eg) = naive_pawn_structure_mg_eg(board, Color::Black);
        mg += white_mg - black_mg;
        eg += white_eg - black_eg;
    }

    if omit != OmittedTerm::KingSafety {
        let (white_mg, white_eg) = naive_king_safety_mg_eg(board, Color::White);
        let (black_mg, black_eg) = naive_king_safety_mg_eg(board, Color::Black);
        mg += white_mg - black_mg;
        eg += white_eg - black_eg;
    }

    if omit != OmittedTerm::BishopPair {
        let (white_mg, white_eg) = naive_bishop_pair_mg_eg(board, Color::White);
        let (black_mg, black_eg) = naive_bishop_pair_mg_eg(board, Color::Black);
        mg += white_mg - black_mg;
        eg += white_eg - black_eg;
    }

    if omit != OmittedTerm::RookFiles {
        let (white_mg, white_eg) = naive_rook_files_mg_eg(board, Color::White);
        let (black_mg, black_eg) = naive_rook_files_mg_eg(board, Color::Black);
        mg += white_mg - black_mg;
        eg += white_eg - black_eg;
    }

    real_scale_factor(board).apply(blend(mg, eg, naive_game_phase(board)))
}

/// The largest absolute value a `(mg, eg)` weight can contribute once blended.
///
/// `interpolate` never returns a value outside the span of the `mg` and `eg`
/// it is given, so whichever half is larger bounds the blended result too.
fn max_magnitude(w: (Score, Score)) -> i32 {
    i32::from(w.0).abs().max(i32::from(w.1).abs())
}

/// The most pawn structure can be worth, per pawn on the board, combined
/// across both colours.
///
/// Derived from the weights rather than written as a number: a retune moves
/// this bound with it, instead of leaving a stale literal that gets widened
/// the next time it fails.
fn pawn_structure_bound_per_pawn() -> i32 {
    max_magnitude(weights::DOUBLED_PENALTY)
        + max_magnitude(weights::ISOLATED_PENALTY)
        + max_magnitude(weights::PASSED_BONUS)
}

/// The most king safety can be worth, for both kings together.
///
/// The king zone is three files wide whatever else is on the board, and the
/// storm cone is those three files by `STORM_RANGE` ranks, so this is a fixed
/// amount rather than one scaling with material.
fn king_safety_bound() -> i32 {
    const ZONE_FILES: i32 = 3;
    let per_king = ZONE_FILES * max_magnitude(weights::SHELTER_PENALTY)
        + ZONE_FILES * max_magnitude(weights::OPEN_FILE_PENALTY)
        + ZONE_FILES * i32::from(weights::STORM_RANGE) * max_magnitude(weights::STORM_PENALTY);
    2 * per_king
}
/// The most the bishop pair can be worth, for both sides together: each
/// side either has the flat bonus or doesn't, so this is a fixed amount
/// rather than one that scales with anything on the board, the same
/// discipline `king_safety_bound` uses.
fn bishop_pair_bound() -> i32 {
    2 * max_magnitude(weights::BISHOP_PAIR_BONUS)
}

/// The most rook-file bonus can be worth per rook: whichever of open or
/// semi-open is larger, since a rook scores at most one of the two. Scaled
/// by rook count the same way `pawn_structure_bound_per_pawn` is scaled by
/// pawn count, not a fixed amount like `bishop_pair_bound`: unlike the
/// bishop pair, which caps at one bonus per side regardless of how many
/// bishops beyond two exist, every rook on the board can independently
/// draw its own bonus.
fn rook_files_bound_per_rook() -> i32 {
    max_magnitude(weights::ROOK_OPEN_FILE_BONUS)
        .max(max_magnitude(weights::ROOK_SEMI_OPEN_FILE_BONUS))
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
    //
    // King safety adds one more way this could, in principle, break: a
    // removed black pawn can be the only thing keeping one of *White's own*
    // zone files from reading as open (`open_file_penalty` needs both
    // colors' pawns absent), so removing it can cost White up to
    // `OPEN_FILE_PENALTY` (25 mg) on that one file. That's smaller than a
    // pawn's own material value (100), so it can't flip the sign by itself,
    // but it's a real, bounded-not-structural reason, same as the PST note
    // above. `eval::king_safety`'s constants are first-pass placeholders
    // sized by reasoning rather than measured games; worth re-checking
    // again if they're ever tuned past a pawn's value.
    //
    // `eval::endgame_scale` breaks this property outright for boards it
    // touches, in either direction, not just down to a tie: `K+N vs K+P`
    // (a pawn on the board, so no signature matches, score passes through
    // unscaled) can score *better* for White than the `K+N vs K` left after
    // removing that pawn (a real known draw, scored exactly `0`), because
    // the unscaled pre-removal score has no way to express "that pawn was
    // never going to matter anyway." That's not a bug to chase: it's the
    // same plateau-and-cliff shape the whole feature is built on, just
    // visible from the removal side instead of the comparison side. So
    // this property is restricted to the boards the feature doesn't touch,
    // where it's exactly as strict as it was before `endgame_scale`
    // existed, via `triggers_endgame_scale` on both the position being
    // reduced and the result of reducing it.
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
        if triggers_endgame_scale(&board) || triggers_endgame_scale(&reduced) {
            return Ok(());
        }
        prop_assert!(eval_white_pov(&reduced) > eval_white_pov(&board));
    }

    // A loose sanity bound rather than a tight one, but a real structural
    // guarantee, not an arbitrary number: each pawn can contribute at most one
    // doubled penalty, one isolated penalty, and one passed bonus to its own
    // side's total, and `interpolate` can never blend a result outside the
    // span of the `mg` and `eg` totals it is given (`eval::phase`'s own test
    // module establishes that). `pawn_structure_bound_per_pawn` sums those
    // three from `eval::weights`, so the bound tracks a retune instead of
    // going stale and being widened the next time it fails.
    //
    // What it catches: a doubled- or isolated-counting bug that scales with
    // the number of *pairs* of pawns rather than the number of pawns, which
    // blows past a linear bound as soon as a board has more than a handful,
    // and `any_board()` generates those regularly.
    #[test]
    fn pawn_structure_contribution_is_bounded_by_pawn_count(board in any_board()) {
        let deviation = i32::from(eval_white_pov(&board))
            - i32::from(naive_white_pov_omitting(&board, OmittedTerm::PawnStructure));
        let bound = pawn_structure_bound_per_pawn() * total_pawn_count(&board);
        prop_assert!(
            deviation.abs() <= bound,
            "pawn-structure deviation {deviation} exceeds the {bound}-centipawn bound for {} pawns",
            total_pawn_count(&board)
        );
    }

    // The same discipline as `pawn_structure_contribution_is_bounded_by_pawn_count`,
    // but a fixed bound rather than one scaled by pawn count: the king zone is
    // three files wide whatever else is on the board, so king safety's maximum
    // contribution is a constant rather than a function of material.
    // `king_safety_bound` derives it from `eval::weights` for the same reason
    // the pawn-structure bound is derived.
    #[test]
    fn king_safety_contribution_is_bounded_by_a_fixed_amount(board in any_board()) {
        let deviation = i32::from(eval_white_pov(&board))
            - i32::from(naive_white_pov_omitting(&board, OmittedTerm::KingSafety));
        let bound = king_safety_bound();
        prop_assert!(
            deviation.abs() <= bound,
            "king-safety deviation {deviation} exceeds the {bound}-centipawn bound"
        );
    }

    // Same discipline as the two bounds above, but for a term that's either
    // fully on or fully off per side rather than scaling with a count: this
    // is what would catch a per-bishop bonus (stacking past `BISHOP_PAIR_BONUS`
    // for a third or fourth bishop) that the concrete FEN tests in
    // `tests/eval.rs` might not happen to construct.
    #[test]
    fn bishop_pair_contribution_is_bounded_by_a_fixed_amount(board in any_board()) {
        let deviation = i32::from(eval_white_pov(&board))
            - i32::from(naive_white_pov_omitting(&board, OmittedTerm::BishopPair));
        let bound = bishop_pair_bound();
        prop_assert!(
            deviation.abs() <= bound,
            "bishop-pair deviation {deviation} exceeds the {bound}-centipawn bound"
        );
    }

    // Same discipline as `pawn_structure_contribution_is_bounded_by_pawn_count`:
    // rook-file bonus scales with rook count, not a fixed cap like the
    // bishop pair, so what catches a per-file-instead-of-per-rook bug (or
    // one that double-counts open and semi-open on the same rook) is a
    // bound that scales too.
    #[test]
    fn rook_files_contribution_is_bounded_by_rook_count(board in any_board()) {
        let deviation = i32::from(eval_white_pov(&board))
            - i32::from(naive_white_pov_omitting(&board, OmittedTerm::RookFiles));
        let bound = rook_files_bound_per_rook() * total_rook_count(&board);
        prop_assert!(
            deviation.abs() <= bound,
            "rook-file deviation {deviation} exceeds the {bound}-centipawn bound for {} rooks",
            total_rook_count(&board)
        );
    }
}
