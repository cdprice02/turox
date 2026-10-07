//! Concrete positions for `eval::rook_files`, each isolating the file bonus as
//! an exact delta.
//!
//! Only the rook under test moves between compared positions, so pawn
//! structure, king safety and phase cancel, and rook PST has no endgame half,
//! so no phase arithmetic is needed. The fixed pawns (White a2, Black g7) give
//! each tested file a different state: h open, g semi-open for White, a closed
//! for White.

use turox_chess::board::Board;
use turox_chess::{Color, Piece, Square};
use turox_engine::eval::pst::pst_value;
use turox_engine::eval::{eval_white_pov, weights};

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
