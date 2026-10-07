//! Concrete positions for `eval::bishop_pair`, each isolating the pair bonus as
//! an exact delta.
//!
//! `only_the_king_has_a_distinct_endgame_table` (in `tests/eval_pst.rs`) pins
//! that bishop PST has no separate endgame half, and
//! `weights::BISHOP_PAIR_BONUS` is flat across both phases too, so neither
//! depends on what `game_phase` reads. `weights::TEMPO_BONUS` is not flat,
//! though (`(mg, 0)`, midgame lane only, see `tests/eval_tempo.rs`), and every
//! position here has White to move with nothing on the Black side to cancel
//! that term against, so unlike the bishop-pair math, tempo's own contribution
//! *does* depend on phase, and needs it pinned to a known value to stay exact.
//!
//! Three queens per side (mirrored, so their own material/PST cancel the
//! same way the kings' do) lands combined non-pawn material at exactly
//! `TOTAL_PHASE`, pinning `game_phase` to 0 and `weights::TEMPO_BONUS.0`
//! (not some phase-blended fraction of it) as the exact tempo contribution
//! in every position below. Queens also clear `scale_factor`'s very first
//! check on their own: a bare king facing one or two
//! *same-coloured* bishops is insufficient mating material, and
//! `eval::endgame_scale` would otherwise scale that straight to zero
//! regardless of what material/PST/pair/tempo terms summed to underneath,
//! swallowing the exact deltas these tests want to isolate. The two-bishop
//! positions use c1/f1, the actual starting squares, so the pair is
//! opposite-coloured and would still score unscaled even without the
//! queens. Kings sit on their own mirrored squares (e1/e8) in every
//! position so their own PST and king-safety contributions cancel out too,
//! leaving only material, bishop PST, the pair bonus, and tempo.

use turox_chess::board::Board;
use turox_chess::{Color, Piece, Square};
use turox_engine::eval::pst::pst_value;
use turox_engine::eval::{eval_white_pov, weights};

// A concrete anchor for the mirror-cancellation setup itself, before
// layering any bishops on: if this ever fails, the queen/king placement
// below isn't cancelling the way the rest of this file assumes, which
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
// this file's every position on the same footing rather than leaving
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
// (`tests/eval.rs`) for every mirror-symmetric board, and this pins
// one concrete instance of that, the same way
// `start_position_scores_only_the_tempo_bonus` pins the general property
// for the start position. Left with only the tempo bonus rather than zero,
// for the same reason that test is: White to move, nothing to cancel it.
#[test]
fn bishop_pairs_on_both_sides_cancel_to_exactly_the_tempo_bonus() {
    let board = Board::try_from_fen("qqb1kbq1/8/8/8/8/8/8/QQB1KBQ1 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), weights::TEMPO_BONUS.0);
}
