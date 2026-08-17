//! Move search: iterative deepening over alpha-beta (or a successor), driven by a
//! time or depth budget and a transposition table.
//!
//! Not yet implemented — waits on `move_gen` for legal moves to search and `eval`
//! for a position score to search toward.

pub mod tt;
