//! Concrete scenario tests for `Move::to_uci`/`Move::from_uci`.
//!
//! `tests/move_uci_props.rs` has the load-bearing round-trip property (every
//! legal move, in every generated position); `any_board()` never generates
//! an en passant state though (see its own doc), so the concrete en passant
//! test here covers that flag directly, the same split `pseudo_legal.rs`
//! uses for en passant generation itself. The castling and promotion tests
//! here also pin exact UCI strings down, which a property alone wouldn't
//! catch if `to_uci`/`from_uci` agreed with each other but both disagreed
//! with the UCI spec.

use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::types::MoveFlags;
use turox_engine::{Move, Square};

// ---- Concrete castling: all four corners ----
//
// All four corners get checked explicitly rather than trusting symmetry.
// Confirmed via `legal_moves` directly (not assumed) that this FEN produces exactly
// `Ra1-c1`/`Rh1-g1`-shaped castles for whichever color is to move, spelled
// by the king's own destination per UCI (`e1g1`, not `e1h1`).

const OPEN_CASTLE_POSITION_WHITE: &str = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
const OPEN_CASTLE_POSITION_BLACK: &str = "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1";

#[allow(
    clippy::panic,
    reason = "test-only helper, not itself a #[test] fn, so clippy's test-context detection doesn't cover it; the interpolated fen/flags are worth keeping over a plain .expect(...)"
)]
fn castle_move(fen: &str, flags: MoveFlags) -> Move {
    let board = Board::try_from_fen(fen).expect("valid FEN");
    *legal_moves(&board)
        .as_slice()
        .iter()
        .find(|m| m.flags() == flags)
        .unwrap_or_else(|| panic!("no {flags:?} move in {fen:?}"))
}

#[test]
fn white_kingside_castle_is_e1g1() {
    let m = castle_move(OPEN_CASTLE_POSITION_WHITE, MoveFlags::KingCastle);
    assert_eq!(m.to_uci(), "e1g1");
}

#[test]
fn white_queenside_castle_is_e1c1() {
    let m = castle_move(OPEN_CASTLE_POSITION_WHITE, MoveFlags::QueenCastle);
    assert_eq!(m.to_uci(), "e1c1");
}

#[test]
fn black_kingside_castle_is_e8g8() {
    let m = castle_move(OPEN_CASTLE_POSITION_BLACK, MoveFlags::KingCastle);
    assert_eq!(m.to_uci(), "e8g8");
}

#[test]
fn black_queenside_castle_is_e8c8() {
    let m = castle_move(OPEN_CASTLE_POSITION_BLACK, MoveFlags::QueenCastle);
    assert_eq!(m.to_uci(), "e8c8");
}

// ---- Concrete promotion: all four pieces ----
//
// All four promotion moves here share the exact same `from`/`to`
// (`a7a8`), differing *only* in which piece they promote to (confirmed via
// `legal_moves` directly): the one case where `from_uci` has to actually
// use the parsed promotion letter to disambiguate, not just match on
// `from`/`to` alone.

const PROMOTION_POSITION: &str = "8/P7/8/8/8/8/8/4k2K w - - 0 1";

#[test]
fn all_four_promotion_suffixes_round_trip() {
    let board = Board::try_from_fen(PROMOTION_POSITION).expect("valid FEN");
    let moves = legal_moves(&board);
    let promotions: Vec<Move> = moves
        .as_slice()
        .iter()
        .copied()
        .filter(|m| m.from() == Square::A7)
        .collect();
    assert_eq!(promotions.len(), 4, "expected all four promotion pieces");

    let expected_suffixes = ["a7a8q", "a7a8r", "a7a8b", "a7a8n"];
    for &suffix in &expected_suffixes {
        let recovered = Move::from_uci(suffix, moves.as_slice())
            .unwrap_or_else(|| panic!("{suffix} must be legal here"));
        assert_eq!(recovered.to_uci(), suffix);
        assert!(promotions.contains(&recovered));
    }
}

/// `all_four_promotion_suffixes_round_trip` only covers quiet promotion
/// (`PromoteQueen` and friends); this covers the other four `MoveFlags`
/// variants, promotion-with-capture, on a position with *both* families
/// available from the same square (confirmed via `legal_moves`) so a bug
/// that only handled one or conflated the two would show up here.
const CAPTURE_PROMOTION_POSITION: &str = "1n5k/P7/8/8/8/8/8/7K w - - 0 1";

#[test]
fn capture_promotion_suffixes_round_trip() {
    let board = Board::try_from_fen(CAPTURE_PROMOTION_POSITION).expect("valid FEN");
    let moves = legal_moves(&board);
    let from_a7: Vec<Move> = moves
        .as_slice()
        .iter()
        .copied()
        .filter(|m| m.from() == Square::A7)
        .collect();
    assert_eq!(
        from_a7.len(),
        8,
        "expected 4 quiet promotions (a7a8) plus 4 capture promotions (a7b8)"
    );

    for &suffix in &["a7b8q", "a7b8r", "a7b8b", "a7b8n"] {
        let recovered = Move::from_uci(suffix, moves.as_slice())
            .unwrap_or_else(|| panic!("{suffix} must be legal here"));
        assert_eq!(recovered.to_uci(), suffix);
        assert!(recovered.flags().is_capture());
        assert!(from_a7.contains(&recovered));
    }
}

// ---- Concrete en passant ----

const EN_PASSANT_POSITION: &str = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1";

#[test]
fn en_passant_capture_round_trips() {
    let board = Board::try_from_fen(EN_PASSANT_POSITION).expect("valid FEN");
    let moves = legal_moves(&board);
    let ep = *moves
        .as_slice()
        .iter()
        .find(|m| m.flags() == MoveFlags::EnPassant)
        .expect("en passant capture must be legal here");

    assert_eq!(ep.to_uci(), "e5d6");
    assert_eq!(Move::from_uci("e5d6", moves.as_slice()), Some(ep));
}

// ---- Malformed input never panics ----

#[test]
fn from_uci_rejects_garbage_without_panicking() {
    let board = Board::start_pos();
    let moves = legal_moves(&board);
    for bad in [
        "", "e2", "e2e", "z9z9",
        "e2e4q", // pawn double push isn't a promotion; well-formed but not legal
        "e2e5",  // not a legal knight-jump for a pawn
        "e7e8x", // 'x' isn't a real promotion letter
        "\0\0\0\0",
        "€4", // '€' is 3 bytes in UTF-8, so this is 4 bytes total (passing
              // a byte-length check meant to require 4-5 *characters*) but
              // only 2 chars; a byte-index slice landing at index 2 falls
              // inside the multi-byte character rather than on a boundary.
    ] {
        assert_eq!(
            Move::from_uci(bad, moves.as_slice()),
            None,
            "expected None for {bad:?}"
        );
    }
}

#[test]
fn from_uci_rejects_a_well_formed_but_illegal_move() {
    // e2e4 is well-formed UCI and a legal move from startpos, but not from
    // this position (kings only): must be rejected because it's absent
    // from `legal`, not because the string itself is malformed.
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert_eq!(Move::from_uci("e2e4", moves.as_slice()), None);
}
