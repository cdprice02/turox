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
use turox_engine::eval::{eval_white_pov, evaluate, Score};
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
//
// A lone pawn with no other pawns on the board is isolated (-10/-10) and
// passed (+10/+20, no enemy pawn anywhere to block it), a net (0, +10)
// pawn-structure contribution in both positions; with only two kings and
// one pawn on the board, `game_phase` reads a pure endgame (256), so
// `eval_white_pov` reduces to the eg total alone: 100 material + PST +
// 10 net pawn-structure eg.
#[test]
fn a_central_pawn_push_changes_pst_but_not_material() {
    let before = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&before), 90); // 100 material + (0 + -20) PST + 10 pawn structure (eg)

    let after = Board::try_from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&after), 130); // 100 material + (0 + 20) PST + 10 pawn structure (eg)
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
#[test]
fn full_phase_material_total_matches_pure_midgame_sum() {
    const MATERIAL: [Score; 6] = [100, 320, 330, 500, 900, 0];

    let board = Board::try_from_fen("bbrrqk2/8/8/8/8/8/QK6/NNNNBBRR w - - 0 1").expect("valid FEN");
    let mut expected: Score = 0;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            let value = MATERIAL[cp.piece().index()] + pst_value(cp.color(), cp.piece(), sq);
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
// positions being compared, so the kings' own (nonzero) piece-square
// contribution cancels out of the difference and doesn't need to be
// computed by hand at all; only the pawns being added or moved matter.
//
// White's pawn PST (endgame == midgame for pawns; only the king has a
// separate endgame table) by rank on the d-file: d2 -20, d4 +20, d5 +25,
// d6 +30 (the same numbers `a_central_pawn_push_changes_pst_but_not_material`
// and the orientation anchors above already pin down). Black's d7 is -20
// (own mirrored anchor above); Black's d4 is +25 and e3 (White) is 0,
// read the same way, by working through `pst::pst_value`'s doc.

// One doubled pawn (d2, d4) against a d2-only baseline. Adding d4 changes
// four independent terms, all attributable to the new pawn alone: its own
// material and PST (100 + 20), its own isolated penalty (-10, since c/e
// stay empty in both positions), its own passed bonus (+20, since neither
// position has any Black pawn anywhere), and one doubled penalty newly
// appearing on the d-file now that it holds two pawns instead of one
// (-20). d2's own isolated/passed status is unchanged by d4 arriving,
// since isolation only looks at adjacent files and passed status only
// looks at enemy pawns, neither of which d4 is. 100 + 20 - 10 + 20 - 20 = 110.
#[test]
fn one_doubled_pawn_scores_material_plus_pst_minus_one_doubled_penalty() {
    let one_pawn = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    let two_pawns = Board::try_from_fen("4k3/8/8/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(eval_white_pov(&two_pawns) - eval_white_pov(&one_pawn), 110);
}

// The case that catches counting doubled pawns as "N per file" instead of
// "N - 1", or double-charging the penalty multiplicatively: a third pawn
// on the same file (d2, d4, d6) must add exactly one more doubled penalty
// on top of the two-pawn case above, not a second one and not a
// proportionally larger one. d6's own contribution: material + PST
// (100 + 30), its own isolated penalty (-10) and passed bonus (+20), plus
// the file's doubled count moving from one penalty (two pawns) to two
// (three pawns), i.e. one more -20. 100 + 30 - 10 + 20 - 20 = 120.
#[test]
fn a_third_doubled_pawn_adds_exactly_one_more_doubled_penalty() {
    let two_pawns = Board::try_from_fen("4k3/8/8/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    let three_pawns =
        Board::try_from_fen("4k3/8/3P4/8/3P4/8/3P4/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(
        eval_white_pov(&three_pawns) - eval_white_pov(&two_pawns),
        120
    );

    // And the three-pawn position as a whole scores two doubled penalties'
    // worth below a single-pawn baseline, not one: 110 + 120 = 230.
    let one_pawn = Board::try_from_fen("4k3/8/8/8/8/8/3P4/4K3 w - - 0 1").expect("valid FEN");
    assert_eq!(
        eval_white_pov(&three_pawns) - eval_white_pov(&one_pawn),
        230
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
// contributes c4's own material and PST (100 + 0, c4's PST entry is 0);
// c4 isn't isolated either, since d4 is right next to it. 100 + 0 + 10 = 110.
#[test]
fn adding_an_adjacent_pawn_removes_the_isolated_penalty() {
    let isolated = Board::try_from_fen("4k3/8/8/3p4/3P4/8/8/4K3 w - - 0 1").expect("valid FEN");
    let supported = Board::try_from_fen("4k3/8/8/3p4/2PP4/8/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(eval_white_pov(&supported) - eval_white_pov(&isolated), 110);
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
// 20 + 70 = 90.
#[test]
fn a_lone_passed_pawn_loses_its_bonus_once_blocked_on_its_own_file() {
    let clear_path = Board::try_from_fen("4k3/8/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked = Board::try_from_fen("4k3/3p4/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(eval_white_pov(&clear_path) - eval_white_pov(&blocked), 90);
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
// 20 + 90 = 110.
#[test]
fn an_adjacent_file_blocker_also_disqualifies_a_passed_pawn() {
    let clear_path = Board::try_from_fen("4k3/8/8/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked_on_adjacent_file =
        Board::try_from_fen("4k3/8/4p3/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(
        eval_white_pov(&clear_path) - eval_white_pov(&blocked_on_adjacent_file),
        110
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
// 20 since it's normally subtracted) adds another 20. 90 + 20 = 110.
#[test]
fn black_passed_pawn_direction_mirrors_white_not_the_other_way_around() {
    let clear_path = Board::try_from_fen("4k3/8/8/8/3p4/8/8/4K3 w - - 0 1").expect("valid FEN");
    let blocked = Board::try_from_fen("4k3/8/8/8/3p4/4P3/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(eval_white_pov(&blocked) - eval_white_pov(&clear_path), 110);
}
