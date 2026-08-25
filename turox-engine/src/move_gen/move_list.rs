//! A stack-allocated move buffer for legal move generation to fill.
//!
//! `Move` is a `Copy` 2-byte value with no `Drop` (see `types::moves`), so a fixed
//! `[Move; 256]` + length is the whole implementation — no `arrayvec` dependency,
//! no `MaybeUninit`, no unsafe. 256 is comfortably above the maximum legal moves in
//! any reachable chess position (218, in a constructed extreme position); 512
//! bytes total versus `Vec<Move>`'s heap allocation per position is the entire
//! point of `Move` being packed into a `u16` rather than kept as a wider struct.
//!
//! Nothing fills one of these yet — that's `pseudo_legal`/`legal`'s job — but
//! the buffer itself is complete and independently tested.

use crate::{types::Move, MoveFlags, Square};
use std::fmt;

pub struct MoveList {
    moves: [Move; Self::CAPACITY],
    len: usize,
}

impl MoveList {
    pub const CAPACITY: usize = 256;
    const SENTINEL: Move = Move::new(Square::A1, Square::A1, MoveFlags::Quiet);

    pub fn new() -> Self {
        Self {
            moves: [Self::SENTINEL; 256],
            len: 0,
        }
    }

    /// Appends `m`. Panics if the list is already at `CAPACITY` - legal move
    /// generation should never produce more moves than that from a reachable
    /// position, so this is a bug check, not a runtime condition to handle.
    pub fn push(&mut self, m: Move) {
        if self.len >= Self::CAPACITY {
            panic!("MoveList already at capacity. A valid chess position should never reach this number ({}) of moves.", self.len + 1);
        }
        self.moves[self.len] = m;
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

/// Lets a `&MoveList` be used anywhere a `&[Move]` is expected (`.iter()`,
/// `.contains(..)`, slicing, ...) without callers writing `.as_slice()`
/// everywhere. Pure delegation, so it's correct as soon as `as_slice` is.
impl std::ops::Deref for MoveList {
    type Target = [Move];

    fn deref(&self) -> &[Move] {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/// The live slice only — not all 256 backing-array entries, most of which are
/// unused filler past `len`.
impl fmt::Debug for MoveList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MoveFlags, Square};

    fn m(from: Square, to: Square) -> Move {
        Move::new(from, to, MoveFlags::Quiet)
    }

    #[test]
    fn new_is_empty() {
        let list = MoveList::new();
        assert_eq!(list.len(), 0);
        assert!(list.is_empty());
        assert!(list.as_slice().is_empty());
    }

    #[test]
    fn push_then_as_slice_round_trips_in_order() {
        let mut list = MoveList::new();
        let moves = [
            m(Square::E2, Square::E4),
            m(Square::G1, Square::F3),
            m(Square::B1, Square::C3),
        ];
        for &mv in &moves {
            list.push(mv);
        }
        assert_eq!(list.len(), 3);
        assert!(!list.is_empty());
        assert_eq!(list.as_slice(), &moves);
    }

    #[test]
    fn fills_to_capacity_without_panicking() {
        let mut list = MoveList::new();
        for i in 0..MoveList::CAPACITY {
            let sq = Square::from_index((i % 64) as u8).expect("i % 64 < 64");
            list.push(m(Square::A1, sq));
        }
        assert_eq!(list.len(), MoveList::CAPACITY);
    }

    #[test]
    #[should_panic]
    fn pushing_past_capacity_panics() {
        let mut list = MoveList::new();
        for i in 0..MoveList::CAPACITY {
            let sq = Square::from_index((i % 64) as u8).expect("i % 64 < 64");
            list.push(m(Square::A1, sq));
        }
        list.push(m(Square::A1, Square::A1));
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(MoveList::default().len(), 0);
    }

    #[test]
    fn deref_matches_as_slice() {
        let mut list = MoveList::new();
        list.push(m(Square::E2, Square::E4));
        list.push(m(Square::D2, Square::D4));
        let via_deref: &[Move] = &list;
        assert_eq!(via_deref, list.as_slice());
    }

    #[test]
    fn into_iter_matches_as_slice() {
        let mut list = MoveList::new();
        list.push(m(Square::E2, Square::E4));
        list.push(m(Square::D2, Square::D4));
        let collected: Vec<Move> = (&list).into_iter().copied().collect();
        assert_eq!(collected, list.as_slice());
    }

    #[test]
    fn debug_shows_only_pushed_moves_not_filler() {
        let mut list = MoveList::new();
        list.push(m(Square::E2, Square::E4));
        let debug = format!("{list:?}");
        assert_eq!(debug.matches("e4").count(), 1, "debug output: {debug}");
    }
}
