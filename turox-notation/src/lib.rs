//! Reading and writing chess games as text.
//!
//! - [`pgn`]: parsing PGN, either whole-file or one game at a time.
//! - [`san`]: resolving a SAN token against a position into a real `Move`.
//!
//! Separate from `turox-chess` because notation is a way of writing games down
//! rather than a rule of the game. See `docs/adr/0008`.

#![deny(clippy::missing_docs_in_private_items)]

pub mod pgn;
pub mod san;
