//! Property tests for `board::zobrist`: does the incrementally-maintained
//! `Board::hash()` agree with `zobrist::compute_hash`'s from-scratch fold.
//!
//! `tests/zobrist.rs` has the concrete `{Color}x{state}` tests (side to
//! move, each castling right alone, en passant) and the perft-grade
//! ground-truth tree walk; this file is proptest only.
//!
//! `any_board()` builds positions through `place`/`from_parts` only, never
//! through `make_move`, so the plain property tests below (matching the
//! naive reference, surviving a FEN round-trip) exercise the non-incremental
//! construction paths, not `make_move`'s own hash-maintenance.
//! `hash_stays_correct_after_a_legal_move` is the one test in this file
//! that actually calls `make_move`, deliberately not `#[ignore]`d so the
//! default `cargo nextest run --workspace` gate covers it directly rather
//! than relying only on the expensive release-only perft-tree walk in
//! `tests/zobrist.rs`.

mod common;

use common::{any_board, any_board_and_legal_move};
use proptest::prelude::*;
use turox_engine::board::zobrist::compute_hash;
use turox_engine::board::Board;

proptest! {
    #[test]
    fn hash_matches_compute_hash_for_any_board(board in any_board()) {
        prop_assert_eq!(board.hash(), compute_hash(&board));
    }

    #[test]
    fn hash_survives_fen_round_trip(board in any_board()) {
        let parsed = Board::try_from_fen(&board.to_fen()).expect("to_fen output must parse");
        prop_assert_eq!(board.hash(), parsed.hash());
    }

    // Expected to fail until board::zobrist's documented make_move gap
    // (side to move, castling rights, en passant) is closed: this is the
    // first point in the file that actually calls `make_move`, the same
    // role `tests/legal_props.rs`'s own
    // `any_board_and_legal_move`-based test plays for move generation.
    #[test]
    fn hash_stays_correct_after_a_legal_move((board, m) in any_board_and_legal_move()) {
        let next = board.make_move(m);
        prop_assert_eq!(next.hash(), compute_hash(&next));
    }
}
