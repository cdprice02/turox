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
use turox_engine::eval::endgame_scale::{scale_factor, ScaleFactor};
use turox_engine::eval::pst::{pst_value, pst_value_eg};
use turox_engine::eval::{eval_white_pov, evaluate, weights, Score};
use turox_engine::{Color, Piece, Square};

/// The longest possible `Square::distance` (Chebyshev) between two squares
/// on an 8x8 board, reproduced here independently of the private
/// `eval::outposts::MAX_DISTANCE` (unreachable from this integration-test
/// crate), the same reason `tests/eval_props.rs` reproduces its own copy.
const MAX_DISTANCE: u8 = 7;

/// One pawn's own net tropism contribution to `eval_white_pov` with White's
/// king fixed at e1 and Black's at e8, the convention most bare-king-plus-
/// pawns positions in this file use (the Pawn structure and King-pawn
/// tropism sections below explain why). Unlike PST, which depends only on
/// the king's own square and so cancels for free whenever a comparison
/// holds both kings fixed, tropism depends on the king's distance to
/// *every pawn*, so it changes whenever a pawn is added, removed, or
/// moved even with both kings pinned. `weights::TROPISM_BONUS` sums
/// linearly per pawn, though, so one pawn's own net contribution
/// (White's tropism from it minus Black's) is exactly what a comparison
/// that adds or removes that one pawn needs, independent of whatever
/// other pawns are already on the board.
fn e1_e8_tropism_delta(sq: Square) -> Score {
    let white = weights::TROPISM_BONUS.1 * Score::from(MAX_DISTANCE - Square::E1.distance(sq));
    let black = weights::TROPISM_BONUS.1 * Score::from(MAX_DISTANCE - Square::E8.distance(sq));
    white - black
}

// Not just an empirical check: `eval_white_pov_is_mirror_antisymmetric` (in
// `tests/eval_props.rs`) guarantees `eval_white_pov(b) == -eval_white_pov(mirrored(b))`
// for any board, and the start position's *placement* is its own mirror
// (White's setup is exactly Black's, rank-flipped and color-swapped) even
// though `mirrored()` also flips side to move, so `mirrored(start)` isn't
// literally `start` itself. Writing `X` for every placement-only term
// (material, PST, pawn structure, king safety, bishop pair, rook files)
// and `T` for `weights::TEMPO_BONUS.0`: `mirrored(start)` has the same `X`
// (identical placement) but the opposite tempo sign (Black to move there),
// so the antisymmetry equation is `X + T == -(X - T)`, which reduces to
// `X == 0` regardless of what's in PST, true for any self-mirror-symmetric
// placement, not a coincidence about these particular table values.
// `eval_white_pov(start)` itself is `X + T == T`, not `0`: tempo is the
// one term the mirror argument doesn't cancel, since it's the one term
// that isn't placement-dependent.
#[test]
fn start_position_scores_only_the_tempo_bonus() {
    let board = Board::start_pos();
    let expected = weights::TEMPO_BONUS.0;
    assert_eq!(eval_white_pov(&board), expected);
    assert_eq!(evaluate(&board), expected);
}

// Stays exact rather than approximate with PST folded in: a1 (rook), e1
// (king), and e8 (king, read via flip_rank as e1) all carry a PST value of
// 0 in the tables above, so this position's PST term is 0 - 0 = 0. Not
// pure material alone, though: a1 has no pawn of either colour on its own
// file, so `eval::rook_files`'s open-file bonus applies on top of the
// rook's raw value, and White has the move, so does `eval::tempo`'s bonus.
//
// Three mirrored queen pairs (b/c/g files, clear of the rook's a-file and
// the kings' e-file) pin combined non-pawn material at exactly
// `TOTAL_PHASE`, the same technique the Bishop pair section above uses, so
// tempo's own contribution is `weights::TEMPO_BONUS.0` exactly rather than
// some phase-blended fraction of it that happens to floor to the same
// value this position's un-pinned phase would have given anyway. The
// queens' own material and PST cancel between the mirrored pairs
// regardless, the same argument the kings' cancellation already rests on.
#[test]
fn white_up_a_rook_scores_its_value_plus_the_open_file_and_tempo_bonuses() {
    let board = Board::try_from_fen("1qq1k1q1/8/8/8/8/8/8/RQQ1K1Q1 w - - 0 1").expect("valid FEN");
    let expected = 500 + weights::ROOK_OPEN_FILE_BONUS.0 + weights::TEMPO_BONUS.0;
    assert_eq!(eval_white_pov(&board), expected);

    let swapped = mirrored(&board);
    assert_eq!(eval_white_pov(&swapped), -expected);
}

// Material is unchanged (one White pawn, relocated); the score changes by
// exactly the pawn's own PST delta, d2 (-20) to d4 (+20) = 40, isolating
// the positional term from the material term.
//
// A lone pawn with no other pawns on the board is isolated (-10/-10) and
// passed (+10/+20, no enemy pawn anywhere to block it), a net (0, +10)
// pawn-structure contribution in both positions; with only two kings and
// one pawn on the board, `game_phase` reads a pure endgame (256), so
// `eval_white_pov` reduces to the eg total alone: 100 material + PST +
// 10 net pawn-structure eg, plus the lone pawn's own net tropism against
// the fixed e1/e8 kings (`e1_e8_tropism_delta`'s own doc explains why this
// doesn't cancel just because both kings stay put).
#[test]
fn a_central_pawn_push_changes_pst_but_not_material() {
    // Spelled as the sum rather than as its total: a retune then moves the
    // expectation with the weights instead of leaving a literal that has to be
    // recomputed by hand, and the arithmetic in a trailing comment silently
    // disagreeing with the assertion beside it is how that goes wrong.
    let material = weights::PIECE_VALUES[Piece::Pawn.index()];
    let structure_eg = weights::ISOLATED_PENALTY.1 + weights::PASSED_BONUS.1;

    let before = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(
        eval_white_pov(&before),
        material
            + pst_value_eg(Color::White, Piece::Pawn, Square::D2)
            + structure_eg
            + e1_e8_tropism_delta(Square::D2)
    );

    let after = Board::try_from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(
        eval_white_pov(&after),
        material
            + pst_value_eg(Color::White, Piece::Pawn, Square::D4)
            + structure_eg
            + e1_e8_tropism_delta(Square::D4)
    );
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

// ---- Tapered eval: king placement by game phase ----

// Only kings and a pawn per side on the board, so `game_phase` reads pure
// endgame (256): the position where the endgame king table's centralization
// bonus should actually show up in `eval_white_pov`, not just in the raw
// table values.
#[test]
fn centralized_king_scores_higher_than_cornered_king_with_low_material() {
    let centralized = Board::try_from_fen("4k3/1p6/8/8/4K3/1P6/8/8 w - - 0 1").expect("valid FEN");
    let cornered = Board::try_from_fen("4k3/1p6/8/8/8/1P6/8/K7 w - - 0 1").expect("valid FEN");

    assert!(eval_white_pov(&centralized) > eval_white_pov(&cornered));
}

// Same king-square comparison, but with a full complement of non-pawn
// material on the board so `game_phase` reads pure midgame (0) instead.
// The midgame king table's own back-rank preference actively fights
// centralization here (unlike the endgame table above), so this doesn't
// assert a sign, only that the low-material comparison's centralization
// preference is the larger of the two: the endgame table's pull toward the
// center is a much bigger swing than whatever the midgame table does with
// the same two squares.
#[test]
fn centralization_preference_is_smaller_with_full_material_than_with_low_material() {
    let low_material_centralized =
        Board::try_from_fen("4k3/1p6/8/8/4K3/1P6/8/8 w - - 0 1").expect("valid FEN");
    let low_material_cornered =
        Board::try_from_fen("4k3/1p6/8/8/8/1P6/8/K7 w - - 0 1").expect("valid FEN");
    let full_material_centralized =
        Board::try_from_fen("rnbqkbnr/pppppppp/8/8/4K3/8/PPPPPPPP/1NBQRBNR w - - 0 1")
            .expect("valid FEN");
    let full_material_cornered =
        Board::try_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/KNBQRBNR w - - 0 1")
            .expect("valid FEN");

    let low_material_delta =
        eval_white_pov(&low_material_centralized) - eval_white_pov(&low_material_cornered);
    let full_material_delta =
        eval_white_pov(&full_material_centralized) - eval_white_pov(&full_material_cornered);
    assert!(low_material_delta > full_material_delta);
}

// The one non-king piece any tapered scheme still has to get right: when
// combined non-pawn material lands exactly on `TOTAL_PHASE` (a full
// complement, 4 knights + 4 bishops + 4 rooks + 2 queens between both
// sides, split unevenly here to show the split itself doesn't matter),
// `game_phase` reads 0 and `eval_white_pov` should reduce to a plain
// midgame-only sum, with no endgame contribution blended in at all.
//
// Every term besides material/PST and tempo cancels out here by
// construction rather than needing to be summed explicitly: no pawns
// exist anywhere, so pawn structure is zero outright, and king safety's
// zone-file penalties (which depend only on whether *any* pawn exists on
// a file, not on which color) land at the same magnitude for both kings
// regardless of their squares. Both sides also happen to hold exactly two
// bishops and two rooks each, so bishop pair and rook files' open-file
// bonus (every file is open with no pawns anywhere) cancel too. Tempo has
// no opposite side to cancel against, so it's added explicitly.
#[test]
fn full_phase_material_total_matches_pure_midgame_sum() {
    let board = Board::try_from_fen("bbrrqk2/8/8/8/8/8/QK6/NNNNBBRR w - - 0 1").expect("valid FEN");
    let mut expected: Score = weights::TEMPO_BONUS.0;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            let value =
                weights::PIECE_VALUES[cp.piece().index()] + pst_value(cp.color(), cp.piece(), sq);
            expected += match cp.color() {
                Color::White => value,
                Color::Black => -value,
            };
        }
    }

    assert_eq!(eval_white_pov(&board), expected);
}

// Concrete anchor for `game_phase`'s clamp: a hand-built position with far
// more non-pawn material than any real game reaches (seven White queens),
// so the raw phase count would go well negative before clamping. Paired
// with the property tests in `tests/eval_props.rs` (which exercise this
// only as often as `any_board()` happens to roll enough extra material)
// rather than relying on randomness alone to hit this exact shape.
#[test]
fn heavily_overloaded_material_does_not_panic_or_invert_the_score() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/QQQQQQQK w - - 0 1").expect("valid FEN");

    assert!(eval_white_pov(&board) > 0);
}

// ---- Pawn structure ----
//
// Every position below has only kings and pawns, so `game_phase` reads
// its maximum (no non-pawn material to subtract from `TOTAL_PHASE`),
// which lands on exactly 256: pure endgame, not just close to it. At that
// exact extreme `interpolate` returns the `eg` half of the packed total
// with no rounding (`eg * 256 / 256` is exact), so comparisons below are
// checked against hand-computed integers, not approximations.
//
// Every pair keeps White's and Black's king on the same square in both
// positions being compared (e1/e8 throughout), so the kings' own (nonzero)
// piece-square contribution cancels out of the difference and doesn't
// need to be computed by hand at all; only the pawns being added or moved
// matter for that part.
//
// `eval::outposts`'s tropism term doesn't get the same free cancellation,
// though, even with both kings held fixed: unlike PST, which depends only
// on the king's own square, tropism depends on the king's distance to
// *every pawn* on the board, so it changes whenever a pawn is added,
// removed, or moved, which is exactly what every test below does.
// `e1_e8_tropism_delta` computes one pawn's own net contribution (White's
// tropism from it minus Black's) with both kings pinned at e1/e8; because
// tropism sums linearly per pawn, adding or removing a single pawn moves
// the total by exactly that pawn's own delta, independent of whatever
// other pawns are already on the board, so isolating one pawn's tropism
// contribution doesn't require recomputing the whole position's total.
//
// White's pawn PST (endgame == midgame for pawns; only the king has a
// separate endgame table) by rank on the d-file: d2 -20, d4 +20, d5 +25,
// d6 +30 (the same numbers `a_central_pawn_push_changes_pst_but_not_material`
// and the orientation anchors above already pin down). Black's d7 is -20
// (own mirrored anchor above); Black's d4 is +25 and e3 (White) is 0,
// read the same way, by working through `pst::pst_value`'s doc.

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

// ---- King safety ----
//
// Every `eval::king_safety` term packs `(mg, 0)`: an `eg` of zero. That
// makes every position below need *some* non-pawn material on the board,
// unlike the pawn-structure section above, which got away with bare kings
// and pawns: at `game_phase`'s pure-endgame extreme (256, no non-pawn
// material at all), `interpolate` returns the `eg` half with no blending,
// which for a king-safety term is always exactly 0. Testing the term's
// real effect needs a phase away from that extreme.
//
// Every FEN below carries the same fixed filler army on ranks 4 and 5 (a
// White queen, two rooks, two bishops, two knights on rank 4; the Black
// mirror on rank 5) for exactly that reason: 4 + 2 + 2 + 1 + 1 + 1 + 1 =
// 12 per side, 24 combined, which is `TOTAL_PHASE` exactly, so
// `game_phase` reads 0 (pure midgame) and `interpolate` reduces to the
// `mg` half with no rounding at all. The filler never moves between any
// two positions being compared and never shares a file or rank with
// anything under test, so its own material and PST contributions are
// identical on both sides of every delta below and cancel out; only its
// existence (to hold the phase at 0) matters. Kings and king-adjacent
// pawns stay on ranks 1/2 and 7/8, well clear of it.

// Fixes White's pawns at c2/d2/e2 (a `d`-file king's full shelter) in both
// positions and only moves the king itself, from d1 (in the shield) to g1
// (in open air on the kingside), so material, every pawn's own PST, and
// the pawn-structure term are all identical in both positions and cancel
// out of the delta entirely: c2/d2/e2 are isolated from each other's
// isolated-pawn status and each other's passed status exactly the same
// way regardless of where the king stands, since neither depends on the
// king's square. Only two things differ: the king's own PST value at its
// two squares (computed here via `pst_value` rather than transcribed, the
// same discipline `full_phase_material_total_matches_pure_midgame_sum`
// above uses), and king safety itself.
//
// At d1: zone files c/d/e all have a White pawn on them, so neither
// `SHELTER_PENALTY` nor `OPEN_FILE_PENALTY` applies to any of the three;
// no Black pawns exist anywhere, so storm is zero too. Total: 0.
// At g1: zone files f/g/h have no pawn of either color on any of them, so
// all three draw both penalties: 3 x (15 + 25) = 120. Total: -120.
// The d1-vs-g1 delta in king safety alone is 0 - (-120) = 120.
#[test]
fn a_full_pawn_shield_scores_better_than_bare_kingside_air() {
    let sheltered =
        Board::try_from_fen("4k3/8/8/qrrbbnn1/QRRBBNN1/8/2PPP3/3K4 w - - 0 1").expect("valid FEN");
    let bare =
        Board::try_from_fen("4k3/8/8/qrrbbnn1/QRRBBNN1/8/2PPP3/6K1 w - - 0 1").expect("valid FEN");

    let king_pst_delta = pst_value(Color::White, Piece::King, Square::D1)
        - pst_value(Color::White, Piece::King, Square::G1);
    let expected = king_pst_delta + 120;
    assert_eq!(eval_white_pov(&sheltered) - eval_white_pov(&bare), expected);
}

// Isolates `OPEN_FILE_PENALTY`'s own marginal contribution from
// `SHELTER_PENALTY`'s, which the position above can't: White's king stays
// fixed on g1 and has no pawns at all in either position, so every zone
// file already draws `SHELTER_PENALTY` in both, a constant that cancels
// out of the delta. The only thing that changes is a lone Black pawn on
// g7 (semi-open toward the king, since a pawn of *some* color still holds
// that file) versus no pawn there at all (fully open, `OPEN_FILE_PENALTY`
// now applies to that one additional file). g7 is far outside the storm
// cone (`STORM_RANGE` ranks from g1's own rank), so storm stays zero in
// both. The lone Black pawn's own pawn-structure contribution is isolated
// (-10, no adjacent Black pawn) plus passed (+10, no White pawn anywhere
// to block it, this position has none) = 0 net, so that term cancels too;
// only its material, its own PST, and the one extra `OPEN_FILE_PENALTY`
// remain.
#[test]
fn open_file_penalty_adds_on_top_of_an_already_missing_shelter_pawn() {
    let semi_open =
        Board::try_from_fen("4k3/6p1/8/qrrbbnn1/QRRBBNN1/8/8/6K1 w - - 0 1").expect("valid FEN");
    let fully_open =
        Board::try_from_fen("4k3/8/8/qrrbbnn1/QRRBBNN1/8/8/6K1 w - - 0 1").expect("valid FEN");

    let black_pawn_material_and_pst = 100 + pst_value(Color::Black, Piece::Pawn, Square::G7);
    let expected = -black_pawn_material_and_pst + 25;
    assert_eq!(
        eval_white_pov(&semi_open) - eval_white_pov(&fully_open),
        expected
    );
}

// Isolates `storm_penalty` the same way the position above isolates
// `OPEN_FILE_PENALTY`: the same lone Black pawn stays on the g-file in
// both positions (so `OPEN_FILE_PENALTY` sees "some pawn on this file"
// either way and doesn't change) and only its *rank* differs, g3 (two
// ranks ahead of White's king on g1, inside `STORM_RANGE`) versus g7 (six
// ranks ahead, well outside it). Material is identical (the same pawn,
// relocated, not added or removed); its own pawn-structure contribution
// is isolated (-10) plus passed (+10) = 0 net at both squares, for the
// same reason as the position above, so that cancels too. Only the pawn's
// own PST delta between the two squares and the storm term itself remain.
#[test]
fn an_advancing_enemy_pawn_costs_more_than_one_still_on_its_home_rank() {
    let close =
        Board::try_from_fen("4k3/8/8/qrrbbnn1/QRRBBNN1/6p1/8/6K1 w - - 0 1").expect("valid FEN");
    let far =
        Board::try_from_fen("4k3/6p1/8/qrrbbnn1/QRRBBNN1/8/8/6K1 w - - 0 1").expect("valid FEN");

    let pawn_pst_delta = pst_value(Color::Black, Piece::Pawn, Square::G7)
        - pst_value(Color::Black, Piece::Pawn, Square::G3);
    let expected = pawn_pst_delta - 10;
    assert_eq!(eval_white_pov(&close) - eval_white_pov(&far), expected);
}

// The asymmetric case this repo's {Color}x{direction} history says to
// write explicitly: the same storm comparison as the position above, but
// for Black's king with a *White* pawn advancing toward it, to catch a
// direction that only happens to work for White. Black's king stays fixed
// on g8; White's king moves to a1, off any file g8 cares about, so White's
// own king safety never changes between the two positions and cancels.
// The White pawn stays on the g-file in both (so `OPEN_FILE_PENALTY` for
// Black doesn't change) and only its rank differs: g6 (two ranks toward
// Black's own back rank, inside `STORM_RANGE`) versus g2 (six ranks away,
// outside it). Same reasoning as above gives the moved pawn's own
// pawn-structure contribution as 0 net at both squares.
#[test]
fn storm_direction_mirrors_for_black_kings_not_just_white_ones() {
    let close =
        Board::try_from_fen("6k1/8/6P1/qrrbbnn1/QRRBBNN1/8/8/K7 w - - 0 1").expect("valid FEN");
    let far =
        Board::try_from_fen("6k1/8/8/qrrbbnn1/QRRBBNN1/8/6P1/K7 w - - 0 1").expect("valid FEN");

    let pawn_pst_delta = pst_value(Color::White, Piece::Pawn, Square::G6)
        - pst_value(Color::White, Piece::Pawn, Square::G2);
    let expected = pawn_pst_delta + 10;
    assert_eq!(eval_white_pov(&close) - eval_white_pov(&far), expected);
}

// ---- Bishop pair ----
//
// `only_the_king_has_a_distinct_endgame_table` (below) pins that bishop PST
// has no separate endgame half, and `weights::BISHOP_PAIR_BONUS` is flat
// across both phases too, so neither depends on what `game_phase` reads.
// `weights::TEMPO_BONUS` is not flat, though (`(mg, 0)`, midgame lane
// only, see the Tempo section below), and every position here has White to
// move with nothing on the Black side to cancel that term against, so
// unlike the bishop-pair math, tempo's own contribution *does* depend on
// phase, and needs it pinned to a known value to stay exact.
//
// Three queens per side (mirrored, so their own material/PST cancel the
// same way the kings' do) lands combined non-pawn material at exactly
// `TOTAL_PHASE`, pinning `game_phase` to 0 and `weights::TEMPO_BONUS.0`
// (not some phase-blended fraction of it) as the exact tempo contribution
// in every position below. Queens also clear `scale_factor`'s very first
// check on their own, standing in for the plain rook earlier drafts of
// this section used for that alone: a bare king facing one or two
// *same-coloured* bishops is insufficient mating material, and
// `eval::endgame_scale` would otherwise scale that straight to zero
// regardless of what material/PST/pair/tempo terms summed to underneath,
// swallowing the exact deltas these tests want to isolate. The two-bishop
// positions use c1/f1, the actual starting squares, so the pair is
// opposite-coloured and would still score unscaled even without the
// queens. Kings sit on their own mirrored squares (e1/e8) in every
// position so their own PST and king-safety contributions cancel out too,
// leaving only material, bishop PST, the pair bonus, and tempo.

// A concrete anchor for the mirror-cancellation setup itself, before
// layering any bishops on: if this ever fails, the queen/king placement
// below isn't cancelling the way the rest of this section assumes, which
// is a different problem than anything bishop-pair-specific. Not literally
// zero: White is to move, and tempo is the one term nothing here cancels
// it against.
#[test]
fn identical_material_besides_bishops_cancels_to_exactly_the_tempo_bonus() {
    let board = Board::try_from_fen("qq2k1q1/8/8/8/8/8/8/QQ2K1Q1 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), weights::TEMPO_BONUS.0);
}

#[test]
fn a_single_bishop_scores_its_own_material_and_pst_with_no_pair_bonus() {
    let board = Board::try_from_fen("qq2k1q1/8/8/8/8/8/8/QQB1K1Q1 w - - 0 1").expect("valid FEN");
    let expected = weights::PIECE_VALUES[Piece::Bishop.index()]
        + pst_value(Color::White, Piece::Bishop, Square::C1)
        + weights::TEMPO_BONUS.0;
    assert_eq!(eval_white_pov(&board), expected);
}

// Crossing from one bishop to two adds the second bishop's own material and
// PST, plus the whole pair bonus in the same step: the delta isolates
// exactly what the extra bishop is worth, bonus included. Tempo cancels
// out of the delta regardless of pinning phase to zero (both positions
// have White to move at the *same* phase either way), but pinning it keeps
// this section's every position on the same footing rather than leaving
// one delta-based test as the odd one out.
#[test]
fn a_second_bishop_adds_its_own_value_plus_the_pair_bonus() {
    let one = Board::try_from_fen("qq2k1q1/8/8/8/8/8/8/QQB1K1Q1 w - - 0 1").expect("valid FEN");
    let two = Board::try_from_fen("qq2k1q1/8/8/8/8/8/8/QQB1KBQ1 w - - 0 1").expect("valid FEN");

    let second_bishop = weights::PIECE_VALUES[Piece::Bishop.index()]
        + pst_value(Color::White, Piece::Bishop, Square::F1);
    let expected = second_bishop + weights::BISHOP_PAIR_BONUS.0;
    assert_eq!(eval_white_pov(&two) - eval_white_pov(&one), expected);
}

// Both sides having the pair must cancel out of `eval_white_pov`'s
// White-minus-Black subtraction, not double-count by summing both sides'
// bonuses into the same side: a `+=` on both colors instead of `+=`/`-=`
// would break `eval_white_pov_is_mirror_antisymmetric`
// (`tests/eval_props.rs`) for every mirror-symmetric board, and this pins
// one concrete instance of that, the same way
// `start_position_scores_only_the_tempo_bonus` pins the general property
// for the start position. Left with only the tempo bonus rather than zero,
// for the same reason that test is: White to move, nothing to cancel it.
#[test]
fn bishop_pairs_on_both_sides_cancel_to_exactly_the_tempo_bonus() {
    let board = Board::try_from_fen("qqb1kbq1/8/8/8/8/8/8/QQB1KBQ1 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), weights::TEMPO_BONUS.0);
}

// ---- Rook files ----
//
// Rook PST has no separate endgame half either (same
// `only_the_king_has_a_distinct_endgame_table` guarantee the bishop-pair
// section above relies on), so every delta below only needs to account for
// material, PST, and the file bonus: no phase arithmetic required. Every
// position in this section keeps its pawns and king squares byte-for-byte
// identical across the positions it's compared against, moving only the
// rook under test, so pawn-structure and king-safety (and, since piece
// counts never change, the phase itself) are identical on both sides of
// every delta and cancel out regardless of what they individually equal.
//
// The fixed background is one White pawn on a2 and one Black pawn on g7:
// together they give every file under test a different state depending on
// which file the tested rook sits on. h-file: no pawn of either colour
// (open). g-file: only Black's pawn (semi-open for White, since White has
// no pawn there). a-file: White's own pawn (closed for White, regardless
// of what's on it for Black).

#[test]
fn an_open_file_rook_scores_more_than_a_closed_file_one() {
    let open = Board::try_from_fen("4k3/6p1/8/8/8/8/P7/4K2R w - - 0 1").expect("valid FEN");
    let closed = Board::try_from_fen("4k3/6p1/8/8/8/8/P7/R3K3 w - - 0 1").expect("valid FEN");

    let rook_pst_delta = pst_value(Color::White, Piece::Rook, Square::H1)
        - pst_value(Color::White, Piece::Rook, Square::A1);
    let expected = rook_pst_delta + weights::ROOK_OPEN_FILE_BONUS.0;
    assert_eq!(eval_white_pov(&open) - eval_white_pov(&closed), expected);
}

#[test]
fn a_semi_open_file_rook_scores_more_than_a_closed_file_one() {
    let semi_open = Board::try_from_fen("4k3/6p1/8/8/8/8/P7/4K1R1 w - - 0 1").expect("valid FEN");
    let closed = Board::try_from_fen("4k3/6p1/8/8/8/8/P7/R3K3 w - - 0 1").expect("valid FEN");

    let rook_pst_delta = pst_value(Color::White, Piece::Rook, Square::G1)
        - pst_value(Color::White, Piece::Rook, Square::A1);
    let expected = rook_pst_delta + weights::ROOK_SEMI_OPEN_FILE_BONUS.0;
    assert_eq!(
        eval_white_pov(&semi_open) - eval_white_pov(&closed),
        expected
    );
}

// The asymmetric case this repo's `{Color}x{direction}` history says to
// write explicitly: the a-file has only a *White* pawn on it, which makes
// it semi-open for a *Black* rook there (no friendly pawn, an enemy pawn
// present) even though the exact same file is closed for a White rook
// (see the two tests above, which put White's own rook on a-file and score
// it as closed). A friendly/enemy swap bug in `rook_files_score` would
// either score this as open (missing the enemy-pawn check entirely) or as
// closed (reading White's pawn as Black's own "friendly" pawn), and either
// wrong answer shows up as a wrong delta below. d-file carries no pawn at
// all, so it's open for a Black rook there, the baseline this compares
// against; White has no rook in either position, and White's own pawn
// stays fixed on a2 throughout, so material, PST, pawn-structure, and
// phase for everything but the Black rook are identical between the two
// FENs and cancel out of the delta.
#[test]
fn a_lone_enemy_pawn_makes_the_file_semi_open_not_closed_for_the_other_color() {
    let semi_open = Board::try_from_fen("r3k3/8/8/8/8/8/P7/4K3 w - - 0 1").expect("valid FEN");
    let open = Board::try_from_fen("3rk3/8/8/8/8/8/P7/4K3 w - - 0 1").expect("valid FEN");

    let rook_pst_delta = pst_value(Color::Black, Piece::Rook, Square::D8)
        - pst_value(Color::Black, Piece::Rook, Square::A8);
    let expected =
        rook_pst_delta + weights::ROOK_OPEN_FILE_BONUS.0 - weights::ROOK_SEMI_OPEN_FILE_BONUS.0;
    assert_eq!(eval_white_pov(&semi_open) - eval_white_pov(&open), expected);
}

// ---- Tempo ----
//
// `TEMPO_BONUS` packs `(mg, 0)`, midgame lane only, so a bare-kings
// position (`game_phase`'s pure-endgame extreme) is exactly where to check
// it evaluates to zero rather than shrinking, and the start position
// (pure midgame, `game_phase` 0) is exactly where to check its full
// magnitude and sign together.

// The start position's own non-tempo total is independently already known
// to be exactly zero (`start_position_scores_only_the_tempo_bonus` above,
// from mirror self-symmetry), so tempo is the only thing either side of
// this delta can be: `TEMPO_BONUS` for White to move, `-TEMPO_BONUS` for
// Black. Written as
// a delta anyway, matching the issue's own framing, so this pins both
// magnitude and sign in one assertion rather than two separate ones that
// each depend on the zero-baseline fact holding.
#[test]
fn tempo_favors_whoever_is_actually_to_move() {
    let white_to_move = Board::start_pos();
    let black_to_move =
        Board::try_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1")
            .expect("valid FEN");

    let expected = 2 * weights::TEMPO_BONUS.0;
    assert_eq!(
        eval_white_pov(&white_to_move) - eval_white_pov(&black_to_move),
        expected
    );
}

// Bare kings on mirrored squares (e1/e8) put every other term at exactly
// zero (material, PST, and king safety all cancel or vanish with no pawns
// on the board), and `game_phase` reads its pure-endgame extreme (256)
// with nothing but two kings on the board, so if tempo leaked into the
// endgame lane this would be the position to catch it on: either side to
// move would score something other than zero.
#[test]
fn tempo_is_zero_in_a_pure_endgame() {
    let white_to_move = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let black_to_move = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 b - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&white_to_move), 0);
    assert_eq!(eval_white_pov(&black_to_move), 0);
}

// ---- Outposts ----
//
// `weights::OUTPOST_BONUS` is flat across both phases, so isolating it
// only needs the background (pawns, kings) held byte-for-byte identical
// between the "no minor" and "minor added" positions being compared, the
// same delta discipline the Rook files section uses. `weights::TEMPO_BONUS`
// is not flat, though, so three mirrored queen pairs pin phase to exactly
// zero in every position below (the same technique the Bishop pair and
// Tempo sections use), keeping tempo's own contribution identical on both
// sides of every delta regardless of the one extra piece of non-pawn
// material an added knight brings.
//
// White's pawn on c4 defends d5 and no enemy pawn exists anywhere, so d5
// is a real outpost per `eval::outposts::tests::outpost_bonus_applies_when_a_friendly_pawn_defends_and_no_enemy_pawn_exists`'s
// own geometry.
#[test]
fn a_knight_on_a_defended_unreachable_square_scores_the_outpost_bonus() {
    let no_knight =
        Board::try_from_fen("qq2k1q1/8/8/8/2P5/8/8/QQ2K1Q1 w - - 0 1").expect("valid FEN");
    let knight_on_outpost =
        Board::try_from_fen("qq2k1q1/8/8/3N4/2P5/8/8/QQ2K1Q1 w - - 0 1").expect("valid FEN");

    let knight_material_and_pst = weights::PIECE_VALUES[Piece::Knight.index()]
        + pst_value(Color::White, Piece::Knight, Square::D5);
    let expected = knight_material_and_pst + weights::OUTPOST_BONUS.0;
    assert_eq!(
        eval_white_pov(&knight_on_outpost) - eval_white_pov(&no_knight),
        expected
    );
}

// The asymmetric case this repo's `{Color}x{direction}` history says to
// write explicitly: the same outpost shape as the test above, but for
// Black (whose pawns defend *downward*, not just White's own case
// mirrored via `mirrored()`), and subtracted rather than added since it's
// Black's own bonus. Black's pawn on c5 defends d4, matching
// `eval::outposts::tests::outpost_bonus_applies_for_black_defended_from_the_opposite_direction`'s
// own geometry.
#[test]
fn a_black_knight_on_a_defended_unreachable_square_scores_the_outpost_bonus() {
    let no_knight =
        Board::try_from_fen("qq2k1q1/8/2p5/8/8/8/8/QQ2K1Q1 w - - 0 1").expect("valid FEN");
    let knight_on_outpost =
        Board::try_from_fen("qq2k1q1/8/2p5/3n4/8/8/8/QQ2K1Q1 w - - 0 1").expect("valid FEN");

    let knight_material_and_pst = weights::PIECE_VALUES[Piece::Knight.index()]
        + pst_value(Color::Black, Piece::Knight, Square::D4);
    let expected = -(knight_material_and_pst + weights::OUTPOST_BONUS.0);
    assert_eq!(
        eval_white_pov(&knight_on_outpost) - eval_white_pov(&no_knight),
        expected
    );
}

// ---- King-pawn tropism ----
//
// `weights::TROPISM_BONUS` packs `(0, eg)`, endgame lane only, the mirror
// image of tempo's `(mg, 0)`. A single pawn (d5) with only bare kings
// otherwise gives `game_phase` its pure-endgame extreme (256) with no
// filler needed: pawns carry zero phase weight, so `game_phase` never sees
// anything to blend away from 256 regardless of how many exist, unlike the
// tempo section above, which needed bare kings specifically because *any*
// other piece would have pulled phase off that extreme. King safety
// vanishes entirely here too (its own `eg` half is always zero, the same
// reason `SHELTER_PENALTY` is `(x, 0)`), leaving only king PST and tropism
// itself to account for.

#[test]
fn a_closer_king_scores_more_tropism_in_a_pure_endgame() {
    let near = Board::try_from_fen("7k/8/8/3P4/8/3K4/8/8 w - - 0 1").expect("valid FEN");
    let far = Board::try_from_fen("7k/8/8/3P4/8/8/8/K7 w - - 0 1").expect("valid FEN");

    let near_distance = Square::D3.distance(Square::D5);
    let far_distance = Square::A1.distance(Square::D5);
    let king_pst_delta = pst_value_eg(Color::White, Piece::King, Square::D3)
        - pst_value_eg(Color::White, Piece::King, Square::A1);
    let tropism_delta = weights::TROPISM_BONUS.1 * Score::from(far_distance - near_distance);
    assert_eq!(
        eval_white_pov(&near) - eval_white_pov(&far),
        king_pst_delta + tropism_delta
    );
}

// Same phase-pinning as the Outposts section above (three mirrored queen
// pairs, `game_phase` exactly 0): if tropism leaked into the midgame lane,
// this delta would carry an extra term the expected value below doesn't
// account for. D2 and H1 are chosen to sit at different distances from d5
// (3 and 4), so a leaked contribution wouldn't happen to cancel by
// coincidence the way equal distances could.
#[test]
fn tropism_does_not_leak_into_the_midgame_lane() {
    let near = Board::try_from_fen("1qq1k1q1/8/8/3P4/8/8/3K4/1QQ3Q1 w - - 0 1").expect("valid FEN");
    let far = Board::try_from_fen("1qq1k1q1/8/8/3P4/8/8/8/1QQ3QK w - - 0 1").expect("valid FEN");

    let expected = pst_value(Color::White, Piece::King, Square::D2)
        - pst_value(Color::White, Piece::King, Square::H1);
    assert_eq!(eval_white_pov(&near) - eval_white_pov(&far), expected);
}

// ---- Piece-square table structure ----
//
// These pin the *shape* of the tables rather than any value in them, which is
// what makes them survive a retune: every one stays true for any table anyone
// would plausibly write, so a future tuning pass changes numbers here without
// touching a single assertion.
//
// That shape is also what catches a typo. Mutation testing found 29 surviving
// mutants in `eval::pst`, the largest cluster in the crate, every one of them
// deleting a minus sign from a table literal. Concrete per-square anchors miss
// those by construction: they pin the squares someone thought to name, and a
// typo lands anywhere. Symmetry covers all 64 at once.

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

// ---- Endgame scale factors ----
//
// Every position below keeps the two kings on asymmetric squares (one
// centralized, one cornered) rather than mirroring each other: a
// self-mirror-symmetric *placement* here is also the bare-kings KK draw
// signature, which `endgame_scale::scale_factor` forces to exactly zero
// regardless of what the underlying score (including tempo) would have
// been on its own, so a mirrored pair of bare kings would pass even if
// `endgame_scale::scale` did nothing at all. Forcing a real, otherwise
// nonzero, king-PST or material asymmetry down to exactly 0 is what
// actually exercises the scale factor.

#[test]
fn bare_kings_score_exactly_zero_despite_asymmetric_king_placement() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/8 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

#[test]
fn a_lone_knight_cannot_escape_the_draw_score() {
    let white_up_a_knight =
        Board::try_from_fen("7k/8/8/8/4K3/8/8/N7 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&white_up_a_knight), 0);

    // The asymmetric case this repo's {Color}x{side} history says to write
    // explicitly: the same signature with Black holding the extra knight,
    // not just White's own case mirrored.
    let black_up_a_knight =
        Board::try_from_fen("4k3/8/8/8/8/8/8/n6K w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&black_up_a_knight), 0);
}

#[test]
fn a_lone_bishop_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/B7 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

#[test]
fn a_pair_of_knights_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/NN6 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

// a1 and c1 are the same square color (both dark): `(0+0)` and `(2+0)` are
// both even under `Square::is_light`'s file+rank parity.
#[test]
fn same_colored_bishops_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/B1B5 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

// The boundary same_colored_bishops_cannot_escape_the_draw_score sits next
// to: a1 and b1 are opposite square colors (`0` even, `1` odd), and a real
// king-and-two-opposite-coloured-bishops position is one of the four basic
// forced checkmates, not a draw. `hard_draw_scale` must leave this one
// alone rather than treating "two bishops, one side" as a single case.
#[test]
fn opposite_colored_bishops_on_one_side_are_not_scaled_to_a_draw() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/BB6 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 600); // two bishops' worth of material, roughly
}

// The other combination that can force mate despite being "only two
// minors": a knight and a bishop together, unlike either alone.
#[test]
fn a_knight_and_bishop_pair_are_not_scaled_to_a_draw() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/BN6 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 600);
}

// White's bishop on b1 (file 1 + rank 0 = odd, light) and Black's on b8
// (file 1 + rank 7 = odd... file 1 + rank 7 = 8, even, dark): opposite
// colors, one each side, the classic fortress case. White's extra d4 pawn
// would be worth a full 100 centipawns plus its own positional terms
// unscaled; `soft_draw_scale` should leave White still (barely) ahead, but
// nowhere near a full pawn's worth.
#[test]
fn opposite_colored_bishops_with_an_extra_pawn_score_well_below_a_pawn() {
    let board = Board::try_from_fen("1b2k3/8/8/8/3P4/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let score = eval_white_pov(&board);
    assert!(
        (0..50).contains(&score),
        "expected a small, positive, well-below-a-pawn score, got {score}"
    );

    // Same position, Black's own extra pawn instead of White's: the
    // {Color}x{side} check this repo's history says to write explicitly,
    // not just White's own case mirrored via `mirrored()`.
    let black_extra_pawn =
        Board::try_from_fen("1b2k3/8/8/3p4/8/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let black_score = eval_white_pov(&black_extra_pawn);
    assert!(
        (-50..0).contains(&black_score),
        "expected a small, negative, well-below-a-pawn score, got {black_score}"
    );
}

// The far end of the ramp from the single-pawn case above: under the old
// flat 1/16 constant this four-pawn material lead would have scored around
// 25 (`4 * 100 / 16`), same as a one-pawn lead scaled down to about 6. The
// pawn-count-indexed curve leaves most of a lead this size intact instead,
// since CPW's framing treats a four-pawn opposite-bishop edge as often
// winning outright, not still a near-draw.
#[test]
fn a_four_pawn_ocb_advantage_scores_far_above_a_one_pawn_advantage() {
    let board = Board::try_from_fen("1b2k3/8/8/8/3PPPP1/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let score = eval_white_pov(&board);
    assert!(
        score > 200,
        "expected the ramp to leave most of a four-pawn lead intact, got {score}"
    );

    // {Color}x{side}: Black holding the four-pawn edge instead of White,
    // not just White's own case mirrored via `mirrored()`.
    let black_board =
        Board::try_from_fen("1b2k3/3pppp1/8/8/8/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let black_score = eval_white_pov(&black_board);
    assert!(
        black_score < -200,
        "expected the ramp to leave most of a four-pawn lead intact for Black too, got {black_score}"
    );

    assert_eq!(eval_white_pov(&mirrored(&board)), -score);
}

// The boundary the position above sits next to: same shape, but both
// bishops on the same square color (b1 and a8 are both light: `1+0=1` and
// `0+7=7`, both odd), so this isn't the opposite-coloured-bishops fortress
// at all. White's extra pawn should show through close to its full value,
// not scaled down.
#[test]
fn same_colored_bishops_with_an_extra_pawn_are_not_scaled_down() {
    let board = Board::try_from_fen("b3k3/8/8/8/3P4/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 80);
}

// ---- ScaleFactor arithmetic ----
//
// The factor's own behaviour, separate from which positions produce which
// factor. The concrete endgame positions above cover the second question; these
// cover the first, so a change to the fixed-point representation fails here
// rather than showing up as an unexplained centipawn drift in a position test.

/// The identity has to be exact, including for negative scores: it is the
/// common case by far, so a rounding error here would be a constant small bias
/// on nearly every evaluation rather than a visible failure.
#[test]
fn an_unscaled_factor_leaves_every_score_untouched() {
    for score in [0, 1, -1, 7, -7, 999, -999, Score::MAX, Score::MIN] {
        assert_eq!(
            ScaleFactor::ONE.apply(score),
            score,
            "ScaleFactor::ONE must be the identity, and was not for {score}"
        );
    }
}

#[test]
fn a_draw_factor_zeroes_every_score() {
    for score in [0, 1, -1, 5000, -5000, Score::MAX, Score::MIN] {
        assert_eq!(ScaleFactor::DRAW.apply(score), 0);
    }
}

/// Truncation goes toward zero on both signs, matching the plain integer
/// division this replaced. Rounding away from zero would make a scaled-down
/// score drift *away* from the draw it is being pulled toward.
#[test]
fn scaling_truncates_toward_zero_on_both_signs() {
    let sixteenth = ScaleFactor::from_reciprocal(16);

    assert_eq!(sixteenth.apply(160), 10);
    assert_eq!(sixteenth.apply(-160), -10);
    // 100/16 is 6.25: both signs land on 6, not 7 and not -7.
    assert_eq!(sixteenth.apply(100), 6);
    assert_eq!(sixteenth.apply(-100), -6);
}

/// A zero divisor is meaningless rather than catastrophic, and saturates to a
/// draw instead of dividing by zero. Worth pinning because the queued scale
/// rules build their divisors from position features, so a zero is reachable
/// by arithmetic rather than only by someone typing it.
#[test]
fn a_zero_divisor_is_a_draw_rather_than_a_panic() {
    assert_eq!(ScaleFactor::from_reciprocal(0), ScaleFactor::DRAW);
    assert_eq!(ScaleFactor::from_reciprocal(0).apply(500), 0);
}

/// A larger divisor scales further down, monotonically. This is the property
/// every future rule depends on, since each one picks a divisor from position
/// features and expects "more unwinnable" to mean "smaller score".
#[test]
fn a_larger_divisor_never_scales_less() {
    let score = 3200;
    let mut previous = ScaleFactor::ONE.apply(score);
    for divisor in [1, 2, 4, 8, 16, 32, 64] {
        let scaled = ScaleFactor::from_reciprocal(divisor).apply(score);
        assert!(
            scaled <= previous,
            "divisor {divisor} scaled {score} to {scaled}, above the previous {previous}"
        );
        previous = scaled;
    }
}

/// `from_numerator` is the general-curve counterpart to `from_reciprocal`:
/// below `UNIT` it produces exactly the fraction asked for, matching a
/// `from_reciprocal` call with the same effective ratio.
#[test]
fn from_numerator_below_unit_matches_the_equivalent_reciprocal() {
    assert_eq!(
        ScaleFactor::from_numerator(16),
        ScaleFactor::from_reciprocal(16)
    );
    assert_eq!(ScaleFactor::from_numerator(128).apply(256), 128);
}

/// A numerator at or past `UNIT` clamps to `ScaleFactor::ONE` rather than
/// amplifying the score: nothing in this module ever scales a score up, so a
/// curve that overshoots means "unscaled", not "boosted".
#[test]
fn from_numerator_at_or_above_unit_clamps_to_one() {
    assert_eq!(ScaleFactor::from_numerator(256), ScaleFactor::ONE);
    assert_eq!(ScaleFactor::from_numerator(1000), ScaleFactor::ONE);
}

/// The pawn-count-indexed opposite-coloured-bishops ramp, checked at the
/// `scale_factor` level rather than through a private helper: a pawn
/// difference of 0 or 1 reproduces the original flat 1/16, a difference of
/// 5 or more reaches fully unscaled, and every step in between is at least
/// as generous as the last. This is the property every future curve tweak
/// depends on, since a smaller pawn edge should never end up scaled *up*
/// relative to a larger one.
#[test]
fn a_larger_ocb_pawn_advantage_never_scales_less() {
    let pawn_ranks = ["8", "3P4", "3PP3", "3PPP2", "3PPPP1", "3PPPPP", "2PPPPPP"];
    let boards: Vec<Board> = pawn_ranks
        .iter()
        .map(|pawns| {
            let fen = format!("1b2k3/8/8/{pawns}/8/8/8/1B2K3 w - - 0 1");
            Board::try_from_fen(&fen).expect("valid FEN")
        })
        .collect();

    let sixteenth = ScaleFactor::from_reciprocal(16);
    assert_eq!(
        scale_factor(&boards[0]),
        sixteenth,
        "a 0-pawn edge should match the old flat constant"
    );
    assert_eq!(
        scale_factor(&boards[1]),
        sixteenth,
        "a 1-pawn edge should match the old flat constant"
    );
    assert_eq!(
        scale_factor(&boards[5]),
        ScaleFactor::ONE,
        "a 5-pawn edge should be fully unscaled"
    );
    assert_eq!(
        scale_factor(&boards[6]),
        ScaleFactor::ONE,
        "a 6-pawn edge should stay fully unscaled"
    );

    let mut previous = ScaleFactor::DRAW;
    for (pawns, board) in pawn_ranks.iter().zip(&boards) {
        let factor = scale_factor(board);
        assert!(
            factor >= previous,
            "pawn rank {pawns:?} scaled to {factor:?}, below the previous {previous:?}"
        );
        previous = factor;
    }
}
