//! What a finished search reports back.
//!
//! Separate from the search itself because every caller outside `search`
//! touches this and nothing else: `uci::session` formats it into `info` and
//! `bestmove` lines, and every test and bench reads its fields.

use crate::eval::Score;
use crate::search::ordering::stats::CutoffStats;
use std::time::Duration;
use turox_chess::types::Move;

/// Ply bound for [`PV`] and `Search::pv`, deliberately not [`MAX_TRACKED_PLY`]: that
/// constant's own justification is defensive slack for pathological recursion depth
/// (quiescence's uncapped in-check evasion chases), which has nothing to do with how
/// deep a *reported* principal variation can realistically go. A PV line's real ceiling
/// is `search_with_info`'s own iterative-deepening bound (`uci::session`'s
/// `DEFAULT_MAX_DEPTH` is `64`), so bounding it there instead saves the same factor of
/// `MAX_TRACKED_PLY / MAX_PV_PLY` squared on `Search::pv`, since that one is triangular
/// (`[PV; MAX_PV_PLY]`, one row per ply) rather than flat.
///
/// [`MAX_TRACKED_PLY`]: super::MAX_TRACKED_PLY
pub(in crate::search) const MAX_PV_PLY: usize = 64;

/// A principal variation, one move per ply starting from wherever it was read: `None`
/// past however deep the line actually runs, the same "untouched slot" convention
/// `Search`'s other per-ply tables carry. Sized to [`MAX_PV_PLY`], not
/// [`MAX_TRACKED_PLY`]; see that constant's own doc for why the two bounds differ.
///
/// [`MAX_TRACKED_PLY`]: super::MAX_TRACKED_PLY
pub type PV = [Option<Move>; MAX_PV_PLY];

/// One completed call to [`Search::search`]: the best move and score found, and the depth
/// actually reached.
///
/// `depth` can be less than the requested `max_depth` if the search was aborted
/// (deadline, node budget, or `request_stop`) before a deeper iteration finished; see
/// `Search::search`'s doc for why a partial iteration's own result is discarded rather
/// than returned. `depth` is `0` specifically when even the first iteration never
/// finished: there, `best_move` still carries the best move found among whatever moves
/// had already fully resolved before the abort, since that's the only case with no
/// earlier completed iteration to fall back on instead.
///
/// [`Search::search`]: super::Search::search
#[derive(Debug, Clone, Copy, Eq)]
pub struct SearchResult {
    /// The principal variation searched
    pub pv: PV,
    /// Side-to-move-relative, same convention as [`evaluate`](crate::eval::evaluate).
    pub score: Score,
    /// The depth actually completed; see the struct doc for when this is
    /// less than the requested `max_depth`.
    pub depth: u8,
    /// Total nodes visited (negamax and quiescence both count) across every
    /// completed and aborted iteration of this call.
    pub nodes: u64,
    /// Wall-clock time spent since [`Search::search`](super::Search::search)
    /// was entered, covering every iteration so far rather than just the one
    /// this result came from. Measured even when no deadline is set, since UCI
    /// reports it regardless of what bounded the search.
    pub time: Duration,
    /// Table occupancy in permille, or `None` when this search has no
    /// transposition table at all. `None` rather than `0` because the two mean
    /// different things to a GUI: an empty table and no table are not the same
    /// state, and only the former is worth reporting as `hashfull 0`.
    pub hashfull: Option<u16>,
    /// Cutoff-index histogram for `negamax`'s own move loop (`search_root`'s
    /// counts the same way), across every completed and aborted iteration so
    /// far, the same accumulation `nodes` uses.
    pub negamax_cutoffs: CutoffStats,
    /// Cutoff-index histogram for `quiescence`'s move loop, kept separate from
    /// `negamax_cutoffs`: quiescence's own list (captures, or evasions while in
    /// check) is a different, usually much shorter list than a main-search
    /// node's, and mixing the two would flatter or distort whichever one
    /// dominates the combined count.
    pub quiescence_cutoffs: CutoffStats,
}

impl SearchResult {
    /// `None` only when the position handed to `search` has no legal moves at all
    /// (checkmate or stalemate); every other case, including an aborted first
    /// iteration, still has a real move to report.
    #[must_use]
    pub const fn best_move(&self) -> Option<Move> {
        self.pv[0]
    }
}

/// Compares every field except `time`, mirroring how `Board` excludes its
/// Zobrist hash: elapsed wall-clock is an observation about one particular
/// run, not part of what a search *found*, so two runs that reached the same
/// move, score, depth and node count are the same result even though they
/// took different amounts of time.
///
/// Without this, `tests/search_props.rs`'s `search_is_deterministic` would
/// compare two timings and fail essentially always, and weakening that test
/// to dodge the problem would give up the check it exists for.
///
/// Destructured rather than read field by field off `self`, so that a field
/// added to `SearchResult` without a decision made here fails to compile
/// rather than going quietly uncompared and weakening every test that
/// compares two results.
impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            pv,
            score,
            depth,
            nodes,
            time: _,
            hashfull,
            negamax_cutoffs,
            quiescence_cutoffs,
        } = self;
        *pv == other.pv
            && *score == other.score
            && *depth == other.depth
            && *nodes == other.nodes
            && *hashfull == other.hashfull
            && *negamax_cutoffs == other.negamax_cutoffs
            && *quiescence_cutoffs == other.quiescence_cutoffs
    }
}
