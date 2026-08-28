//! Move search: iterative deepening over alpha-beta (or a successor), driven by a
//! time or depth budget and a transposition table.
//!
//! The search loop itself isn't implemented yet, but its prerequisites are:
//! `move_gen` for legal moves to search, `eval` for a position score to
//! search toward, and `draw` for the fifty-move/repetition checks a search
//! node makes before recursing further.

pub mod draw;
pub mod tt;
