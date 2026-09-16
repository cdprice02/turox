//! Piece-to history: a fact about which quiet moves tend to be good, independent of any
//! one search tree.
//!
//! Complements the killer table (`negamax`'s own `Search::killers` field) rather than
//! replacing it: a killer is a fact about *this search tree's* sibling structure, rebuilt
//! fresh every `go` (ADR-0003). This table accumulates over many more nodes than two
//! killer slots ever see, and it survives across `go` calls the way the transposition
//! table does (ADR-0001, ADR-0005), aging by half each call rather than resetting to
//! nothing.
//!
//! Indexed `[side][piece][to]`, not the historically-original `[side][from][to]`
//! "butterfly board" CPW names the technique after: `docs/research/move-ordering.md`
//! found Stockfish itself indexes this way, and two different pieces leaving the same
//! square are unrelated events for "was this destination good," so the `from` square
//! carries no information this table needs. Quiet moves only, same rule as killers
//! (`Search::note_cutoff_move`'s own doc): a capture or promotion is already ordered by
//! MVV-LVA/material gain, so recording it here would duplicate that ordering.

use crate::eval::Score;
use crate::types::{Color, Piece, Square};

/// How far one `[side][piece][to]` cell's score can sit from zero in either direction.
///
/// Comfortably below `Score::MAX`/`Score::MIN`, so `update`'s `saturating_add` followed by
/// this clamp never has to reason about the addition itself overflowing `Score`: the
/// operand being added in is itself bounded (see [`history_delta`]), and the running total
/// is always re-clamped into this range immediately after every update.
const MAX_MAGNITUDE: Score = 16_384;

/// `depth * depth`: the classical history-heuristic weighting (CPW's page, per
/// `docs/research/move-ordering.md`), so a refutation found deep in the tree counts for
/// more than one found near the leaves, which there are vastly more of. Shared by both
/// the bonus (cutoff) and the malus (searched, no cutoff) side of an update: the two are
/// the same formula with opposite sign, not separate formulas to keep in sync.
///
/// Deliberately not history-gravity-scaled (bonus/malus shrinking as a cell nears the
/// ceiling): that's a real refinement, but landing it alongside this issue's own SPRT
/// would leave two variables moving under one measurement. Filed as a fast-follow.
///
/// Computed in `i32` and clamped down: `depth` is a plain `u8` with no enforced ceiling at
/// this layer (a GUI's `go depth` can ask for anything up to 255), and 255 * 255 overflows
/// `Score` (`i16`) well before `update`'s own clamp ever gets a chance to bound it.
fn history_delta(depth: u8) -> Score {
    let squared = i32::from(depth) * i32::from(depth);
    Score::try_from(squared).unwrap_or(Score::MAX)
}

/// Quiet-move ordering scores, `[side][piece][to]`. See the module doc for the shape and
/// lifetime reasoning.
#[derive(Debug)]
pub struct PieceToHistory {
    /// `[side][piece][to]`; see the module doc for why `from` carries no information this
    /// table needs.
    scores: [[[Score; Square::ALL.len()]; Piece::ALL.len()]; Color::ALL.len()],
}

impl Default for PieceToHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PieceToHistory {
    /// An empty table: every `[side][piece][to]` cell starts at `0`, meaning "no opinion,"
    /// the same neutral value ordering already falls back to when no history exists at all
    /// (see `negamax::move_priority`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scores: [[[0; Square::ALL.len()]; Piece::ALL.len()]; Color::ALL.len()],
        }
    }

    /// This cell's current score, for move ordering to read directly. `0` for a
    /// never-updated cell, same neutral meaning `new` starts every cell at.
    #[must_use]
    pub const fn score(&self, side: Color, piece: Piece, to: Square) -> Score {
        self.scores[side.index()][piece.index()][to.index()]
    }

    /// Records that `piece` moving to `to` (as `side`) caused a beta cutoff at search
    /// depth `depth`: raises that cell by `depth * depth`, clamped to a fixed ceiling well
    /// below `Score::MAX` (see this module's own doc).
    ///
    /// Callers must only call this for material-neutral moves (quiet moves, same rule as
    /// killers): a capture or promotion causing a cutoff is already explained by
    /// MVV-LVA/material gain, and recording it here would duplicate that signal under a
    /// different name. This type has no way to enforce that itself since it never sees a
    /// `Move`, only the `(side, piece, to)` a caller has already decided to record.
    pub fn record_cutoff(&mut self, side: Color, piece: Piece, to: Square, depth: u8) {
        self.update(side, piece, to, history_delta(depth));
    }

    /// Records that `piece` moving to `to` (as `side`) was searched at depth `depth` but
    /// did *not* cause the cutoff at its node: lowers that cell by the same `depth * depth`
    /// weighting and ceiling `record_cutoff` raises it by. This is the malus that keeps the
    /// table from saturating toward "every quiet move is good": without it, a move tried
    /// often enough (which every quiet move near the root is) would eventually look as
    /// good as one that actually keeps causing cutoffs.
    ///
    /// Same material-neutral-only contract as [`Self::record_cutoff`].
    pub fn record_no_cutoff(&mut self, side: Color, piece: Piece, to: Square, depth: u8) {
        self.update(side, piece, to, -history_delta(depth));
    }

    /// Shared by [`Self::record_cutoff`] and [`Self::record_no_cutoff`]: same clamp, same
    /// cell lookup, opposite sign on `delta`.
    fn update(&mut self, side: Color, piece: Piece, to: Square, delta: Score) {
        let cell = &mut self.scores[side.index()][piece.index()][to.index()];
        *cell = cell
            .saturating_add(delta)
            .clamp(-MAX_MAGNITUDE, MAX_MAGNITUDE);
    }

    /// Halves every cell toward zero, keeping each one's sign and rough relative standing
    /// against its neighbors while decaying old evidence. Called once per `go` command
    /// (`uci::session::run`), not once per node: this is about a table that outlives a
    /// single search tree tracking the *current* position rather than the whole game, not
    /// about anything that happens within one search; see the module doc and ADR-0005.
    pub fn age(&mut self) {
        for side in &mut self.scores {
            for piece in side {
                for cell in piece {
                    *cell /= 2;
                }
            }
        }
    }

    /// Wipes every cell back to `0`, distinct from [`Self::age`]'s halving: what
    /// `ucinewgame` drives (mirrors `Tt::clear`), since history from a finished game isn't
    /// merely stale evidence about the current one to decay, it's evidence about a
    /// different game entirely.
    pub const fn clear(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_table_has_no_opinion_on_any_cell() {
        let history = PieceToHistory::new();
        assert_eq!(history.score(Color::White, Piece::Knight, Square::F3), 0);
    }

    #[test]
    fn record_cutoff_raises_the_entry() {
        let mut history = PieceToHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        assert!(history.score(Color::White, Piece::Knight, Square::F3) > 0);
    }

    #[test]
    fn record_no_cutoff_lowers_the_entry() {
        let mut history = PieceToHistory::new();
        history.record_no_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        assert!(history.score(Color::White, Piece::Knight, Square::F3) < 0);
    }

    #[test]
    fn bonus_is_depth_weighted() {
        let mut shallow = PieceToHistory::new();
        shallow.record_cutoff(Color::White, Piece::Knight, Square::F3, 2);

        let mut deep = PieceToHistory::new();
        deep.record_cutoff(Color::White, Piece::Knight, Square::F3, 8);

        assert!(
            deep.score(Color::White, Piece::Knight, Square::F3)
                > shallow.score(Color::White, Piece::Knight, Square::F3),
            "a cutoff found at higher depth must raise the entry more than one found \
             near the leaves"
        );
    }

    #[test]
    fn updates_to_one_cell_do_not_affect_a_different_side_piece_or_destination() {
        let mut history = PieceToHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        assert_eq!(history.score(Color::Black, Piece::Knight, Square::F3), 0);
        assert_eq!(history.score(Color::White, Piece::Bishop, Square::F3), 0);
        assert_eq!(history.score(Color::White, Piece::Knight, Square::G3), 0);
    }

    #[test]
    fn age_halves_every_entry() {
        let mut history = PieceToHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        let before = history.score(Color::White, Piece::Knight, Square::F3);

        history.age();

        assert_eq!(
            history.score(Color::White, Piece::Knight, Square::F3),
            before / 2
        );
    }

    #[test]
    fn age_preserves_relative_order_between_two_positive_entries() {
        let mut history = PieceToHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 8);
        history.record_cutoff(Color::White, Piece::Bishop, Square::C4, 2);
        assert!(
            history.score(Color::White, Piece::Knight, Square::F3)
                > history.score(Color::White, Piece::Bishop, Square::C4)
        );

        history.age();

        assert!(
            history.score(Color::White, Piece::Knight, Square::F3)
                > history.score(Color::White, Piece::Bishop, Square::C4),
            "halving both entries must not change which one ranks higher"
        );
    }

    #[test]
    fn repeated_cutoffs_saturate_instead_of_overflowing() {
        let mut history = PieceToHistory::new();
        for _ in 0..1000 {
            history.record_cutoff(Color::White, Piece::Queen, Square::D4, 255);
        }
        assert_eq!(
            history.score(Color::White, Piece::Queen, Square::D4),
            MAX_MAGNITUDE
        );
    }

    #[test]
    fn repeated_maluses_saturate_instead_of_overflowing() {
        let mut history = PieceToHistory::new();
        for _ in 0..1000 {
            history.record_no_cutoff(Color::White, Piece::Queen, Square::D4, 255);
        }
        assert_eq!(
            history.score(Color::White, Piece::Queen, Square::D4),
            -MAX_MAGNITUDE
        );
    }

    #[test]
    fn clear_resets_every_cell_to_zero() {
        let mut history = PieceToHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        history.record_no_cutoff(Color::Black, Piece::Pawn, Square::E5, 2);

        history.clear();

        assert_eq!(history.score(Color::White, Piece::Knight, Square::F3), 0);
        assert_eq!(history.score(Color::Black, Piece::Pawn, Square::E5), 0);
    }
}
