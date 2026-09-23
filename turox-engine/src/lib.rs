//! turox: a chess engine.
//!
//! # Architecture
//!
//! - [`search`]: negamax with alpha-beta over iterative deepening, driven by a
//!   depth, node, or time budget, with a transposition table and move ordering.
//! - [`eval`]: static position evaluation, tapered between midgame and endgame.
//!   Its submodule list is the term list; prose here would only go stale.
//! - [`uci`]: the UCI protocol, driving the engine from `turox-cli`.
//!
//! What a position *is* lives in `turox-chess` (`types`, `board`, `move_gen`,
//! `book`); this crate is the part that decides which move to play. The
//! dependency runs one way and the compiler keeps it that way, so a tool that
//! parses notation or builds an opening book never compiles a search.

// `missing_docs` covers public items; this covers the rest. It sits here rather
// than in the workspace `[lints]` table because that table reaches every
// target, and a doc per fixture constant in `tests/` and `benches/` is noise.
#![deny(clippy::missing_docs_in_private_items)]

pub mod eval;
pub mod search;
pub mod uci;

use turox_chess::board::Board;
use turox_chess::book::Book;
/// The engine's top-level handle: the position it's tracking, plus the
/// loop that drives it from a UCI-speaking GUI.
#[derive(Debug, Default)]
pub struct Engine {
    /// The position the session is tracking, rebuilt by each `position`
    /// command rather than mutated move by move.
    board: Board,
    /// Consulted before every `go` reaches `Search`; `None` (the default)
    /// means every `go` searches exactly as it always has.
    book: Option<Book>,
}

impl Engine {
    /// A new engine on the default (empty) `Board`, with no opening book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches an opening book, consulted before every `go` reaches
    /// `Search` for the rest of this engine's life.
    #[must_use]
    pub fn with_book(mut self, book: Book) -> Self {
        self.book = Some(book);
        self
    }

    /// The position the engine is currently tracking.
    #[must_use]
    pub const fn board(&self) -> &Board {
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
        uci::run_session(&mut self.board, self.book.as_ref(), reader, writer);
    }
}
