//! The rules of chess: what a position is, what moves are legal from it, and
//! how positions are identified.
//!
//! # Architecture
//!
//! - [`types`]: core value types (`Bitboard`, `Square`, `Color`, `Piece`, `Move`,
//!   ...) with no dependency on `Board`. Re-exported at the crate root, so callers
//!   write `turox_chess::Bitboard` rather than reaching into the module.
//! - [`board`]: `Board` (piece placement plus game state) and FEN parsing/
//!   formatting, built on `types`.
//! - [`book`]: the opening book's file format and lookup, keyed on
//!   `board::zobrist`'s hash. Here rather than in `turox-engine` because the
//!   generator writing it has to agree with the engine reading it.
//! - [`move_gen`]: attack tables, magic bitboards, pseudolegal and legal move
//!   generation, and `perft`.
//!
//! `types` sits at the crate root rather than under `board` because move
//! generation, search, and evaluation all need `Bitboard`/`Square`/`Move` without
//! depending on `Board` itself.
//!
//! Nothing here knows how to choose a move; that is `turox-engine`'s job. See
//! `docs/adr/0008` for why the two are separate crates.

// `missing_docs` covers public items; this covers the rest. It sits here rather
// than in the workspace `[lints]` table because that table reaches every
// target, and a doc per fixture constant in `tests/` and `benches/` is noise.
#![deny(clippy::missing_docs_in_private_items)]

pub mod board;
pub mod book;
pub mod move_gen;
// Test support, behind a feature so a normal build never compiles it.
#[cfg(feature = "strategies")]
pub mod strategies;
pub mod types;

pub use types::*;
