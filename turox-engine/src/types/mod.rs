//! Core value types shared by `board`, `move_gen`, `search`, and `eval`.
//!
//! These live at the crate root rather than nested under `board/` because move
//! generation, search, and evaluation all need `Bitboard`/`Square`/`Move` without
//! depending on `Board` itself.

pub mod bitboard;
pub mod castling;
pub mod color;
pub mod moves;
pub mod piece;
pub mod square;

pub use bitboard::{Bitboard, Direction};
pub use castling::CastlingRights;
pub use color::Color;
pub use moves::{Move, MoveFlags};
pub use piece::{ColoredPiece, Piece};
pub use square::{File, Rank, Square};
