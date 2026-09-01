//! A stack-allocated move buffer for legal move generation to fill.
//!
//! `Move` is a `Copy` 2-byte value with no `Drop` (see `types::moves`), so a fixed
//! `[Move; 256]` + length is the whole implementation: no `arrayvec` dependency,
//! no `MaybeUninit`, no unsafe. 256 is comfortably above the maximum legal moves in
//! any reachable chess position (218, in a constructed extreme position); 512
//! bytes total versus `Vec<Move>`'s heap allocation per position is the entire
//! point of `Move` being packed into a `u16` rather than kept as a wider struct.
//!
//! Filled by `pseudo_legal`/`legal`, and reordered in place (`as_mut_slice`/
//! `DerefMut`) by `search`'s move ordering once a position's moves are on
//! hand to sort.

use crate::{types::Move, MoveFlags, Square};
use std::fmt;

/// A fixed-capacity buffer of moves, filled by `pseudo_legal`/`legal`.
pub struct MoveList {
    moves: [Move; Self::CAPACITY],
    len: usize,
}

impl MoveList {
    /// The maximum number of moves a single `MoveList` can hold.
    pub const CAPACITY: usize = 256;
    const SENTINEL: Move = Move::new(Square::A1, Square::A1, MoveFlags::Quiet);

    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            moves: [Self::SENTINEL; Self::CAPACITY],
            len: 0,
        }
    }

    /// Appends `m`.
    ///
    /// # Panics
    ///
    /// If the list is already at `CAPACITY`: legal move generation should never produce
    /// more moves than that from a reachable position, so this is a bug check, not a
    /// runtime condition to handle. The panic message can't include the actual length:
    /// `const fn` panics only accept a literal string, not `format!`-style arguments.
    pub const fn push(&mut self, m: Move) {
        assert!(self.len < Self::CAPACITY, "MoveList already at capacity; a valid chess position should never reach this many moves");
        self.moves[self.len] = m;
        self.len += 1;
    }

    /// Keeps only the moves for which `f` returns `true`, in place and in
    /// order. `legal_moves` uses this to filter `pseudo_legal_moves`'s output
    /// down to legal moves without allocating a second `MoveList` (and paying
    /// for its 256-entry sentinel-array init) just to copy the survivors into.
    pub fn retain(&mut self, mut f: impl FnMut(Move) -> bool) {
        let mut write = 0;
        for read in 0..self.len {
            if f(self.moves[read]) {
                self.moves[write] = self.moves[read];
                write += 1;
            }
        }
        self.len = write;
    }

    /// The number of moves pushed so far.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no moves have been pushed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The pushed moves, in push order.
    #[must_use]
    pub const fn as_slice(&self) -> &[Move] {
        // `&self.moves[..self.len]` would be more idiomatic, but range
        // indexing isn't const-callable yet (`Index` isn't a const trait on
        // stable); `split_at` is.
        self.moves.split_at(self.len).0
    }

    /// The pushed moves, in push order, mutably: what move ordering (e.g.
    /// MVV-LVA capture ordering) sorts in place. Same `split_at`-based
    /// approach as `as_slice`, so it's just as incapable of touching the
    /// unused tail past `len`: the returned slice never includes it.
    pub const fn as_mut_slice(&mut self) -> &mut [Move] {
        self.moves.split_at_mut(self.len).0
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

/// Same reasoning as `Deref`, mutably: lets a `&mut MoveList` be sorted
/// in place (`.sort_by_key(..)`, `.reverse()`, ...) without callers writing
/// `.as_mut_slice()` everywhere.
impl std::ops::DerefMut for MoveList {
    fn deref_mut(&mut self) -> &mut [Move] {
        self.as_mut_slice()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/// The live slice only, not all 256 backing-array entries, most of which are
/// unused filler past `len`.
impl fmt::Debug for MoveList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MoveFlags, Rank, Square};

    fn m(from: Square, to: Square) -> Move {
        Move::new(from, to, MoveFlags::Quiet)
    }

    #[test]
    fn new_is_empty() {
        let list = MoveList::new();
        assert_eq!(list.len(), 0);
        assert!(list.is_empty());
        assert_eq!(list.as_slice(), []);
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
    fn retain_drops_non_matching_and_preserves_order_of_the_rest() {
        let mut list = MoveList::new();
        let moves = [
            m(Square::E2, Square::E4),
            m(Square::G1, Square::F3),
            m(Square::B1, Square::C3),
            m(Square::D2, Square::D4),
        ];
        for &mv in &moves {
            list.push(mv);
        }
        // Keep only moves landing on the 4th rank: E4 and D4, in their
        // original relative order.
        list.retain(|mv| mv.to().rank() == Rank::R4);
        assert_eq!(list.as_slice(), &[moves[0], moves[3]]);
    }

    #[test]
    fn retain_keeping_everything_is_a_no_op() {
        let mut list = MoveList::new();
        let moves = [m(Square::E2, Square::E4), m(Square::G1, Square::F3)];
        for &mv in &moves {
            list.push(mv);
        }
        list.retain(|_| true);
        assert_eq!(list.as_slice(), &moves);
    }

    #[test]
    fn retain_dropping_everything_empties_the_list() {
        let mut list = MoveList::new();
        list.push(m(Square::E2, Square::E4));
        list.push(m(Square::G1, Square::F3));
        list.retain(|_| false);
        assert!(list.is_empty());
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
    #[should_panic = "MoveList already at capacity; a valid chess position should never reach this many moves"]
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
    fn as_mut_slice_reorders_in_place_without_disturbing_len_or_the_tail() {
        let mut list = MoveList::new();
        let moves = [
            m(Square::E2, Square::E4),
            m(Square::G1, Square::F3),
            m(Square::B1, Square::C3),
        ];
        for &mv in &moves {
            list.push(mv);
        }
        list.as_mut_slice().reverse();
        assert_eq!(list.len(), 3);
        assert_eq!(list.as_slice(), &[moves[2], moves[1], moves[0]]);

        // If reordering had somehow touched `len` or the backing array past
        // it (rather than being confined to the live `len()`-element slice
        // `as_mut_slice` hands out), a push right after would land on the
        // wrong index or silently overwrite something already there.
        let fourth = m(Square::D2, Square::D4);
        list.push(fourth);
        assert_eq!(list.len(), 4);
        assert_eq!(list.as_slice(), &[moves[2], moves[1], moves[0], fourth]);
    }

    #[test]
    fn deref_mut_matches_as_mut_slice() {
        let mut list = MoveList::new();
        list.push(m(Square::E2, Square::E4));
        list.push(m(Square::D2, Square::D4));
        list.reverse(); // via DerefMut, not a direct as_mut_slice() call
        assert_eq!(
            list.as_slice(),
            &[m(Square::D2, Square::D4), m(Square::E2, Square::E4)]
        );
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
