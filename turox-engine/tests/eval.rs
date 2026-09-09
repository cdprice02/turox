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
