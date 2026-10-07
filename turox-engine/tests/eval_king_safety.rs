//! Concrete positions for `eval::king_safety`, each isolating one term as an
//! exact delta.
//!
//! Every `eval::king_safety` term packs `(mg, 0)`: an `eg` of zero. That
//! makes every position below need *some* non-pawn material on the board,
//! unlike `tests/eval_pawn_structure.rs`, which got away with bare kings
//! and pawns: at `game_phase`'s pure-endgame extreme (256, no non-pawn
//! material at all), `interpolate` returns the `eg` half with no blending,
//! which for a king-safety term is always exactly 0. Testing the term's
//! real effect needs a phase away from that extreme.
//!
//! Every FEN below carries the same fixed filler army on ranks 4 and 5 (a
//! White queen, two rooks, two bishops, two knights on rank 4; the Black
//! mirror on rank 5) for exactly that reason: 4 + 2 + 2 + 1 + 1 + 1 + 1 =
//! 12 per side, 24 combined, which is `TOTAL_PHASE` exactly, so
//! `game_phase` reads 0 (pure midgame) and `interpolate` reduces to the
//! `mg` half with no rounding at all. The filler never moves between any
//! two positions being compared and never shares a file or rank with
//! anything under test, so its own material and PST contributions are
//! identical on both sides of every delta below and cancel out; only its
//! existence (to hold the phase at 0) matters. Kings and king-adjacent
//! pawns stay on ranks 1/2 and 7/8, well clear of it.

use turox_chess::board::Board;
use turox_chess::{Color, Piece, Square};
use turox_engine::eval::eval_white_pov;
use turox_engine::eval::pst::pst_value;

// Fixes White's pawns at c2/d2/e2 (a `d`-file king's full shelter) in both
// positions and only moves the king itself, from d1 (in the shield) to g1 (in
// open air on the kingside), so material, every pawn's own PST, and the
// pawn-structure term are all identical in both positions and cancel out of the
// delta entirely: c2/d2/e2 are isolated from each other's isolated-pawn status
// and each other's passed status exactly the same way regardless of where the
// king stands, since neither depends on the king's square. Only two things
// differ: the king's own PST value at its two squares (computed here via
// `pst_value` rather than transcribed, the same discipline
// `full_phase_material_total_matches_pure_midgame_sum` in `tests/eval_phase.rs`
// uses), and king safety itself.
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
