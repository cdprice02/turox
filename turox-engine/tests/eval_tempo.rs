//! Concrete positions for `eval::tempo`: its full magnitude and sign, and its
//! absence in a pure endgame.
//!
//! `TEMPO_BONUS` packs `(mg, 0)`, midgame lane only, so a bare-kings
//! position (`game_phase`'s pure-endgame extreme) is exactly where to check
//! it evaluates to zero rather than shrinking, and the start position
//! (pure midgame, `game_phase` 0) is exactly where to check its full
//! magnitude and sign together.

use turox_chess::board::Board;
use turox_engine::eval::{eval_white_pov, weights};

// The start position's own non-tempo total is independently already known to be
// exactly zero (`start_position_scores_only_the_tempo_bonus`, from mirror
// self-symmetry), so tempo is the only thing either side of this delta can be:
// `TEMPO_BONUS` for White to move, `-TEMPO_BONUS` for Black. Written as a delta
// anyway, so this pins both magnitude and sign in one assertion rather than two
// separate ones that each depend on the zero-baseline fact holding.
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
