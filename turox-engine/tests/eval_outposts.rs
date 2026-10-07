//! Concrete positions for `eval::outposts`'s outpost bonus and king-pawn
//! tropism, each isolating one term as an exact delta.

use turox_chess::board::Board;
use turox_chess::{Color, Piece, Square};
use turox_engine::eval::pst::{pst_value, pst_value_eg};
use turox_engine::eval::{eval_white_pov, weights, Score};

// `weights::OUTPOST_BONUS` is flat across both phases, so isolating it only
// needs the background (pawns, kings) held byte-for-byte identical between the
// "no minor" and "minor added" positions being compared. `weights::TEMPO_BONUS`
// is not flat, though, so three mirrored queen pairs pin phase to exactly zero
// in every position below, keeping tempo's own contribution identical on both
// sides of every delta regardless of the one extra piece of non-pawn material
// an added knight brings.
//
// White's pawn on c4 defends d5 and no enemy pawn exists anywhere, so d5 is a
// real outpost per
// `eval::outposts::tests::outpost_bonus_applies_when_a_friendly_pawn_defends_and_no_enemy_pawn_exists`'s
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

// The asymmetric case this repo's `{Color}x{direction}` history says to write
// explicitly: the same outpost shape as the test above, but for Black (whose
// pawns defend *downward*, not just White's own case mirrored via
// `mirrored()`), and subtracted rather than added since it's Black's own bonus.
// Black's pawn on c5 defends d4, matching
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

// `weights::TROPISM_BONUS` packs `(0, eg)`, endgame lane only, the mirror image
// of tempo's `(mg, 0)`. A single pawn (d5) with only bare kings otherwise gives
// `game_phase` its pure-endgame extreme (256) with no filler needed: pawns
// carry zero phase weight, so `game_phase` never sees anything to blend away
// from 256 regardless of how many exist. King safety vanishes entirely here too (its own `eg`
// half is always zero, the same reason `SHELTER_PENALTY` is `(x, 0)`), leaving
// only king PST and tropism itself to account for.
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

// Same phase-pinning as the outpost positions (three mirrored queen
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
