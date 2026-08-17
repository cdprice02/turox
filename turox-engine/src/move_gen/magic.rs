//! Sliding-piece (bishop, rook, queen) attack generation via magic bitboards.
//!
//! Not yet implemented. This is the piece that most directly needs
//! `Bitboard::subsets` (Carry-Rippler enumeration of an occupancy mask) to
//! generate its lookup tables — table generation walks every possible blocker
//! occupancy for each square's relevant mask and records the resulting attack
//! set, then finds a magic multiplier that hashes each occupancy to a distinct
//! table slot.
//!
//! Intended shape: per-square relevant-occupancy masks, magic numbers (found by
//! search or hardcoded from a known-good set), and a flat attack table indexed by
//! `(magic * occupancy) >> shift`. `fn bishop_attacks(sq: Square, occupied:
//! Bitboard) -> Bitboard` and `fn rook_attacks(sq: Square, occupied: Bitboard) ->
//! Bitboard` are the two entry points `legal.rs` will call; `queen_attacks` is
//! just their union.
