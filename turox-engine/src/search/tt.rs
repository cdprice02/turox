//! Transposition table.
//!
//! A fixed-size hash table of previously-searched positions, keyed by the Zobrist hash
//! from `board::zobrist`, storing a score, best move, and search depth so transposing
//! move orders don't get re-searched from scratch. Owned by `uci::session::run` (like
//! `history`), not by `Search` itself, and threaded into a `Search` call the same way:
//! `Search` is rebuilt fresh on every `go`, so a table that lived inside it would never
//! see the transpositions that matter most in real play, ones found across *separate*
//! `go` calls in the same game.

use crate::eval::Score;
use crate::types::Move;

/// How a stored score relates to the true minimax value at the depth it was searched to.
///
/// Determined by comparing the score against the *original* `alpha`/`beta` a node was
/// searched with, not narrowed values a move loop tightened along the way: comparing
/// against the wrong (already-tightened) `alpha` is a classic, quiet way to produce a
/// cutoff that looks plausible but isn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// Nothing beat the original `alpha`: the true value is at most this score.
    UpperBound,
    /// The true minimax value, no bound involved.
    Exact,
    /// A beta cutoff happened: the true value is at least this score.
    LowerBound,
}

/// One transposition table slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The full Zobrist key this entry was stored under, kept alongside the table index
    /// so a hash collision at the same index can't be mistaken for a real hit on `probe`.
    pub key: u64,
    /// The best move found at this node, packed via `Move::bits`. Write-only in this
    /// module: nothing here reads it back yet, since using it to reorder a node's move
    /// list is separate, later work. Stored now so that later change doesn't need to
    /// touch this struct's layout.
    pub mv: u16,
    /// Ply-adjusted: not the score as seen from the root, but a form independent of *how*
    /// this node was reached, so a later [`Tt::probe`] at a different ply from a different
    /// path can correctly re-derive its own root-relative value from it.
    pub score: Score,
    /// The remaining search depth `score` was computed at.
    pub depth: u8,
    /// How `score` relates to the true minimax value; see [`Bound`]'s own doc.
    pub bound: Bound,
}

impl Entry {
    /// Whether this entry can resolve the probing node outright, and if so, the score to
    /// return: `None` when either the search behind this entry didn't go deep enough
    /// (`self.depth < min_depth`) or its `bound` doesn't actually license a cutoff against
    /// `alpha`/`beta` (`Exact` always does; `LowerBound` only once its score already meets
    /// or beats `beta`; `UpperBound` only once its score already falls at or below
    /// `alpha`).
    ///
    /// `ply` is the probing node's distance from the root, used to undo the ply
    /// adjustment `Tt::store` applied to this entry's own `score`; see that method's doc
    /// for why the adjustment exists at all.
    #[must_use]
    pub fn cutoff_score(&self, min_depth: u8, alpha: Score, beta: Score, ply: u8) -> Option<Score> {
        if self.depth < min_depth {
            return None;
        }
        let score = self.score + Score::from(ply);
        match self.bound {
            Bound::Exact => Some(score),
            Bound::LowerBound if score >= beta => Some(score),
            Bound::UpperBound if score <= alpha => Some(score),
            Bound::UpperBound | Bound::LowerBound => None,
        }
    }
}

/// A fixed-size, always-replace transposition table.
#[derive(Debug)]
pub struct Tt {
    entries: Vec<Option<Entry>>,
    mask: u64,
}

impl Tt {
    /// Builds a table sized to fit within `hash_mb` megabytes, rounded *down* to the
    /// largest power-of-two entry count that fits, so a lookup's index is a plain `key &
    /// mask`, never a division or modulo.
    #[must_use]
    pub fn new(hash_mb: usize) -> Self {
        let bytes = hash_mb * 1024 * 1024;
        let entry_size = std::mem::size_of::<Option<Entry>>();
        let count = u64::try_from(bytes / entry_size).unwrap_or(u64::MAX); // saturate to u64::MAX if the caller asked for more than that
        let pow2_count = if count.is_power_of_two() {
            count
        } else {
            count.next_power_of_two() >> 1
        };
        let mask = pow2_count - 1;
        let pow2_count = usize::try_from(pow2_count).unwrap_or(usize::MAX); // saturate to usize::MAX if the caller asked for more than that
        Self {
            entries: vec![None; pow2_count],
            mask,
        }
    }

    /// Rebuilds this table at a new size, discarding every existing entry. What a GUI's
    /// `setoption name Hash value <n>` drives.
    pub fn resize(&mut self, hash_mb: usize) {
        *self = Self::new(hash_mb);
    }

    /// Wipes every entry without changing the table's size. What `ucinewgame` drives,
    /// deliberately distinct from `resize`: the size isn't changing, only what's cached
    /// from a game that's now over.
    pub fn clear(&mut self) {
        self.entries.fill(None);
    }

    /// Looks up `key`, returning the entry stored there only if its own `key` actually
    /// matches (not just the index): the table is smaller than the full key space, so a
    /// different position can and will land on the same index. Whether the entry found
    /// this way can resolve the probing node outright is [`Entry::cutoff_score`]'s
    /// question, not this method's; this only answers "what, if anything, is here."
    ///
    /// Compares against the *full* `key`, not `key & self.mask`: every key that lands on
    /// a given index shares the same masked value by construction, so comparing masked
    /// values here would make every collision at this index look like a hit for whichever
    /// key was stored last, defeating the reason a full key is stored at all.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the `expect` on the index conversion only fails if the
    /// entry count exceeds `usize::MAX`, and `Hash`'s own advertised ceiling (1024 MB) is
    /// nowhere near large enough to produce that many entries even on a 32-bit target.
    #[must_use]
    pub fn probe(&self, key: u64) -> Option<Entry> {
        let index = usize::try_from(key & self.mask)
            .expect("Hash's advertised MB ceiling keeps the entry count well within usize");
        self.entries[index].filter(|entry| entry.key == key)
    }

    /// Records `key`'s search result at `ply` (distance from the root): `score` is
    /// ply-adjusted to a form independent of how this node was reached (see
    /// [`Entry::score`]'s own doc), `depth` and `mv` are stored as given, and `bound` is
    /// derived from comparing `score` against `alpha`/`beta` (see [`Bound`]'s own doc for
    /// the exact mapping, and the gotcha in comparing against the right ones).
    /// Always-replace: overwrites whatever was in this slot before, no depth-preferred
    /// comparison.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the `expect` on the index conversion only fails if the
    /// entry count exceeds `usize::MAX`, and `Hash`'s own advertised ceiling (1024 MB) is
    /// nowhere near large enough to produce that many entries even on a 32-bit target.
    #[allow(
        clippy::too_many_arguments,
        reason = "one slot's worth of independent fields plus the alpha/beta window `Bound` \
                  is derived from; free to regroup into a params struct while implementing \
                  if that reads better"
    )]
    pub fn store(
        &mut self,
        key: u64,
        ply: u8,
        depth: u8,
        score: Score,
        alpha: Score,
        beta: Score,
        mv: Move,
    ) {
        let bound = if score <= alpha {
            Bound::UpperBound
        } else if score >= beta {
            Bound::LowerBound
        } else {
            Bound::Exact
        };
        let entry = Entry {
            key,
            mv: mv.bits(),
            score: score - Score::from(ply),
            depth,
            bound,
        };
        let index = usize::try_from(key & self.mask)
            .expect("Hash's advertised MB ceiling keeps the entry count well within usize");
        self.entries[index] = Some(entry);
    }
}
