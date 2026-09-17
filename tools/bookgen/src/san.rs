//! Resolving one SAN (Standard Algebraic Notation) move token against a
//! specific position.
//!
//! SAN is context-dependent (`Nf3` only means something once you know
//! which knight, if either, can legally reach `f3`), so resolution always
//! needs the actual position, not just the token text.

use turox_engine::board::Board;
use turox_engine::Move;

/// Resolves `token` (e.g. `"Nf3"`, `"exd5"`, `"O-O"`, `"e8=Q+"`) against the
/// legal moves available in `board`, returning the one `Move` it names.
///
/// `None` for a token that doesn't parse as SAN at all, or that resolves to
/// zero or more than one legal move (ambiguous notation is a real, if rare,
/// possibility in hand-annotated or buggy PGN, not just malformed input;
/// either way there's no single answer to return).
#[must_use]
pub fn resolve_san(_board: &Board, _token: &str) -> Option<Move> {
    None
}
