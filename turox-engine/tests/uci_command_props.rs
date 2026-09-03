//! Property test for `uci::command::parse`.
//!
//! No round-trip property test against an emitted form here: that needs
//! `Display`/emission (a later, sibling issue) to exist first. The one
//! property test below instead round-trips a `position fen ... moves ...`
//! line built directly from `Board::to_fen`/`Move::to_uci` (both already
//! real) against `parse`, which is meaningful on its own and doesn't need
//! to wait. Concrete tests (one per command, plus malformed input) live in
//! `tests/uci_command.rs`, not here: this file is proptest only.

mod common;

use common::any_board_with_legal_move;
use proptest::prelude::*;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::uci::{parse, Command};

proptest! {
    /// Builds a `position fen <fen> moves <uci>` line directly from
    /// `Board::to_fen` and `Move::to_uci` (no dependency on `parse`'s own
    /// output format) and checks `parse` recovers exactly the board that
    /// applying that one move through the trusted `Board::make_move`
    /// directly produces.
    #[test]
    fn position_fen_moves_round_trips_to_the_final_board(board in any_board_with_legal_move()) {
        let moves = legal_moves(&board);
        let m = moves.as_slice()[0];
        let expected = board.make_move(m);

        let line = format!("position fen {} moves {}", board.to_fen(), m.to_uci());
        prop_assert_eq!(parse(&line), Some(Command::Position(expected)));
    }
}
