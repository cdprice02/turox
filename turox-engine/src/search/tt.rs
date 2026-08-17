//! Transposition table: a fixed-size hash table of previously-searched positions,
//! keyed by the Zobrist hash from `board::zobrist`, storing a score, best move, and
//! search depth so transposing move orders don't get re-searched from scratch.
//!
//! Not yet implemented.
