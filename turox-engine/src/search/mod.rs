//! Move search: [`Search`] runs negamax with fail-soft alpha-beta over
//! iterative deepening, quiescence search at the horizon, and MVV-LVA move
//! ordering, driven by a depth or node budget (a wall-clock time budget is a
//! later addition, layered on the same abort mechanism). Built on
//! `move_gen` for legal moves, `eval` for the position score to search
//! toward, `draw` for the fifty-move/repetition checks a search node makes
//! before recursing further, and `board::zobrist` for the hashes `draw`'s
//! repetition check and the future transposition table (`tt`) both key on.

pub mod draw;
mod negamax;
pub mod tt;

pub use negamax::{Search, SearchResult, MATE, MAX_QUIESCENCE_DEPTH};
