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
/// Two, so a ply can hold the refutation that worked and the one before it;
/// every array shaped by this resizes together if it changes.
pub(super) const KILLER_SLOTS: usize = 2;

/// Per-ply killer and mate-killer storage.
///
/// Both halves are indexed by ply and clamped the same way, so a ply past the
/// bound reads and writes the last slot rather than panicking. That is
/// reachable: quiescence's in-check evasion recursion is not depth-capped.
pub(super) struct KillerTable {
    /// Two killers per ply, most recent first.
    pub(super) slots: [[Option<Move>; KILLER_SLOTS]; MAX_TRACKED_PLY],
    /// One killer per ply that refuted a sibling with a mate-indicating score.
    ///
    /// One slot rather than two: a forced mate is rare enough at a fail-high
    /// that a second would mostly sit empty. Always-replace, which is where
    /// the two-slot promotion policy converges anyway.
    pub(super) mate: [Option<Move>; MAX_TRACKED_PLY],
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

    /// This ply's killer slots.
    pub(super) fn slots_at(&self, ply: u8) -> [Option<Move>; KILLER_SLOTS] {
        self.slots[Self::index(ply)]
    }

    /// This ply's mate killer, if it has one.
    pub(super) fn mate_at(&self, ply: u8) -> Option<Move> {
        self.mate[Self::index(ply)]
    }

    /// Whether `m` is already one of this ply's killers.
    pub(super) fn holds(&self, ply: u8, m: Move) -> bool {
        self.slots[Self::index(ply)].contains(&Some(m))
    }

    /// Whether `m` is already this ply's mate killer.
    pub(super) fn holds_mate(&self, ply: u8, m: Move) -> bool {
        self.mate[Self::index(ply)] == Some(m)
    }

    /// Records `m` as this ply's most recent killer.
    pub(super) fn record(&mut self, ply: u8, m: Move) {
        let idx = Self::index(ply);
        let slots = &mut self.slots[idx];
        let m = Some(m);
        // Only slot 0 is checked. A repeat of slot 1 takes the shift branch,
        // which promotes it to slot 0 and pushes the old slot 0 down, exactly
        // the behaviour a repeat should produce. Guarding slot 0 is what keeps
        // a repeat of it from duplicating into both.
        if slots[0] != m {
            slots[1] = slots[0];
            slots[0] = m;
        }
    }

    /// Records `m` as this ply's mate killer, replacing whatever was there.
    pub(super) fn record_mate(&mut self, ply: u8, m: Move) {
        self.mate[Self::index(ply)] = Some(m);
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
    fn recording_into_empty_slots_fills_the_first() {
        let mut table = KillerTable::new();
        table.record(0, first());
        assert_eq!(table.slots_at(0), [Some(first()), None]);
    }

    #[test]
    fn a_new_killer_shifts_the_previous_one_into_the_second_slot() {
        let mut table = KillerTable::new();
        table.record(0, first());
        table.record(0, second());
        assert_eq!(
            table.slots_at(0),
            [Some(second()), Some(first())],
            "the old slot 0 moves down rather than being discarded"
        );
    }

    #[test]
    fn a_repeat_of_the_first_slot_does_not_duplicate() {
        let mut table = KillerTable::new();
        table.record(0, first());
        table.record(0, second());
        table.record(0, second());
        assert_eq!(
            table.slots_at(0),
            [Some(second()), Some(first())],
            "recording slot 0's move again must not shift slot 1 out"
        );
    }

    #[test]
    fn a_repeat_of_the_second_slot_is_promoted_without_duplicating() {
        let mut table = KillerTable::new();
        table.record(0, first());
        table.record(0, second());
        table.record(0, first());
        assert_eq!(
            table.slots_at(0),
            [Some(first()), Some(second())],
            "a repeat of slot 1 becomes the new slot 0, not a duplicate in both"
        );
    }

    #[test]
    fn a_ply_past_the_bound_reads_and_writes_the_last_slot() {
        let mut table = KillerTable::new();
        let beyond = u8::MAX;
        table.record(beyond, first());
        table.record_mate(beyond, second());
        assert_eq!(table.slots_at(beyond), [Some(first()), None]);
        assert_eq!(table.mate_at(beyond), Some(second()));
    }

    #[test]
    fn plies_do_not_share_slots() {
        let mut table = KillerTable::new();
        table.record(3, first());
        assert_eq!(table.slots_at(3), [Some(first()), None]);
        assert_eq!(
            table.slots_at(4),
            [None, None],
            "a sibling ply is untouched"
        );
    }

    #[test]
    fn a_mate_killer_replaces_rather_than_shifts() {
        let mut table = KillerTable::new();
        table.record_mate(0, first());
        table.record_mate(0, second());
        assert_eq!(table.mate_at(0), Some(second()));
        assert!(table.holds_mate(0, second()));
        assert!(!table.holds_mate(0, first()));
    }
}
