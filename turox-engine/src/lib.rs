//! turox: a chess engine.
//!
//! # Architecture
//!
//! - [`types`] — core value types (`Bitboard`, `Square`, `Color`, `Piece`, `Move`,
//!   ...) with no dependency on `Board`. Re-exported at the crate root, so callers
//!   write `turox_engine::Bitboard` rather than reaching into the module.
//! - [`board`] — `Board` (piece placement plus game state) and FEN parsing/
//!   formatting, built on `types`.
//! - [`move_gen`] — attack tables, magic bitboards, pseudolegal and legal move
//!   generation, and `perft`.
//! - `search` — iterative deepening search over a transposition table.
//!   *(planned)*
//! - `eval` — static position evaluation. *(planned)*
//! - `uci` — the UCI protocol, driving the engine from `turox-cli`. *(planned)*
//!
//! `types` sits at the crate root rather than under `board` because move
//! generation, search, and evaluation all need `Bitboard`/`Square`/`Move` without
//! depending on `Board` itself.

pub mod board;
pub mod eval;
pub mod move_gen;
mod rng;
pub mod search;
pub mod types;
pub mod uci;

pub use types::*;

#[derive(Debug, Default)]
pub struct Engine {
    board: board::Board,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn board(&self) -> &board::Board {
        &self.board
    }

    pub fn run(&self) {}
}
