//! Concrete positions for `eval::phase`: king placement by game phase, and the
//! blend at its extremes.

use turox_chess::board::Board;
use turox_chess::{Color, Square};
use turox_engine::eval::pst::pst_value;
use turox_engine::eval::{eval_white_pov, weights, Score};

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
// with the property tests in `tests/eval.rs` (which exercise this
// only as often as `any_board()` happens to roll enough extra material)
// rather than relying on randomness alone to hit this exact shape.
#[test]
fn heavily_overloaded_material_does_not_panic_or_invert_the_score() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/QQQQQQQK w - - 0 1").expect("valid FEN");

    assert!(eval_white_pov(&board) > 0);
}
