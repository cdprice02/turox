//! Property test for `Move::to_uci`/`Move::from_uci`.
//!
//! Concrete tests (castling, promotion, en passant, malformed input) live
//! in `tests/move_uci.rs`, not here: this file is proptest only.

mod common;

use common::any_board_with_legal_move;
use proptest::prelude::*;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::Move;

proptest! {
    /// The load-bearing check: for every legal move in every generated
    /// position, `to_uci` then `from_uci` (resolved against that same
    /// position's legal moves) recovers the exact original `Move`, flags
    /// included. This is what actually proves `from_uci`'s legal-move
    /// matching disambiguates correctly, not just that the two functions
    /// don't panic.
    #[test]
    fn to_uci_then_from_uci_recovers_the_original_move(board in any_board_with_legal_move()) {
        let moves = legal_moves(&board);
        for &m in moves.as_slice() {
            let uci = m.to_uci();
            let recovered = Move::from_uci(&uci, moves.as_slice());
            prop_assert_eq!(recovered, Some(m), "uci string was {:?}", uci);
        }
    }
}
