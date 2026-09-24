//! Move search.
//!
//! [`Search`] runs negamax with fail-soft alpha-beta over iterative deepening, quiescence
//! search at the horizon, and MVV-LVA move ordering, driven by a depth, node, or
//! wall-clock time budget, honoring an external stop request via [`Search::stop_flag`].
//! `time::allocate_time` turns a real game clock into the deadline
//! `Search::with_deadline` wants. Built on `move_gen` for legal moves, `eval` for the
//! position score to search toward, `draw` for the fifty-move/repetition checks a search
//! node makes before recursing further, and `board::zobrist` for the hashes `draw`'s
//! repetition check and the transposition table (`tt`) both key on.

pub mod cutoff_history;
pub mod draw;
mod killers;
mod negamax;
pub mod time;
pub mod tt;

/// Ply bound for every per-ply side table a search keeps: the killers, the
/// mate killers, the hash moves and the previous iteration's principal
/// variation. A ply past it reads and writes the last slot rather than
/// panicking, which quiescence's uncapped in-check evasion recursion can
/// reach.
///
/// Here rather than beside any one of those tables, because it is the bound
/// they share; moving it next to one would make the others import their size
/// from an unrelated concept.
const MAX_TRACKED_PLY: usize = 512;

pub use negamax::{
    is_mate_score, CutoffCause, CutoffStats, Search, SearchResult, MATE, MAX_MATE_PLY,
    MAX_QUIESCENCE_DEPTH,
};
