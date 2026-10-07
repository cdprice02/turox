//! Concrete positions for `eval::pawn_structure`, each checked against a
//! hand-computed integer.
//!
//! Every position has only kings and pawns, so `game_phase` is exactly 256 and
//! `interpolate` returns the endgame half with no rounding. Kings stay on e1/e8
//! in both positions of a pair, so their PST cancels. Tropism does not cancel,
//! because it depends on every pawn's distance to each king;
//! `e1_e8_tropism_delta` gives one pawn's net contribution, and since tropism
//! sums linearly, adding or removing a pawn moves the total by exactly that
//! pawn's delta.
//!
//! Pawn PST values the expected numbers use: White d2 -20, d4 +20, d5 +25, d6
//! +30, e3 0; Black d7 -20, d4 +25.

use turox_chess::board::Board;
use turox_chess::{Color, Piece, Square};
use turox_engine::eval::pst::pst_value_eg;
use turox_engine::eval::{eval_white_pov, weights, Score};

/// The longest possible `Square::distance` (Chebyshev) between two squares
/// on an 8x8 board, reproduced here independently of the private
/// `eval::outposts::MAX_DISTANCE` (unreachable from this integration-test crate).
const MAX_DISTANCE: u8 = 7;

/// One pawn's own net tropism contribution to `eval_white_pov` with White's
/// king fixed at e1 and Black's at e8, the convention every position in this
/// file uses. Unlike PST, which depends only on the king's own square and so
/// cancels for free whenever a comparison holds both kings fixed, tropism
/// depends on the king's distance to *every pawn*, so it changes whenever a
/// pawn is added, removed, or moved even with both kings pinned.
/// `weights::TROPISM_BONUS` sums linearly per pawn, though, so one pawn's own
/// net contribution (White's tropism from it minus Black's) is exactly what a
/// comparison that adds or removes that one pawn needs, independent of whatever
/// other pawns are already on the board.
fn e1_e8_tropism_delta(sq: Square) -> Score {
    let white = weights::TROPISM_BONUS.1 * Score::from(MAX_DISTANCE - Square::E1.distance(sq));
    let black = weights::TROPISM_BONUS.1 * Score::from(MAX_DISTANCE - Square::E8.distance(sq));
    white - black
}

// One doubled pawn (d2, d4) against a d2-only baseline. Adding d4 changes
// five independent terms, all attributable to the new pawn alone: its own
// material and PST (100 + 20), its own isolated penalty (-10, since c/e
// stay empty in both positions), its own passed bonus (+20, since neither
// position has any Black pawn anywhere), one doubled penalty newly
// appearing on the d-file now that it holds two pawns instead of one
// (-20), and its own net tropism contribution against the fixed e1/e8
// kings (+2). d2's own isolated/passed/tropism status is unchanged by d4
// arriving, since isolation only looks at adjacent files, passed status
// only looks at enemy pawns, and tropism sums linearly per pawn, none of
// which d4 disturbs. 100 + 20 - 10 + 20 - 20 + 2 = 112.
#[test]
fn one_doubled_pawn_scores_material_plus_pst_minus_one_doubled_penalty() {
    let one_pawn = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    let two_pawns = Board::try_from_fen("4k3/8/8/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");

    // The new d4 pawn's own material and PST, its own isolated penalty (c and
    // e stay empty in both), its own passed bonus (no Black pawn anywhere),
    // the one doubled penalty it creates, and its own net tropism against the
    // fixed e1/e8 kings. Endgame throughout, so the eg half of each tapered
    // weight is the one that lands.
    assert_eq!(
        eval_white_pov(&two_pawns) - eval_white_pov(&one_pawn),
        weights::PIECE_VALUES[Piece::Pawn.index()]
            + pst_value_eg(Color::White, Piece::Pawn, Square::D4)
            + weights::ISOLATED_PENALTY.1
            + weights::PASSED_BONUS.1
            + weights::DOUBLED_PENALTY.1
            + e1_e8_tropism_delta(Square::D4)
    );
}

// The case that catches counting doubled pawns as "N per file" instead of
// "N - 1", or double-charging the penalty multiplicatively: a third pawn
// on the same file (d2, d4, d6) must add exactly one more doubled penalty
// on top of the two-pawn case above, not a second one and not a
// proportionally larger one. d6's own contribution: material + PST
// (100 + 30), its own isolated penalty (-10) and passed bonus (+20), the
// file's doubled count moving from one penalty (two pawns) to two (three
// pawns), i.e. one more -20, and its own net tropism against the fixed
// e1/e8 kings (-6). 100 + 30 - 10 + 20 - 20 - 6 = 114.
#[test]
fn a_third_doubled_pawn_adds_exactly_one_more_doubled_penalty() {
    let two_pawns = Board::try_from_fen("4k3/8/8/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    let three_pawns =
        Board::try_from_fen("4k3/8/3P4/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(
        eval_white_pov(&three_pawns) - eval_white_pov(&two_pawns),
        120 + e1_e8_tropism_delta(Square::D6)
    );

    // And the three-pawn position as a whole scores two doubled penalties'
    // worth below a single-pawn baseline, not one (230, the pre-tropism
    // total the two deltas above would sum to), plus both new pawns' own
    // net tropism (d4's +2 and d6's -6): 230 - 4 = 226.
    let one_pawn = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(
        eval_white_pov(&three_pawns) - eval_white_pov(&one_pawn),
        230 + e1_e8_tropism_delta(Square::D4) + e1_e8_tropism_delta(Square::D6)
    );
}

// A lone, isolated d4 pawn against the same pawn once c4 arrives to
// support it. Both positions include a Black pawn on d5 directly ahead of
// d4, purely to keep d4 (and c4, once it exists) blocked from passed
// status in both positions, so the passed-pawn term stays at zero on both
// sides of the comparison and doesn't leak into a delta meant to isolate
// the isolated-pawn term specifically; d5 itself is identical in both
// positions, so its own contribution (subtracted from White's POV either
// way) cancels out of the difference too. Adding c4 removes d4's isolated
// penalty (it now has a same-color neighbor on an adjacent file) and
// contributes c4's own material and PST (100 + 0, c4's PST entry is 0),
// plus its own net tropism against the fixed e1/e8 kings (+2); c4 isn't
// isolated either, since d4 is right next to it. 100 + 0 + 10 + 2 = 112.
#[test]
fn adding_an_adjacent_pawn_removes_the_isolated_penalty() {
    let isolated = Board::try_from_fen("4k3/8/8/3p4/3P4/8/8/4K3 w - - 0 1").expect("valid FEN");
    let supported = Board::try_from_fen("4k3/8/8/3p4/2PP4/8/8/4K3 w - - 0 1").expect("valid FEN");

    // The new c4 pawn's own material, PST, and net tropism, plus the isolated
    // penalty d4 no longer pays now that it has a neighbour. Subtracting the
    // penalty is what removing it means, which is why this term is a minus.
    assert_eq!(
        eval_white_pov(&supported) - eval_white_pov(&isolated),
        weights::PIECE_VALUES[Piece::Pawn.index()]
            + pst_value_eg(Color::White, Piece::Pawn, Square::C4)
            - weights::ISOLATED_PENALTY.1
            + e1_e8_tropism_delta(Square::C4)
    );
}

// A lone White d5 pawn with a completely clear path to promotion (no
// Black pawns anywhere) against the same pawn with a Black pawn newly
// placed on d7, directly ahead on its own file: exactly what
// `front_attack_span` includes, so this must cancel d5's passed bonus.
// The clear-path position scores higher by two independent things that
// both disappear once d7 shows up: White's own passed bonus (+20) and the
// entire value Black's new pawn brings to Black's side of the score,
// which is subtracted from White's POV and so *raises* White's total when
// it's absent. Black d7 alone: material + PST (100 + -20, the same
// pawn-structure penalty anchor used elsewhere in this file), isolated
// (-10, no Black pawn on c or e), not passed (0, blocked by White's own
// d5, which sits on d7's `front_attack_span(Black)`): 100 - 20 - 10 = 70.
// Tropism doesn't care which color a pawn belongs to, only which king is
// measuring distance to it, so d7 also removes its own net tropism
// against the fixed e1/e8 kings when it disappears (-(-10), a pawn far
// from White's king and close to Black's): 20 + 70 + 10 = 100.
#[test]
fn a_lone_passed_pawn_loses_its_bonus_once_blocked_on_its_own_file() {
    let clear_path = Board::try_from_fen("4k3/8/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked = Board::try_from_fen("4k3/3p4/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");

    // Two independent things disappear when Black's d7 pawn shows up: White's
    // own passed bonus, and the whole value that pawn brings to Black's side,
    // which is subtracted from White's POV and so raises White's total by its
    // absence. Black's d7 is itself isolated and not passed, blocked by
    // White's d5.
    let black_d7 = weights::PIECE_VALUES[Piece::Pawn.index()]
        + pst_value_eg(Color::Black, Piece::Pawn, Square::D7)
        + weights::ISOLATED_PENALTY.1;
    assert_eq!(
        eval_white_pov(&clear_path) - eval_white_pov(&blocked),
        weights::PASSED_BONUS.1 + black_d7 - e1_e8_tropism_delta(Square::D7)
    );
}

// The same clear-path d5 pawn, but blocked by a Black pawn on e6 instead
// of directly ahead on the d-file: still inside `front_attack_span` (the
// span widens one file either way), so this must disqualify d5 from
// passed status just as directly as the same-file case above did. White
// loses its own passed bonus (+20, unaffected by which of the three files
// the blocker sits on) and Black's new e6 pawn's own value is no longer
// subtracted: material + PST (100 + 0, e6's entry on Black's own table is
// 0), isolated (-10, nothing on d or f), not passed (0, blocked by White's
// d5, which sits on e6's `front_attack_span(Black)` too): 100 - 10 = 90.
// e6 also removes its own net tropism against the fixed e1/e8 kings when
// it disappears, the same as d7 did in the test above (-(-6)):
// 20 + 90 + 6 = 116.
#[test]
fn an_adjacent_file_blocker_also_disqualifies_a_passed_pawn() {
    let clear_path = Board::try_from_fen("4k3/8/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked_on_adjacent_file =
        Board::try_from_fen("4k3/8/4p3/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(
        eval_white_pov(&clear_path) - eval_white_pov(&blocked_on_adjacent_file),
        110 - e1_e8_tropism_delta(Square::E6)
    );
}

// The mirror of the two passed-pawn cases above, for Black advancing
// toward rank 1 instead of White advancing toward rank 8: the asymmetric
// case most likely to catch a friendly/enemy or forward-direction mixup,
// since a bug that swaps White and Black's own logic could still pass a
// same-shaped White-only test by accident. A lone Black d4 pawn (clear
// path to rank 1) against the same pawn once a White pawn appears on e3,
// on the adjacent file and strictly ahead of d4 from Black's own point of
// view (a lower rank), which must disqualify it exactly as the White
// cases above did. White's new e3 pawn contributes its own material + PST
// (100 + 0) directly to White's POV, isolated (-10, nothing on d or f for
// White), not passed (0, blocked by Black's own d4, on e3's
// `front_attack_span(White)`): 100 - 10 = 90. Black's own passed bonus
// disappearing (+20 lost from Black's side, which *raises* White's POV by
// 20 since it's normally subtracted) adds another 20. e3 also brings its
// own net tropism against the fixed e1/e8 kings (+6, a pawn close to
// White's king and far from Black's). 90 + 20 + 6 = 116.
#[test]
fn black_passed_pawn_direction_mirrors_white_not_the_other_way_around() {
    let clear_path = Board::try_from_fen("4k3/8/8/8/3p4/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked = Board::try_from_fen("4k3/8/8/8/3p4/4P3/8/4K3 w - - 0 1").expect("valid FEN");

    // White's new e3 pawn adds its own material, PST, isolated penalty, and
    // net tropism to White's POV directly, and is not passed itself, blocked
    // by Black's d4. Black losing its passed bonus raises White's POV by that
    // much again, since Black's total is subtracted.
    let white_e3 = weights::PIECE_VALUES[Piece::Pawn.index()]
        + pst_value_eg(Color::White, Piece::Pawn, Square::E3)
        + weights::ISOLATED_PENALTY.1
        + e1_e8_tropism_delta(Square::E3);
    assert_eq!(
        eval_white_pov(&blocked) - eval_white_pov(&clear_path),
        white_e3 + weights::PASSED_BONUS.1
    );
}
