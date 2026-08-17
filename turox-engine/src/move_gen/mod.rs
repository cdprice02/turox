//! Move generation: attack tables, sliding-piece magics, and pin/check-aware legal
//! move generation. Not yet implemented — depends on `Bitboard`'s core arithmetic
//! and scanning primitives (see `types::bitboard`), which are the current exercise.

pub mod legal;
pub mod magic;
pub mod move_list;
pub mod tables;
