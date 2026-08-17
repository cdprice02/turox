//! A stack-allocated move buffer for legal move generation to fill.
//!
//! `Move` is a `Copy` 2-byte value with no `Drop` (see `types::moves`), so a fixed
//! `[Move; 256]` + length is the whole implementation — no `arrayvec` dependency,
//! no `MaybeUninit`, no unsafe. 256 is comfortably above the maximum legal moves in
//! any reachable chess position (218, in a constructed extreme position); 512
//! bytes total versus `Vec<Move>`'s heap allocation per position is the entire
//! point of `Move` being packed into a `u16` rather than kept as a wider struct.
//!
//! Methods are stubbed pending the move-generation change that will actually fill
//! one of these.

use crate::types::Move;

// Fields are unread for now: every method that would touch them is `todo!()`.
// Drop this once `new`/`push`/etc. are implemented.
#[allow(dead_code)]
pub struct MoveList {
    moves: [Move; Self::CAPACITY],
    len: usize,
}

impl MoveList {
    pub const CAPACITY: usize = 256;

    #[allow(unused_variables, clippy::new_without_default)]
    pub fn new() -> Self {
        todo!()
    }

    /// Appends `m`. Panics if the list is already at `CAPACITY` — legal move
    /// generation should never produce more moves than that from a reachable
    /// position, so this is a bug check, not a runtime condition to handle.
    #[allow(unused_variables)]
    pub fn push(&mut self, m: Move) {
        todo!()
    }

    pub fn len(&self) -> usize {
        todo!()
    }

    pub fn is_empty(&self) -> bool {
        todo!()
    }

    pub fn as_slice(&self) -> &[Move] {
        todo!()
    }
}
