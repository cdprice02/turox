//! How often each ordering technique earned its place.
//!
//! Reported on every UCI `info` line and asserted on directly by
//! `tests/search.rs`, which is the only check that a technique is doing
//! anything before an SPRT measures whether it wins games.

use turox_macros::Ordinal;

/// What produced a beta cutoff, for [`CutoffStats::by_cause`].
///
/// About *why the move was tried early*, not what kind of move it is: a capture
/// that cuts off because the transposition table named it is a hash hit, not a
/// capture hit. The question this answers is which ordering technique is
/// earning its place.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Ordinal)]
#[repr(u8)]
pub enum CutoffCause {
    /// The transposition table's stored move for this position.
    HashMove,
    /// A quiet move already sitting in a killer slot for this ply.
    Killer,
    /// A quiet move already sitting in this ply's mate-killer slot: it
    /// refuted an earlier sibling with a mate-indicating score.
    MateKiller,
    /// Anything else: ordinary move ordering got there without a technique
    /// tracked here claiming credit.
    Other,
}

/// Which move index took a beta cutoff, the standard diagnostic for move-ordering quality.
///
/// A well-ordered search takes most of its cutoffs on the first move tried (index 0),
/// since that's what alpha-beta pruning is actually trying to arrange.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CutoffStats {
    /// Nodes whose move loop hit a beta cutoff (`alpha >= beta`) at all.
    pub fail_high_nodes: u64,
    /// Cutoffs per [`CutoffCause`], indexed by [`CutoffCause::index`].
    ///
    /// Answers which ordering technique actually paid off, as distinct from
    /// `fail_high_nodes` and `cutoff_index`, which measure ordering quality
    /// overall and say nothing about what produced it. Summing this always
    /// equals `fail_high_nodes`, the same invariant `cutoff_index` has.
    ///
    /// An array keyed by an enum rather than a counter field per technique:
    /// each new ordering technique that wants its own hit rate adds a variant,
    /// not a field here plus another `bool` parameter on `record`.
    pub by_cause: [u64; CutoffCause::COUNT],
    /// `cutoff_index[i]` counts cutoffs at move index `i`, for `i < 15`. Index `15` is an
    /// overflow bucket for the 16th move onward, so a long tail of rare late cutoffs can't
    /// make this array itself unbounded; summing the whole array always equals
    /// `fail_high_nodes`.
    pub cutoff_index: [u64; 16],
}

impl CutoffStats {
    /// Records a cutoff at `index` (0-based position in the already-ordered
    /// move list), attributed to `cause`.
    pub(in crate::search) fn record(&mut self, index: usize, cause: CutoffCause) {
        self.fail_high_nodes += 1;
        self.cutoff_index[index.min(15)] += 1;
        self.by_cause[cause.index()] += 1;
    }
}
