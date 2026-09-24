//! Killer moves: quiet moves that caused a beta cutoff at a given ply, tried
//! early at sibling nodes on the bet that what refuted one sibling refutes the
//! next.
//!
//! Tree-scoped and owned by `Search` rather than the session, because a killer
//! is a refutation specific to this search tree's shape rather than a fact
//! about a position. See `docs/adr/0003-killer-table-owned-by-search-not-session.md`.

use super::MAX_TRACKED_PLY;
use turox_chess::types::Move;

/// Killer slots per ply.
///
/// Two, so a ply can hold the refutation that worked and the one before it.
const KILLER_SLOTS: usize = 2;

/// What a ply already held for a move, as of just before it was recorded.
pub(super) struct Held {
    /// The move was already one of this ply's killers.
    pub(super) killer: bool,
    /// The move was already this ply's mate killer.
    pub(super) mate_killer: bool,
}

/// Per-ply killer and mate-killer storage.
///
/// Indexed by ply and clamped, so a ply past the bound reads and writes the
/// last slot rather than panicking. That is reachable: quiescence's in-check
/// evasion recursion is not depth-capped.
pub(super) struct KillerTable {
    /// Two killers per ply, most recent first.
    slots: [[Option<Move>; KILLER_SLOTS]; MAX_TRACKED_PLY],
    /// One killer per ply that refuted a sibling with a mate-indicating score.
    ///
    /// One slot rather than two: a forced mate is rare enough at a fail-high
    /// that a second would mostly sit empty. Always-replace, which is where
    /// the two-slot promotion policy converges anyway.
    mate: [Option<Move>; MAX_TRACKED_PLY],
}

impl KillerTable {
    /// An empty table.
    pub(super) const fn new() -> Self {
        Self {
            slots: [[None; KILLER_SLOTS]; MAX_TRACKED_PLY],
            mate: [None; MAX_TRACKED_PLY],
        }
    }

    /// The ply index, clamped to the table's bound.
    fn index(ply: u8) -> usize {
        usize::from(ply).min(MAX_TRACKED_PLY - 1)
    }

    /// Whether `m` is one of this ply's killers.
    pub(super) fn holds(&self, ply: u8, m: Move) -> bool {
        self.slots[Self::index(ply)].contains(&Some(m))
    }

    /// Whether `m` is this ply's mate killer.
    pub(super) fn holds_mate(&self, ply: u8, m: Move) -> bool {
        self.mate[Self::index(ply)] == Some(m)
    }

    /// Records `m` as this ply's most recent killer, and as its mate killer
    /// too when `mate_score`, reporting what the ply already held.
    ///
    /// One call rather than a query and then a write, because the answer is
    /// only meaningful taken before the write. Leaving that order to each
    /// caller is how a cutoff ends up crediting itself.
    pub(super) fn record(&mut self, ply: u8, m: Move, mate_score: bool) -> Held {
        let idx = Self::index(ply);
        let held = Held {
            killer: self.slots[idx].contains(&Some(m)),
            mate_killer: self.mate[idx] == Some(m),
        };

        let slots = &mut self.slots[idx];
        let m = Some(m);
        // Only slot 0 is checked. A repeat of slot 1 takes the shift branch,
        // which promotes it and pushes the old slot 0 down, exactly what a
        // repeat should do. Guarding slot 0 keeps a repeat of it from
        // duplicating into both.
        if slots[0] != m {
            slots[1] = slots[0];
            slots[0] = m;
        }
        if mate_score {
            self.mate[idx] = m;
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use turox_chess::types::{MoveFlags, Square};

    /// Any two distinct quiet moves; the table never inspects legality.
    fn first() -> Move {
        Move::new(Square::E1, Square::D1, MoveFlags::Quiet)
    }

    fn second() -> Move {
        Move::new(Square::E1, Square::F1, MoveFlags::Quiet)
    }

    #[test]
    fn recording_into_an_empty_ply_holds_nothing_first() {
        let mut table = KillerTable::new();
        let held = table.record(0, first(), false);
        assert!(!held.killer, "nothing was there to have predicted it");
        assert!(!held.mate_killer);
        assert!(table.holds(0, first()));
    }

    #[test]
    fn a_ply_holds_two_killers_at_once() {
        let mut table = KillerTable::new();
        table.record(0, first(), false);
        table.record(0, second(), false);
        assert!(
            table.holds(0, first()),
            "the older killer is kept, not discarded"
        );
        assert!(table.holds(0, second()));
    }

    #[test]
    fn a_third_killer_evicts_the_oldest() {
        let mut table = KillerTable::new();
        let third = Move::new(Square::E1, Square::E2, MoveFlags::Quiet);
        table.record(0, first(), false);
        table.record(0, second(), false);
        table.record(0, third, false);
        assert!(!table.holds(0, first()), "two slots, so the oldest goes");
        assert!(table.holds(0, second()));
        assert!(table.holds(0, third));
    }

    #[test]
    fn a_repeat_reports_that_the_ply_already_held_it() {
        let mut table = KillerTable::new();
        table.record(0, first(), false);
        let held = table.record(0, first(), false);
        assert!(
            held.killer,
            "the answer is taken before the write, not after"
        );
    }

    #[test]
    fn a_repeat_of_the_older_killer_does_not_evict_the_newer_one() {
        let mut table = KillerTable::new();
        table.record(0, first(), false);
        table.record(0, second(), false);
        table.record(0, first(), false);
        assert!(
            table.holds(0, first()) && table.holds(0, second()),
            "promoting a move already held must not duplicate it into both slots \
             and push the other out"
        );
    }

    #[test]
    fn a_mate_killer_is_also_an_ordinary_killer() {
        let mut table = KillerTable::new();
        table.record(0, first(), true);
        assert!(table.holds_mate(0, first()));
        assert!(table.holds(0, first()), "a mate cutoff is a cutoff");
    }

    #[test]
    fn the_mate_slot_replaces_rather_than_shifts() {
        let mut table = KillerTable::new();
        table.record(0, first(), true);
        table.record(0, second(), true);
        assert!(table.holds_mate(0, second()));
        assert!(!table.holds_mate(0, first()), "one slot, always-replace");
    }

    #[test]
    fn an_ordinary_score_leaves_the_mate_slot_alone() {
        let mut table = KillerTable::new();
        table.record(0, first(), true);
        table.record(0, second(), false);
        assert!(
            table.holds_mate(0, first()),
            "only a mate score writes that slot"
        );
    }

    #[test]
    fn plies_do_not_share_slots() {
        let mut table = KillerTable::new();
        table.record(3, first(), true);
        assert!(!table.holds(4, first()), "a sibling ply is untouched");
        assert!(!table.holds_mate(4, first()));
    }

    #[test]
    fn a_ply_past_the_bound_is_clamped_rather_than_a_panic() {
        let mut table = KillerTable::new();
        table.record(u8::MAX, first(), true);
        assert!(table.holds(u8::MAX, first()));
        assert!(table.holds_mate(u8::MAX, first()));
    }
}
