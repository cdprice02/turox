//! Move generation: attack tables, sliding-piece magics, square-attack queries,
//! and pin/check-aware legal move generation.
//!
//! `tables` (leaper attacks), `magic` (slider attacks), `attacks`
//! (square-attack queries built on both), `move_list` (the stack-allocated
//! move buffer), and `pseudo_legal` (per-piece pseudolegal generation) are
//! done. `legal` (the check-filtered wrapper around it, plus perft) is not yet
//! started.

pub mod attacks;
pub mod legal;
pub mod magic;
pub mod move_list;
pub mod pseudo_legal;
pub mod tables;
