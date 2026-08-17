//! Pin- and check-aware legal move generation: the piece that ties `tables`,
//! `magic`, and `Board` together into an actual list of legal moves for a
//! position.
//!
//! Not yet implemented. Depends on `tables` (leaper attacks, `between`/`line` for
//! pin detection) and `magic` (slider attacks) both landing first, plus
//! `Board::make_move` for perft to validate against.
//!
//! Intended entry point: `fn legal_moves(board: &Board) -> MoveList`, generating
//! into the stack-allocated buffer from `move_list` rather than a `Vec`.
