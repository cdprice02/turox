//! turox: a chess engine.
//!
//! # Architecture
//!
//! - [`types`]: core value types (`Bitboard`, `Square`, `Color`, `Piece`, `Move`,
//!   ...) with no dependency on `Board`. Re-exported at the crate root, so callers
//!   write `turox_engine::Bitboard` rather than reaching into the module.
//! - [`board`]: `Board` (piece placement plus game state) and FEN parsing/
//!   formatting, built on `types`.
//! - [`move_gen`]: attack tables, magic bitboards, pseudolegal and legal move
//!   generation, and `perft`.
//! - [`search`]: negamax with alpha-beta over iterative deepening, driven by
//!   a depth or node budget (a transposition table is a later addition).
//! - [`eval`]: static position evaluation (material and piece-square
//!   tables).
//! - [`uci`]: the UCI protocol, driving the engine from `turox-cli`.
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

/// The engine's top-level handle: the position it's tracking, plus the
/// loop that drives it from a UCI-speaking GUI.
#[derive(Debug, Default)]
pub struct Engine {
    board: board::Board,
}

impl Engine {
    /// A new engine on the default (empty) `Board`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The position the engine is currently tracking.
    pub fn board(&self) -> &board::Board {
        &self.board
    }

    /// Drives the engine from real stdin/stdout via UCI. What `turox-cli`
    /// actually calls; see [`Engine::run_with_io`] for the generic, directly
    /// testable version this wraps.
    pub fn run(&mut self) {
        // `BufReader::new(stdin())`, not `stdin().lock()`: `run_with_io`
        // moves `reader` onto its own thread, and `StdinLock` isn't `Send`
        // (it holds a `MutexGuard`) even though plain `Stdin` is. Slightly
        // more per-read locking overhead than a pre-acquired lock, entirely
        // negligible for a UCI engine reading occasional command lines.
        let reader = std::io::BufReader::new(std::io::stdin());
        let stdout = std::io::stdout();
        self.run_with_io(reader, stdout.lock());
    }

    /// Drives the engine from `reader`/`writer` via UCI, until `quit`
    /// arrives or `reader` runs out of input. Generic over `R`/`W` (rather
    /// than hardcoded to real stdin/stdout) so a test can drive a whole
    /// session against in-memory buffers instead of a real process's
    /// standard streams; see `uci::session` for the actual loop.
    pub fn run_with_io<R, W>(&mut self, reader: R, writer: W)
    where
        R: std::io::BufRead + Send + 'static,
        W: std::io::Write,
    {
        uci::run_session(&mut self.board, reader, writer);
    }
}
