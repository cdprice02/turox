//! Move generation: attack tables, sliding-piece magics, square-attack
//! queries, pseudolegal generation, and pin/check-aware legal move generation
//! plus `perft`.
//!
//! All six submodules are done: `tables` (leaper attacks), `magic` (slider
//! attacks), `attacks` (square-attack queries built on both), `move_list`
//! (the stack-allocated move buffer), `pseudo_legal` (per-piece pseudolegal
//! generation), and `legal` (the check-filtered wrapper around it, plus
//! `perft`) — verified end-to-end against all six standard perft test
//! positions (`tests/perft.rs`), including their deep (`#[ignore]`d by
//! default) depths.

pub mod attacks;
pub mod legal;
pub mod magic;
pub mod move_list;
pub mod pseudo_legal;
pub mod tables;
