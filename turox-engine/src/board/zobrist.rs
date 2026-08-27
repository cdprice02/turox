//! Incremental Zobrist hashing of a `Board`, for transposition-table lookups.
//!
//! Not yet implemented. This is a placeholder for the search change. The intended
//! shape: a set of random `u64` keys (one per `(Square, ColoredPiece)`, plus
//! side-to-move, castling rights, and en-passant file) generated once at
//! `const`/static init, XORed together for a position's initial hash, and then
//! incrementally XORed in `Board::make_move` rather than recomputed from scratch
//! per move.

// TODO(zobrist): const-generate the key table (a fixed PRNG seed, not
// `std::random`, so hashes are reproducible across runs and platforms) and add a
// `Board::zobrist_hash(&self) -> u64` that folds it in.
