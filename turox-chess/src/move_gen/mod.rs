//! Move generation: attack tables, sliding-piece magics, square-attack
//! queries, pseudolegal generation, and pin/check-aware legal move generation
//! plus `perft`.
//!
//! `tables` (leaper attacks), `magic` (slider attacks), `attacks`
//! (square-attack queries built on both), `move_list` (the stack-allocated
//! move buffer), `pseudo_legal` (per-piece pseudolegal generation), and
//! `legal` (the pin-aware wrapper around it, plus `perft`).
//!
//! Correctness rests on perft rather than on unit tests of the pieces:
//! `tests/perft.rs` walks the six standard positions to fixed depths and
//! compares exact node counts, which catches a rule error anywhere in the
//! chain in a way that per-piece assertions do not.

pub mod attacks;
pub mod legal;
pub mod magic;
pub mod move_list;
pub mod pseudo_legal;
pub mod tables;
