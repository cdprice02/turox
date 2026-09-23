//! The history heuristic: a fact about which quiet moves tend to be good, independent of
//! any one search tree.
//!
//! Complements the killer table (`negamax`'s own `Search::killers` field) rather than
//! replacing it: a killer is a fact about *this search tree's* sibling structure, rebuilt
//! fresh every `go`. This table accumulates over many more nodes than two killer slots
//! ever see, so it is threaded in from `uci::session::run` and survives across `go` calls
//! the way the transposition table does, aging by half each call rather than resetting to
//! nothing.
//!
//! Indexed `[side][piece][to]` rather than `[side][from][to]`: two different pieces
//! leaving the same square are unrelated events for "was this destination good," so the
//! `from` square carries no information this table needs. Quiet moves only, same rule as
//! killers (`Search::note_cutoff_move`'s own doc): a capture or promotion is already
//! ordered by MVV-LVA/material gain, so recording it here would duplicate that ordering.

use crate::eval::Score;
use turox_chess::types::{Color, Piece, Square};

/// How far one `[side][piece][to]` cell's score can sit from zero in either direction, and
/// the denominator `update`'s gravity term scales by: the two roles share one constant
/// deliberately, since gravity's whole point is damping growth as a cell nears this same
/// ceiling.
///
/// Comfortably below `Score::MAX`/`Score::MIN`, so `update`'s own final clamp into this
/// range always fits back into `Score` (`i16`) even though the arithmetic ahead of it runs
/// in `i32` to hold intermediate products that would overflow `Score` directly.
const MAX_MAGNITUDE: Score = 16_384;

/// `depth * depth`, so a cutoff found deep in the tree counts for more than one found near
/// the leaves, which there are vastly more of. Shared by both the bonus (cutoff) and the
/// malus (searched, no cutoff) side of an update: the two are the same formula with
/// opposite sign, not separate formulas to keep in sync.
///
/// Computed in `i32` and clamped down: `depth` is a plain `u8` with no enforced ceiling at
/// this layer (a GUI's `go depth` can ask for anything up to 255), and 255 * 255 overflows
/// `Score` (`i16`) well before `update`'s own clamp ever gets a chance to bound it.
fn cutoff_delta(depth: u8) -> Score {
    let squared = i32::from(depth) * i32::from(depth);
    Score::try_from(squared).unwrap_or(Score::MAX)
}

/// Quiet-move ordering scores, `[side][piece][to]`. See the module doc for the shape and
/// lifetime reasoning.
#[derive(Debug)]
pub struct CutoffHistory {
    /// `[side][piece][to]`; see the module doc for why `from` carries no information this
    /// table needs.
    scores: [[[Score; Square::ALL.len()]; Piece::ALL.len()]; Color::ALL.len()],
}

impl Default for CutoffHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl CutoffHistory {
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
        self.update(side, piece, to, cutoff_delta(depth));
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
        self.update(side, piece, to, -cutoff_delta(depth));
    }

    /// Shared by [`Self::record_cutoff`] and [`Self::record_no_cutoff`]: same cell lookup,
    /// opposite sign on `bonus`, and the same history-gravity formula on both.
    ///
    /// The standard gravity update (Stockfish and others use this exact shape): damps a
    /// same-direction bonus as `current` approaches the ceiling, but a bonus reversing a
    /// cell from the *opposite* extreme (a deeply malused cell suddenly causing a cutoff)
    /// moves it *more* than the same bonus would move a fresh cell, not less -- the
    /// surprising-reversal case this exists for. Computed in `i32`: `current * bonus.abs()`
    /// can reach `MAX_MAGNITUDE * Score::MAX`, well past what `Score` (`i16`) holds, before
    /// the division brings it back down.
    #[expect(
        clippy::expect_used,
        reason = "updated is clamped to ±MAX_MAGNITUDE on the line just above, and MAX_MAGNITUDE \
                  is itself a Score, so the conversion back can never fail"
    )]
    fn update(&mut self, side: Color, piece: Piece, to: Square, bonus: Score) {
        let cell = &mut self.scores[side.index()][piece.index()][to.index()];
        let current = i32::from(*cell);
        let bonus = i32::from(bonus);
        let gravity = current * bonus.abs() / i32::from(MAX_MAGNITUDE);
        let updated =
            (current + bonus - gravity).clamp(i32::from(-MAX_MAGNITUDE), i32::from(MAX_MAGNITUDE));
        *cell = Score::try_from(updated)
            .expect("clamped to ±MAX_MAGNITUDE just above, which always fits Score");
    }

    /// Halves every cell toward zero, keeping each one's sign and rough relative standing
    /// against its neighbors while decaying old evidence. Called once per `go` command
    /// (`uci::session::run`), not once per node: this is about a table that outlives a
    /// single search tree tracking the *current* position rather than the whole game, not
    /// about anything that happens within one search.
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
        let history = CutoffHistory::new();
        assert_eq!(history.score(Color::White, Piece::Knight, Square::F3), 0);
    }

    #[test]
    fn record_cutoff_raises_the_entry() {
        let mut history = CutoffHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        assert!(history.score(Color::White, Piece::Knight, Square::F3) > 0);
    }

    #[test]
    fn record_no_cutoff_lowers_the_entry() {
        let mut history = CutoffHistory::new();
        history.record_no_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        assert!(history.score(Color::White, Piece::Knight, Square::F3) < 0);
    }

    #[test]
    fn bonus_is_depth_weighted() {
        let mut shallow = CutoffHistory::new();
        shallow.record_cutoff(Color::White, Piece::Knight, Square::F3, 2);

        let mut deep = CutoffHistory::new();
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
        let mut history = CutoffHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        assert_eq!(history.score(Color::Black, Piece::Knight, Square::F3), 0);
        assert_eq!(history.score(Color::White, Piece::Bishop, Square::F3), 0);
        assert_eq!(history.score(Color::White, Piece::Knight, Square::G3), 0);
    }

    #[test]
    fn age_halves_every_entry() {
        let mut history = CutoffHistory::new();
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
        let mut history = CutoffHistory::new();
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

    // ---- History gravity ----
    //
    // `update`'s gravity term, exercised directly rather than only through
    // the saturation tests below (which pin the eventual ceiling, not how
    // a single bonus behaves on the way there).

    #[test]
    fn a_bonus_moves_a_near_ceiling_cell_less_than_a_near_zero_cell() {
        let mut near_ceiling = CutoffHistory::new();
        for _ in 0..50 {
            near_ceiling.record_cutoff(Color::White, Piece::Knight, Square::F3, 255);
        }
        let before = near_ceiling.score(Color::White, Piece::Knight, Square::F3);

        let mut near_zero = CutoffHistory::new();

        near_ceiling.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        near_zero.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        let near_ceiling_delta =
            near_ceiling.score(Color::White, Piece::Knight, Square::F3) - before;
        let near_zero_delta = near_zero.score(Color::White, Piece::Knight, Square::F3);

        assert!(
            near_ceiling_delta < near_zero_delta,
            "the same bonus (depth 4) must move a cell already near the ceiling ({before}) \
             less than a fresh cell at zero: near-ceiling moved {near_ceiling_delta}, \
             near-zero moved {near_zero_delta}"
        );
    }

    #[test]
    fn a_cutoff_bonus_never_decreases_a_cell_even_at_the_ceiling() {
        let mut history = CutoffHistory::new();
        for _ in 0..50 {
            history.record_cutoff(Color::White, Piece::Knight, Square::F3, 255);
        }
        let before = history.score(Color::White, Piece::Knight, Square::F3);

        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        assert!(
            history.score(Color::White, Piece::Knight, Square::F3) >= before,
            "gravity must only damp a same-direction bonus toward zero movement, never \
             reverse its direction: before {before}, after {}",
            history.score(Color::White, Piece::Knight, Square::F3)
        );
    }

    #[test]
    fn a_malus_never_increases_a_cell_even_at_the_floor() {
        let mut history = CutoffHistory::new();
        for _ in 0..50 {
            history.record_no_cutoff(Color::White, Piece::Knight, Square::F3, 255);
        }
        let before = history.score(Color::White, Piece::Knight, Square::F3);

        history.record_no_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        assert!(
            history.score(Color::White, Piece::Knight, Square::F3) <= before,
            "gravity must never reverse a same-direction malus's own direction: before \
             {before}, after {}",
            history.score(Color::White, Piece::Knight, Square::F3)
        );
    }

    /// The property that sets this formula apart from a plain
    /// distance-from-the-ceiling scale (which would damp a recovery from
    /// the opposite extreme exactly as hard as a same-direction raise):
    /// a cutoff on a cell deep in malus territory is a surprising
    /// reversal, and gravity's subtracted term adds to the raw bonus
    /// rather than damping it in that case, so the cell moves *more* than
    /// the same bonus would move a cell starting at zero.
    #[test]
    fn a_cutoff_moves_a_deeply_malused_cell_more_than_a_fresh_one() {
        let mut deeply_malused = CutoffHistory::new();
        for _ in 0..50 {
            deeply_malused.record_no_cutoff(Color::White, Piece::Knight, Square::F3, 255);
        }
        let before = deeply_malused.score(Color::White, Piece::Knight, Square::F3);

        let mut fresh = CutoffHistory::new();

        deeply_malused.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        fresh.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);

        let recovery_delta = deeply_malused.score(Color::White, Piece::Knight, Square::F3) - before;
        let fresh_delta = fresh.score(Color::White, Piece::Knight, Square::F3);

        assert!(
            recovery_delta > fresh_delta,
            "a cutoff reversing a deeply malused cell ({before}) must move it more than the \
             same bonus moves a fresh cell: reversal moved {recovery_delta}, fresh moved \
             {fresh_delta}"
        );
    }

    #[test]
    fn repeated_cutoffs_saturate_instead_of_overflowing() {
        let mut history = CutoffHistory::new();
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
        let mut history = CutoffHistory::new();
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
        let mut history = CutoffHistory::new();
        history.record_cutoff(Color::White, Piece::Knight, Square::F3, 4);
        history.record_no_cutoff(Color::Black, Piece::Pawn, Square::E5, 2);

        history.clear();

        assert_eq!(history.score(Color::White, Piece::Knight, Square::F3), 0);
        assert_eq!(history.score(Color::Black, Piece::Pawn, Square::E5), 0);
    }
}
