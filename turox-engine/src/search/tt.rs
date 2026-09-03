//! Transposition table.
//!
//! A fixed-size hash table of previously-searched positions, keyed by the Zobrist hash
//! from `board::zobrist`, storing a score, best move, and search depth so transposing
//! move orders don't get re-searched from scratch. Owned by `uci::session::run` (like
//! `history`), not by `Search` itself, and threaded into a `Search` call the same way:
//! `Search` is rebuilt fresh on every `go`, so a table that lived inside it would never
//! see the transpositions that matter most in real play, ones found across *separate*
//! `go` calls in the same game.
//!
//! Signatures below reflect the design settled during planning; bodies are `todo!()`,
//! left for the actual bit-twiddling and control flow to fill in.

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
    /// Narrowed from [`Score`] (`i32`); [`crate::search::MATE`]'s own magnitude leaves
    /// enough headroom under `i16::MAX` that this is always lossless. Ply-adjusted: not
    /// the score as seen from the root, but a form independent of *how* this node was
    /// reached, so a later [`Tt::probe`] at a different ply from a different path can
    /// correctly re-derive its own root-relative value from it.
    pub score: i16,
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
    /// adjustment `Tt::store` applied before narrowing this entry's own `score`; see that
    /// method's doc for why the adjustment exists at all.
    #[must_use]
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn cutoff_score(
        &self,
        min_depth: u32,
        alpha: Score,
        beta: Score,
        ply: u16,
    ) -> Option<Score> {
        let _ = (min_depth, alpha, beta, ply);
        todo!("depth/bound eligibility, then undo the ply adjustment on `self.score`")
    }
}

/// A fixed-size, always-replace transposition table.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "written by `new`/`store`, read by `probe`; all three are still `todo!()` stubs"
)]
pub struct Tt {
    entries: Vec<Option<Entry>>,
    mask: u64,
}

impl Tt {
    /// Builds a table sized to fit within `hash_mb` megabytes, rounded *down* to the
    /// largest power-of-two entry count that fits, so a lookup's index is a plain `key &
    /// mask`, never a division or modulo.
    #[must_use]
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn new(hash_mb: usize) -> Self {
        let _ = hash_mb;
        todo!("compute the power-of-two entry count from `hash_mb`, size `entries`, derive `mask`")
    }

    /// Rebuilds this table at a new size, discarding every existing entry. What a GUI's
    /// `setoption name Hash value <n>` drives.
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn resize(&mut self, hash_mb: usize) {
        let _ = hash_mb;
        todo!("replace `self` with a freshly sized table")
    }

    /// Wipes every entry without changing the table's size. What `ucinewgame` drives,
    /// deliberately distinct from `resize`: the size isn't changing, only what's cached
    /// from a game that's now over.
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn clear(&mut self) {
        todo!("reset every slot to empty")
    }

    /// Looks up `key`, returning the entry stored there only if its own `key` actually
    /// matches (not just the index): the table is smaller than the full key space, so a
    /// different position can and will land on the same index. Whether the entry found
    /// this way can resolve the probing node outright is [`Entry::cutoff_score`]'s
    /// question, not this method's; this only answers "what, if anything, is here."
    #[must_use]
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn probe(&self, key: u64) -> Option<Entry> {
        let _ = key;
        todo!("compute the index from `key & self.mask`, return the slot only on a real key match")
    }

    /// Records `key`'s search result at `ply` (distance from the root): `score` is
    /// ply-adjusted to a form independent of how this node was reached before being
    /// narrowed to `i16` (see [`Entry::score`]'s own doc), `depth` and `mv` are stored as
    /// given, and `bound` is derived from comparing `score` against `alpha`/`beta` (see
    /// [`Bound`]'s own doc for the exact mapping, and the gotcha in comparing against the
    /// right ones). Always-replace: overwrites whatever was in this slot before, no
    /// depth-preferred comparison.
    #[allow(
        clippy::too_many_arguments,
        reason = "one slot's worth of independent fields plus the alpha/beta window `Bound` \
                  is derived from; free to regroup into a params struct while implementing \
                  if that reads better"
    )]
    #[allow(clippy::todo, reason = "stub: signature settled, body still to write")]
    pub fn store(
        &mut self,
        key: u64,
        ply: u16,
        depth: u32,
        score: Score,
        alpha: Score,
        beta: Score,
        mv: Move,
    ) {
        let _ = (key, ply, depth, score, alpha, beta, mv);
        todo!("derive `Bound`, ply-adjust and narrow `score`, write the slot at `key & self.mask`")
    }
}
