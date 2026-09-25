//! The tiers a move is sorted into, and where each tier lands once sorted.
//!
//! Separate from the ordering pass itself because these are the vocabulary a
//! per-move policy reads: [`PriorityRuns::is_quiet`] is what late move
//! reductions ask, and what futility pruning and late move pruning will ask.

use std::ops::Range;
use turox_macros::Ordinal;

/// Ranked bottom to top, worst move first: `#[derive(PartialOrd, Ord)]` on the
/// enum compares by declaration order, so this declaration *is* the ranking,
/// not a lookup table alongside it. Declared worst-first, the reverse of how
/// it reads in prose, so a move's raw `Ord` already agrees with `Score`'s own
/// "bigger is better": [`MoveOrdering::move_priority`](super::MoveOrdering::move_priority) returns `(MovePriority,
/// Score)` with neither half wrapped in `Reverse`, and the one flip
/// `sort_unstable_by_key`'s ascending sort needs happens once, in
/// [`MoveOrdering::order`](super::MoveOrdering::order), instead of being smuggled into half the tuple.
/// Carries no payload of its own: `move_priority`'s tuple has a second element
/// for that, so the fine-grained tiebreak *within* a tier (MVV-LVA's delta
/// among captures, [`CutoffHistory`](super::history::CutoffHistory)'s score among quiets) has one shared
/// place to live rather than a separate payload per variant that needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Ordinal)]
#[repr(u8)]
pub(super) enum MovePriority {
    /// A capture losing material, ordered by how much. Below `Quiet` because a
    /// move that hangs a piece is worse than an untried ordinary move.
    LosingCapture,
    /// Everything not otherwise classified.
    Quiet,
    /// A quiet move that caused a beta cutoff at a sibling of this ply.
    Killer,
    /// A quiet move that refuted a sibling *with a mate score*. Ranked above
    /// ordinary killers because a forced mate is worth more than material.
    /// See [`MoveOrdering::on_cutoff`](super::MoveOrdering::on_cutoff).
    MateKiller,
    /// An even trade. Gain is exactly `0` by definition, so unlike the winning and losing
    /// tiers it needs no tiebreak beyond ordinary declaration order.
    EqualCapture,
    /// A capture winning material, ordered by how much; see [`MoveOrdering::move_priority`](super::MoveOrdering::move_priority)'s
    /// own doc for why the ordering value lives in the tuple this enum is half of, not a
    /// payload on the variant.
    WinningCapture,
    /// The transposition table's stored move for this position: already proved
    /// best by a deeper or equal search, so nothing cheaper predicts better.
    Hash,
    /// This ply's move in the previous iteration's completed best line (see
    /// `MoveOrdering::previous_pv_line`). Ranked above `Hash` even though the two
    /// usually agree: a hash-table entry for this exact position can belong to
    /// a different, more recently searched line if replacement has since
    /// overwritten it, where this search's own recorded line cannot.
    PrincipalVariation,
}

impl MovePriority {
    /// How many tiers there are, which is the width of a per-tier count.
    pub(super) const COUNT: usize = Self::ALL.len();

    /// This tier's position in the order [`MoveOrdering::order`](super::MoveOrdering::order) produces, best
    /// first. The inverse of `index`, which numbers by declaration and so runs
    /// worst first.
    pub(super) const fn rank(self) -> usize {
        Self::COUNT - 1 - self.index()
    }
}

/// Where each [`MovePriority`] begins and ends in an ordered move list.
///
/// Ordering sorts on the tier ahead of any tiebreak, so every tier occupies one
/// maximal contiguous run. That is what lets a per-move policy ask which class
/// a move belongs to from its index alone, rather than re-deriving a
/// classification the sort already made.
///
/// Bounds rather than a tier per move: the ranges are the whole of what a
/// caller reads, and a 256-entry array would ride on every frame of a deep
/// recursion to answer the same question. A reduction scaled by a move's own
/// history score would need the per-move form, and wanting that number is the
/// point at which widening becomes worth paying for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::search) struct PriorityRuns {
    /// The tier at rank `r` occupies `bounds[r]..bounds[r + 1]`, making this a
    /// prefix sum over the per-tier counts whose last element is the list's
    /// length. An absent tier is an empty range rather than a sentinel, which
    /// is what keeps every lookup total.
    bounds: [u16; MovePriority::COUNT + 1],
}

impl PriorityRuns {
    /// Builds the bounds from the number of moves in each tier, indexed by
    /// [`MovePriority::rank`]. The counts are the one extra thing an ordering
    /// pass has to collect, since it already visits every move once to compute
    /// its key.
    pub(super) fn from_counts(counts: &[u16; MovePriority::COUNT]) -> Self {
        let mut bounds = [0u16; MovePriority::COUNT + 1];
        let mut total = 0u16;
        for (slot, count) in bounds.iter_mut().skip(1).zip(counts) {
            total += *count;
            *slot = total;
        }
        Self { bounds }
    }

    /// The half-open index range `tier` occupies, empty if no move has it.
    pub(super) fn range(self, tier: MovePriority) -> Range<usize> {
        let rank = tier.rank();
        usize::from(self.bounds[rank])..usize::from(self.bounds[rank + 1])
    }

    /// Whether the move at `index` is quiet: not a capture, not a promotion,
    /// and not lifted above `Quiet` by the hash move, the previous principal
    /// variation or a killer. This is the predicate late move reductions,
    /// futility pruning and late move pruning all ask, and the reason this type
    /// exists at all.
    pub(in crate::search) fn is_quiet(self, index: usize) -> bool {
        self.range(MovePriority::Quiet).contains(&index)
    }
}
