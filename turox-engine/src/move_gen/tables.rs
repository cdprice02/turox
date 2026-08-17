//! Precomputed attack tables for the non-sliding pieces (knight, king, pawn) and
//! the ray-tracing helpers (`between`, `line`) legal move generation needs for
//! pin and check detection.
//!
//! Not yet implemented. Everything here is meant to be a `const` table, built at
//! compile time from `Bitboard`'s `shift`/transform primitives (`types::bitboard`)
//! — which is why this waits on that exercise being done first: these tables are
//! the first real consumer of `Bitboard::shift` and friends outside their own
//! tests.
//!
//! Intended public surface:
//! - `const KNIGHT_ATTACKS: [Bitboard; 64]`, `const KING_ATTACKS: [Bitboard; 64]`
//! - `const PAWN_ATTACKS: [[Bitboard; 64]; 2]` (indexed by `Color`)
//! - `fn between(a: Square, b: Square) -> Bitboard` — squares strictly between `a`
//!   and `b` on a shared rank/file/diagonal, or `Bitboard::EMPTY` if they don't
//!   share one. Used for "is a piece pinned against the king" checks.
//! - `fn line(a: Square, b: Square) -> Bitboard` — the full rank/file/diagonal
//!   through both `a` and `b`, or `Bitboard::EMPTY` if they don't share one.
