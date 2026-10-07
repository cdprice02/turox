//! Concrete positions for `eval::rook_files`, each isolating the file bonus as
//! an exact delta.
//!
//! Rook PST has no separate endgame half either (same
//! `only_the_king_has_a_distinct_endgame_table` guarantee
//! `tests/eval_bishop_pair.rs` relies on), so every delta below only needs to
//! account for material, PST, and the file bonus: no phase arithmetic required.
//! Every position in this file keeps its pawns and king squares byte-for-byte
//! identical across the positions it's compared against, moving only the rook
//! under test, so pawn-structure and king-safety (and, since piece counts never
//! change, the phase itself) are identical on both sides of every delta and
//! cancel out regardless of what they individually equal.
//!
//! The fixed background is one White pawn on a2 and one Black pawn on g7:
//! together they give every file under test a different state depending on
//! which file the tested rook sits on. h-file: no pawn of either colour
//! (open). g-file: only Black's pawn (semi-open for White, since White has
//! no pawn there). a-file: White's own pawn (closed for White, regardless
//! of what's on it for Black).

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
