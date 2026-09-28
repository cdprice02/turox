//! Board and move fixtures shared by more than one module's tests.
//!
//! Here rather than duplicated because a fixture that drifts between two
//! copies leaves both modules' tests passing while they assert about
//! different positions.

use turox_chess::board::Board;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::types::{Move, Square};

/// A white queen with both a capture and quiet moves available, and no two
/// captures of equal value, so a test can name a tier without ambiguity.
pub(super) fn capture_and_quiet_position() -> Board {
    Board::try_from_fen("4k3/8/8/3n4/4Q3/8/8/4K3 w - - 0 1").expect("valid FEN")
}

/// The legal move from `from` to `to` in `board`, panicking if there is
/// none: a test naming a move that is not legal in its own fixture is a
/// broken test, not a case to handle.
pub(super) fn find_move(board: &Board, from: Square, to: Square) -> Move {
    *legal_moves(board)
        .as_slice()
        .iter()
        .find(|m| m.from() == from && m.to() == to)
        .unwrap_or_else(|| panic!("{from:?}{to:?} must be a legal move in this position"))
}
