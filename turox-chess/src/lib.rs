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
//!   `board::zobrist`'s hash. Here rather than in `turox-engine` because it has
//!   two callers that must agree on the format, the engine reading it and the
//!   generator writing it, and only one of those is the engine.
//! - [`move_gen`]: attack tables, magic bitboards, pseudolegal and legal move
//!   generation, and `perft`.
//!
//! `types` sits at the crate root rather than under `board` because move
//! generation, search, and evaluation all need `Bitboard`/`Square`/`Move` without
//! depending on `Board` itself.
//!
//! Nothing here knows how to choose a move. That is `turox-engine`'s job, and
//! the split is a one-way dependency the compiler enforces: a tool that parses
//! notation or generates a book takes this crate and never compiles a search.

// `missing_docs` covers public items; this covers the rest. It sits here rather
// than in the workspace `[lints]` table because that table reaches every
// target, and a doc per fixture constant in `tests/` and `benches/` is noise.
#![deny(clippy::missing_docs_in_private_items)]

pub mod board;
pub mod book;
pub mod move_gen;
// Test support, not part of what this crate is for, which is why it is behind a
// feature rather than always compiled: `turox-engine`'s property tests need the
// same `Board` strategies these define, and two copies of 200 lines of
// generator is exactly the duplication this repo keeps getting bitten by.
#[cfg(feature = "strategies")]
pub mod strategies;
pub mod types;

pub use types::*;
