//! Static position evaluation: material, piece-square tables, and pawn structure
//! (using `Bitboard::north_fill`/`file_fill` for passed/isolated/doubled pawns once
//! those land), returned from the side-to-move's perspective.
//!
//! Not yet implemented; waits on `board` for position state and, for the pawn
//! structure terms specifically, on the Kogge-Stone fill methods on `Bitboard`.
