//! `Search`: negamax with fail-soft alpha-beta over iterative deepening,
//! quiescence search at the horizon, and MVV-LVA capture ordering.
//!
//! Mirrors `move_gen`'s naive-reference discipline: there's no perft
//! equivalent for search, so `tests/search_props.rs` checks this against an
//! independent, unpruned negamax reference rather than trusting a
//! read-through.

use crate::eval::{evaluate, Score};
use crate::search::draw::{is_draw, is_fifty_move_draw, is_threefold_repetition};
use crate::search::ordering::history::CutoffHistory;
use crate::search::ordering::stats::CutoffStats;
use crate::search::ordering::MoveOrdering;
use crate::search::result::{SearchResult, MAX_PV_PLY, PV};
use crate::search::selectivity::lmr;
use crate::search::time::should_skip_next_iteration;
use crate::search::tt::Tt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use turox_chess::board::Board;
use turox_chess::move_gen::attacks::in_check;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::move_gen::move_list::MoveList;
use turox_chess::types::Move;
use turox_rng::xorshift64star;

/// The score magnitude of a certain checkmate.
///
/// A node where the side to move has no legal moves and is in check scores
/// `Score::from(ply) - MATE`: very negative, and more negative the *smaller* `ply` is (a mate
/// reached in fewer plies from the root is a faster, more forced mate against that side).
/// One ply up, negamax's sign flip turns that into `MATE - ply` for the side that just
/// delivered it, so a shorter forced mate always outscores a longer one. This exact ply
/// direction is a classic place for an off-by-one to hide silently and look plausible;
/// `tests/search_props.rs`'s mate puzzles are what actually pin the formula down, not
/// this comment.
pub const MATE: Score = 30_000;

/// The widest distance-from-root a mate score can carry.
///
/// Comfortably above any ply a real search reaches (iterative deepening stops
/// at 64, plus a bounded quiescence tail) and far above anything the static
/// evaluation terms can produce, so "is this a mate" never has to guess. The
/// margin only has to separate the two ranges, not be tight.
pub const MAX_MATE_PLY: Score = 512;

/// Whether `score` encodes a forced mate rather than an ordinary evaluation.
///
/// The single definition of that boundary. It used to be answered in two
/// places with two different thresholds: the transposition table's ply
/// adjustment and the UCI layer's `mate` reporting each had their own idea of
/// how close to [`MATE`] counted, which is exactly the kind of drift that lets
/// a score be treated as a mate by one and as a centipawn value by the other.
///
/// `saturating_abs` rather than `abs`: [`Score::MIN`] has no positive
/// counterpart and would overflow.
#[must_use]
pub const fn is_mate_score(score: Score) -> bool {
    score.saturating_abs() >= MATE - MAX_MATE_PLY
}

/// How many plies past `negamax`'s horizon `quiescence` is allowed to keep resolving
/// captures and promotions before it gives up and falls back to stand pat, same as if
/// none were available.
///
/// Without a cap, a sufficiently tangled position (many mutually-en-prise pieces) can
/// make the *breadth* of the capture tree blow up long before it naturally bottoms out on
/// material alone: `tests/search_props.rs`'s `alpha_beta_agrees_with_unpruned_negamax`
/// property test hit exactly this on an `any_board()`-generated position, burning minutes
/// of CPU on a single case. A handful of plies is typical; too low and captures get cut
/// off mid-exchange again, just one horizon further out, defeating quiescence's own
/// purpose.
///
/// `pub`, not private: `tests/search_props.rs`'s own `naive_quiescence` oracle needs the
/// identical cap, not a hand-copied literal that could drift out of sync and silently
/// turn the property test into a comparison between two different search depths.
pub const MAX_QUIESCENCE_DEPTH: u8 = 8;

/// What [`Search::search_root`] found for one depth: either the full move loop finished,
/// or an abort cut it short partway through.
enum RootOutcome {
    /// The move loop finished every move at this depth. `best_move` is `None` only for
    /// the genuine terminal case: no legal moves at all.
    Completed(Score, PV),
    /// The abort hit before every move at this depth could be tried. `best_so_far` is
    /// the best move and score among moves whose subtree had already fully resolved
    /// before the interruption, if any had; `None` when the abort landed before the
    /// move loop could report anything (the top-of-function check, or the depth-0
    /// quiescence-only path, which has no per-move loop to have made progress in).
    /// If `Some`, the `[Option<Move>; ...]` should have at least one move.
    Aborted {
        /// See this variant's own doc above.
        best_so_far: Option<(Score, PV)>,
    },
}

/// The non-move inputs to one [`Search::alpha_beta_loop`] call.
///
/// A struct rather than four more parameters: the loop already takes a
/// closure and a move list, and the pruning techniques queued behind this
/// seam (late move reductions, futility margins) each want to add another
/// knob here. Growing a named struct is the difference between that staying
/// readable and the signature becoming positional guesswork.
///
/// Negamax-only now: quiescence's two loops stopped sharing this one once
/// PVS's windowing and PV bookkeeping became something only `negamax` and
/// `search_root` need (see [`Search::quiescence_loop`] for quiescence's own,
/// much simpler copy). `initial_max` and the `CutoffSink` enum this struct
/// used to carry both existed only to keep that sharing working: `initial_max`
/// was quiescence's capture loop threading `stand_pat` in as its floor, and
/// `CutoffSink` was choosing which histogram to record to. Neither has a
/// reason to exist now that there is exactly one caller shape and exactly one
/// histogram (`self.negamax_cutoffs`).
///
/// `Copy` because every field is: it is passed by value so the loop can
/// destructure and shadow `alpha` without the caller losing its own copy,
/// which is exactly what `negamax` relies on to keep `original_alpha`.
#[derive(Clone, Copy)]
struct LoopCtx {
    /// Distance from the root, for the killer table, the mate formula, and
    /// indexing [`Search::pv`]'s row for this node.
    ply: u8,
    /// Lower bound on entry. The loop raises its own copy as moves improve on
    /// it; the caller's value is untouched, which is what lets `negamax` keep
    /// the original for its transposition-table bound classification.
    alpha: Score,
    /// Upper bound. Never modified: a cutoff is `alpha >= beta`.
    beta: Score,
    /// This node's remaining depth. The loop searches children at `depth - 1`
    /// normally, and shallower when a [`Verdict::Reduce`] asks it to, which is
    /// why the loop needs the number rather than leaving it captured by the
    /// caller's closure.
    depth: u8,
}

impl LoopCtx {
    /// Whether this is a principal variation node: a null window can only ever
    /// prove a bound, so a node searched with one is not on the line.
    ///
    /// Rests on every wide window in the search belonging to a PV node and
    /// every null window not. A technique that searched a PV node with a null
    /// window, or a non-PV node with a wide one, would break this silently.
    const fn is_pv(self) -> bool {
        self.beta > self.alpha + 1
    }
}

/// What the caller decides about one move, asked as the loop reaches it.
///
/// The loop owns the mechanism (windowing, the re-searches, the bookkeeping)
/// and this is the policy: which moves are worth what. Every pruning and
/// reduction technique in the backlog is expressible as one of these four,
/// which is the reason the seam has this shape rather than a wider closure
/// signature that grows a parameter per technique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Only outside `cfg(test)`: the tests below construct every variant to prove
// the loop handles it, which is the point of writing them before any policy
// exists. Late move reductions construct `Reduce`; `Skip` and `Stop` wait on
// futility pruning and late move pruning, and this expectation fails the
// build once both arrive.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "futility pruning and late move pruning are what construct Skip and Stop; the loop's handling of both is already tested"
    )
)]
enum Verdict {
    /// Search it normally, at full depth.
    Search,
    /// Search it `n` plies shallower, and re-search at full depth if the
    /// shallow result beats alpha. Late move reductions.
    Reduce(u8),
    /// Skip it entirely. Futility pruning.
    Skip,
    /// Stop the loop here, searching nothing further. Late move pruning.
    Stop,
}

/// What [`Verdict`] is decided against.
// `index` and `searched` are read by late move reductions; `alpha` is the one
// field still waiting for a reader.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "futility pruning is what reads alpha, since its condition needs the alpha the loop currently holds"
    )
)]
struct MoveCtx {
    /// This move's index in the list, which is what a reduction table and a
    /// late-move-pruning threshold are both indexed by.
    index: usize,
    /// Alpha *as the loop currently holds it*, raised by every move already
    /// searched. Futility's condition reads this, which is why the decision
    /// cannot be hoisted above the loop: the entry alpha would prune too
    /// little, and a stale one too much.
    alpha: Score,
    /// How many moves have actually been searched, as opposed to reached.
    /// Differs from `index` as soon as anything is skipped.
    searched: usize,
}

/// What [`Search::negamax`] and [`Search::quiescence`] both return: the
/// fail-soft score, and whether the subtree that produced it passed
/// through a threefold-repetition draw anywhere, in either function (a
/// repetition reachable only through a forced sequence of quiescence
/// evasions taints exactly the same way one reachable through `negamax`'s
/// own move loop does).
///
/// A tainted score is still correct to use for comparisons, cutoffs, and PV
/// bookkeeping at the calling node: what a tainted score is *not* safe for is
/// a transposition-table store, since the table is keyed on board state
/// alone and a repetition's availability depends on the path taken to reach
/// it. `negamax` checks this flag itself, immediately before its own store
/// (`quiescence` has no store of its own to guard: see its own doc), and
/// every caller that goes on to store anything of its own must OR its
/// children's `tainted` into its own before doing the same.
#[derive(Debug, Clone, Copy)]
struct TaintedScore {
    /// Fail-soft: the real best score found, not a clamped bound.
    score: Score,
    /// Whether some node in the subtree that produced `score` was a
    /// threefold-repetition draw. See this struct's own doc for what that
    /// does and doesn't license a caller to do with `score`.
    tainted: bool,
}

/// What one [`Search::alpha_beta_loop`] call produced.
struct LoopOutcome {
    /// Fail-soft: the real best score found, not a clamped bound.
    max: Score,
    /// The move that produced `max`, `None` if no move improved on
    /// `initial_max`. Quiescence computes this and ignores it, which costs an
    /// `Option<Move>` write per improvement and keeps one loop instead of two.
    best_move: Option<Move>,
    /// Whether `search_child` returned `None` partway through. `max` and
    /// `best_move` then describe the moves that *did* resolve, which is what
    /// the root reports as its partial result; every other caller propagates
    /// the abort instead.
    aborted: bool,
    /// The move index a beta cutoff broke on, `None` if the loop ran to completion
    /// without one. Callers feed this straight into [`MoveOrdering::update_history`],
    /// which needs to know not just *whether* a cutoff happened (that's `best_move`
    /// improving past `initial_max`) but exactly which prefix of `moves` was tried before
    /// it, malus-worthy quiets and all.
    cutoff_index: Option<usize>,
}

/// Mutable search state threaded through one [`Search::search`] call: the node counter
/// and abort conditions the periodic check reads, and the repetition hash stack.
///
/// A struct rather than several parameters threaded through the recursion is what makes
/// the abort check and the draw check cheap to reach at every node.
///
/// `'a` is only ever used by `tt`: everything else here is owned outright. A `Search`
/// with no [`Search::with_tt`] call never actually borrows anything, so `'a` costs
/// nothing at most call sites; Rust infers it from context the same way it always does
/// for an unused generic parameter.
pub struct Search<'a> {
    /// Every node visited by this search, across all iterative-deepening
    /// iterations. Drives `max_nodes` as well as being reported.
    nodes: u64,
    /// Hashes of every position on the path leading up to (but not
    /// including) the position currently being searched, per
    /// [`crate::search::draw::is_threefold_repetition`]'s contract. Seeded by the caller
    /// with real game history, so repetitions that already happened in the
    /// actual game are visible, not just ones the search tree itself
    /// revisits. Grows by one push per ply descended, shrinks by one pop on
    /// backtrack: it must hold a node's own hash while searching that
    /// node's children, but never the node's own hash while checking the
    /// node itself.
    history: Vec<u64>,
    /// Wall-clock abort point, checked periodically rather than per node. See
    /// `max_nodes` for the deterministic counterpart used by tests.
    deadline: Option<Instant>,
    /// A deterministic alternative to `deadline`: aborts once `nodes`
    /// reaches this count. A wall-clock deadline makes "iterative deepening
    /// was interrupted partway through a deeper iteration" untestable
    /// without a flaky sleep; a node budget makes it exact and repeatable
    /// (see `tests/search_props.rs`).
    max_nodes: Option<u64>,
    /// `Arc`, not a plain `bool`: UCI's `stop` command arrives on a
    /// different thread than the one running `search` (the reader thread
    /// parsing stdin, while the main thread blocks inside `search`), so
    /// setting it has to be visible across threads without
    /// `Search` itself being shared. [`Search::stop_flag`] hands out a
    /// clone of this same `Arc` before a caller moves `Search` onto its own
    /// search thread, so the main thread can still set it later.
    stop: Arc<AtomicBool>,
    /// Every table that decides which move this search tries first, and the
    /// pass that reads them. One field so that a new ordering technique lands
    /// entirely inside `ordering`; see that module.
    ordering: MoveOrdering<'a>,
    /// `None` by default (`negamax` searches with no transposition table at all, the same
    /// as before one existed); set via [`Search::with_tt`]. Borrowed, not owned: the table
    /// lives in `uci::session::run` (like `history` conceptually does, though `history` is
    /// actually copied in), so it survives across the separate `Search` a later `go` call
    /// rebuilds, rather than starting cold every time.
    tt: Option<&'a mut Tt>,
    /// The triangular PV table.
    pv: [PV; MAX_PV_PLY],
    /// xorshift64* state when root move randomization is on, `None` when it is
    /// off (the default, so every existing test and bench stays deterministic
    /// without knowing this exists).
    ///
    /// Only the *root* move list is shuffled. Interior nodes stay deterministic
    /// because shuffling them would fight move ordering, which is the single
    /// biggest lever on search efficiency this engine has.
    root_rng: Option<u64>,
    /// Accumulates across `search_root` and `negamax`'s move loops; see
    /// [`CutoffStats`] and [`SearchResult::negamax_cutoffs`].
    negamax_cutoffs: CutoffStats,
    /// Accumulates across `quiescence`'s move loop (captures and evasions
    /// both); see [`CutoffStats`] and [`SearchResult::quiescence_cutoffs`].
    quiescence_cutoffs: CutoffStats,
}

impl<'a> Search<'a> {
    /// A new search seeded with `history`: the real game's position hashes
    /// so far, not including the position `search` will be called on. No
    /// deadline or node budget by default, so `search` runs every
    /// iteration up to `max_depth` to completion; see `with_deadline`/
    /// `with_max_nodes` to bound it.
    #[must_use]
    pub fn new(history: Vec<u64>) -> Self {
        Self {
            nodes: 0,
            history,
            deadline: None,
            max_nodes: None,
            stop: Arc::new(AtomicBool::new(false)),
            ordering: MoveOrdering::new(),
            tt: None,
            pv: [[None; MAX_PV_PLY]; MAX_PV_PLY],
            root_rng: None,
            negamax_cutoffs: CutoffStats::default(),
            quiescence_cutoffs: CutoffStats::default(),
        }
    }

    /// Breaks ties between equally-good root moves at random instead of always
    /// taking the first one generated, so the same position does not produce
    /// the same game every time.
    ///
    /// `seed` is forced nonzero: xorshift64* maps zero to zero forever, so a
    /// zero seed would silently disable the shuffle rather than fail loudly.
    ///
    /// Off by default. Search results are otherwise reproducible, and several
    /// tests depend on that, so this is something a caller opts into (the UCI
    /// session does) rather than something they have to opt out of.
    #[must_use]
    pub const fn with_root_randomization(mut self, seed: u64) -> Self {
        self.root_rng = Some(if seed == 0 { 1 } else { seed });
        self
    }

    /// Fisher-Yates over the root move list, so every permutation is equally
    /// likely. A cheaper "swap two random entries" would bias the result toward
    /// the original order, which is the order this exists to stop depending on.
    fn shuffle_root_moves(&mut self, moves: &mut MoveList) {
        let Some(state) = self.root_rng.as_mut() else {
            return;
        };
        let slice = moves.as_mut_slice();
        for i in (1..slice.len()).rev() {
            *state = xorshift64star(*state);
            let span = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
            let j = usize::try_from(*state % span).unwrap_or(0);
            slice.swap(i, j);
        }
    }

    /// Aborts the search once `nodes` reaches `max_nodes`, checked on the
    /// same periodic schedule `deadline` and `stop` are.
    #[must_use]
    pub const fn with_max_nodes(mut self, max_nodes: u64) -> Self {
        self.max_nodes = Some(max_nodes);
        self
    }

    /// Aborts the search once `Instant::now()` passes `deadline`, checked
    /// on the same periodic schedule `max_nodes` and `stop` are.
    #[must_use]
    pub const fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Uses `stop` as this search's stop flag instead of the fresh one
    /// [`Search::new`] creates. For the UCI loop's `stop`/`quit` handling
    /// specifically: the loop needs to reach into a search that's actively
    /// blocking the thread that would otherwise poll for more commands, so
    /// it has to hold onto (and be able to set) the exact same `Arc` the
    /// search itself checks, not just a fresh one of its own.
    #[must_use]
    pub fn with_stop_flag(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop = stop;
        self
    }

    /// Gives this search a transposition table to probe/store against in `negamax`.
    /// Without this, `negamax` searches exactly as if `tt` didn't exist: probing and
    /// storing are both no-ops when `self.tt` is `None`, not an error condition.
    #[must_use]
    pub const fn with_tt(mut self, tt: &'a mut Tt) -> Self {
        self.tt = Some(tt);
        self
    }

    /// Gives this search a cutoff history table to read during move ordering and update
    /// after every `search_root`/`negamax`/quiescence-evasion move loop. Without this,
    /// quiet moves beyond the killer table's two slots are ordered and left unrecorded
    /// exactly as if history didn't exist: a no-op, not an error condition, the same
    /// convention `with_tt`'s absence carries.
    #[must_use]
    pub const fn with_cutoff_history(mut self, cutoff_history: &'a mut CutoffHistory) -> Self {
        self.ordering.set_cutoff_history(cutoff_history);
        self
    }

    /// Requests that the search stop as soon as it's next checked (a plain
    /// `Relaxed` store: `should_abort` only ever needs to see the flag
    /// eventually, not synchronize any other memory with it). Equivalent to
    /// setting the handle from [`Search::stop_flag`] directly; this is the
    /// convenience version for callers on the same thread as the search.
    pub fn request_stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// A shareable handle to this search's stop flag: clone it out *before*
    /// moving `Search` onto its own search thread, and a caller on another
    /// thread (UCI's `stop` command handler, running on the reader thread)
    /// can still set it via `Ordering::Relaxed` `store`, honored the next time
    /// `should_abort` checks (same `nodes & 2047` schedule as `deadline`
    /// and `max_nodes`).
    #[must_use]
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    /// Total nodes visited so far by this `Search`.
    #[must_use]
    pub const fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Whether the search should abort right now. Only actually reads
    /// `stop`/`deadline`/`max_nodes` every 2048th node
    /// (`self.nodes & 2047 == 0`), so the clock read (and even the branch)
    /// is amortized to roughly nothing per node rather than paid on every
    /// single one. Call this immediately after incrementing `self.nodes` at
    /// the top of `negamax`/`quiescence`.
    #[expect(
        clippy::verbose_bit_mask,
        reason = "the mask form is the standard periodic-check idiom, not an oversight; `trailing_zeros() >= 11` would desync from this fn's own doc comment for no clarity gain"
    )]
    fn should_abort(&self) -> bool {
        self.nodes & 2047 == 0
            && (self.stop.load(Ordering::Relaxed)
                || self.deadline.is_some_and(|d| Instant::now() >= d)
                || self.max_nodes.is_some_and(|max| self.nodes >= max))
    }

    /// Iterative deepening driver: repeatedly searches the root at
    /// increasing depths, keeping only the result of the last iteration
    /// that ran to completion. Delegates to [`Search::search_with_info`]
    /// with a no-op callback; see that method to observe each iteration's
    /// result as it lands rather than only the final one.
    ///
    /// An iteration interrupted partway through (`search_root` aborts)
    /// must not overwrite the previous iteration's result; returning a
    /// partial iteration's in-progress best move as if it were a completed
    /// depth is the classic bug here. But when the *first* iteration
    /// aborts, there is no previous completed iteration to fall back on,
    /// and a position with legal moves must never report `best_move: None`
    /// regardless; see `search_with_info`'s handling of that case. Before
    /// starting each iteration past the first, [`should_skip_next_iteration`]
    /// may also stop the loop early rather than start a doomed one; see its
    /// own doc.
    pub fn search(&mut self, board: &Board, max_depth: u8) -> SearchResult {
        self.search_with_info(board, max_depth, |_| {})
    }

    /// Same iterative deepening driver as [`Search::search`], but calls
    /// `on_iteration_complete` with the result of every iteration that
    /// runs to completion, not just the last one; `search` is this method
    /// with a no-op callback. Kept as a separate method (rather than
    /// threading an `Option<impl FnMut>` through `search` itself) so
    /// existing callers that only want a final result (`benches/search.rs`,
    /// most of `tests/search_props.rs`) don't need to know callbacks exist.
    ///
    /// `on_iteration_complete` only fires for iterations that actually ran
    /// to completion. If depth 1 itself aborts, `result` falls back to
    /// whatever best move `search_root` had already resolved before the
    /// abort (see `RootOutcome::Aborted`, a private implementation detail
    /// of `search_root` rather than a linkable public item) instead of the
    /// zeroed-out initial value, since a position with legal moves must
    /// never report `best_move: None`; that fallback is reported at
    /// `depth: 0`, since depth 1 didn't actually finish, and does not
    /// reach the callback.
    pub fn search_with_info<F: FnMut(&SearchResult)>(
        &mut self,
        board: &Board,
        max_depth: u8,
        mut on_iteration_complete: F,
    ) -> SearchResult {
        let started = Instant::now();
        let mut result = SearchResult {
            pv: [None; MAX_PV_PLY],
            score: 0,
            depth: 0,
            nodes: 0,
            time: Duration::ZERO,
            hashfull: None,
            negamax_cutoffs: CutoffStats::default(),
            quiescence_cutoffs: CutoffStats::default(),
        };
        // Only tracked when `self.deadline` is set: a `max_nodes`-bounded or
        // fully unbounded search has nothing to estimate against, so these
        // stay `None` the whole loop and the soft-limit check below never
        // fires, leaving that path byte-for-byte unaffected by this check.
        let mut previous_iteration_elapsed: Option<Duration> = None;
        // This iteration's and the one before it's own node count (the
        // growth `should_skip_next_iteration` measures a ratio from), not
        // `self.nodes()`'s running total across the whole search.
        let mut previous_iteration_nodes: Option<u64> = None;
        let mut nodes_before_previous_iteration: Option<u64> = None;

        for depth in 1..=max_depth {
            let nodes_before_this_iteration = self.nodes();

            if let (Some(deadline), Some(elapsed), Some(nodes_last)) = (
                self.deadline,
                previous_iteration_elapsed,
                previous_iteration_nodes,
            ) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if should_skip_next_iteration(
                    elapsed,
                    nodes_last,
                    nodes_before_previous_iteration,
                    remaining,
                ) {
                    break;
                }
            }

            let iteration_started = self.deadline.is_some().then(Instant::now);
            match self.search_root(board, depth) {
                RootOutcome::Completed(score, pv) => {
                    result = SearchResult {
                        pv,
                        score,
                        depth,
                        nodes: self.nodes(),
                        time: started.elapsed(),
                        hashfull: self.tt.as_deref().map(Tt::hashfull),
                        negamax_cutoffs: self.negamax_cutoffs,
                        quiescence_cutoffs: self.quiescence_cutoffs,
                    };
                    on_iteration_complete(&result);
                }
                RootOutcome::Aborted { best_so_far } => {
                    // Only depth 1 aborting can reach here with `result.pv`
                    // still full of `None`: every later depth has a completed iteration
                    // already sitting in `result` to fall back on instead, per
                    // this method's own doc.
                    if result.best_move().is_none() {
                        if let Some((score, pv)) = best_so_far {
                            result = SearchResult {
                                pv,
                                score,
                                depth: 0,
                                nodes: self.nodes(),
                                time: started.elapsed(),
                                hashfull: self.tt.as_deref().map(Tt::hashfull),
                                negamax_cutoffs: self.negamax_cutoffs,
                                quiescence_cutoffs: self.quiescence_cutoffs,
                            };
                        }
                    }
                    break;
                }
            }
            if let Some(started) = iteration_started {
                previous_iteration_elapsed = Some(started.elapsed());
                nodes_before_previous_iteration = previous_iteration_nodes;
                previous_iteration_nodes = Some(self.nodes() - nodes_before_this_iteration);
            }
        }
        result
    }

    /// The alpha-beta move loop shared by `negamax` and `search_root` (see
    /// `search_root`'s own doc for the handful of things it does differently
    /// around this call, not within it). Quiescence's two loops used to share
    /// this too; they now have their own, much simpler
    /// [`Search::quiescence_loop`], because PVS's windowing and PV bookkeeping
    /// are `negamax`/`search_root`-only (see `LoopCtx`'s own doc for why the
    /// split), and forcing both shapes through one generic function was
    /// costing more in accreted, per-caller branching than the sharing was
    /// buying.
    ///
    /// `search_child` receives `&mut Self` rather than capturing it, which is
    /// what lets a closure own the parts that genuinely differ (which ply the
    /// recursive call is at, and whether the repetition stack is maintained
    /// across it) while the loop keeps the parts that must not: fail-soft
    /// score comparison, alpha raising, the cutoff test, and recording the
    /// cutoff to the killer table and the histogram. The window is passed in
    /// already negated, so the caller never repeats negamax's sign convention
    /// either. The closure's `bool` parameter is `is_pv` for *that specific
    /// call*: move 0 gets `ctx.is_pv` itself (a PV node's first move stays on
    /// the principal variation; a cut node's first move was never on it to
    /// begin with), every other move's initial probe gets `false` (a null
    /// window can only ever prove a bound, never hand back an exact score
    /// worth recording), and a re-search (see below) gets `true` regardless of
    /// `ctx.is_pv`, since a move that just proved itself better than
    /// everything else found so far is a live candidate for the real line.
    ///
    /// The null-window probe and its possible re-search: for every move after
    /// the first, `search_child` first runs with a one-point window just
    /// above the current `alpha` (`-alpha - 1, -alpha`). That can only tell
    /// you "at most alpha" or "better than alpha, exact value unknown" -- so
    /// if, once negated back into this node's own frame, the probe's score
    /// lands strictly between `alpha` and `beta`, it's re-run with the real
    /// window to get the precise value. A probe failing high past `beta` is
    /// already exact (fail-soft returns the real score found, not a clamped
    /// bound) and would have caused the same cutoff on a full-window search
    /// too, so it's trusted outright *unless this is a PV node*, where it's
    /// re-searched anyway purely so `self.pv` gets a real line for this move
    /// -- see the match arm's own comment for why that's free here. Losing
    /// the exact-vs-ambiguous distinction -- treating "probe didn't need a
    /// re-search" as an abort, or vice versa -- is the easy way to get this
    /// backwards; `None` from `search_child` means only one thing anywhere in
    /// this function: the search was interrupted.
    ///
    /// Generic over `F` rather than taking `&mut dyn FnMut`: this is the
    /// innermost loop of the entire engine, so it monomorphizes per call
    /// site and inlines exactly as hand-written copies would.
    #[expect(
        clippy::too_many_lines,
        reason = "the engine's innermost loop; the sequence is the algorithm, and splitting it costs the reader more than the length does"
    )]
    fn alpha_beta_loop<D, F>(
        &mut self,
        moves: &MoveList,
        ctx: LoopCtx,
        mut decide: D,
        mut search_child: F,
    ) -> LoopOutcome
    where
        D: FnMut(&mut Self, Move, &MoveCtx) -> Verdict,
        F: FnMut(&mut Self, Move, LoopCtx) -> Option<Score>,
    {
        let node_is_pv = ctx.is_pv();
        let LoopCtx {
            ply,
            mut alpha,
            beta,
            depth,
        } = ctx;

        let mut max = Score::MIN;
        let mut best_move = None;
        let mut cutoff_index = None;
        let mut searched = 0usize;
        let full = depth.saturating_sub(1);

        for (i, &m) in moves.iter().enumerate() {
            // The first move is never put to `decide`. A node that pruned
            // every move is indistinguishable from one with no legal moves,
            // which scores a live position as mate or stalemate; that is a
            // property of the loop rather than of any one technique, so it
            // lives here instead of in every policy that could violate it.
            let verdict = if searched == 0 {
                Verdict::Search
            } else {
                decide(
                    self,
                    m,
                    &MoveCtx {
                        index: i,
                        alpha,
                        searched,
                    },
                )
            };
            let reduction = match verdict {
                Verdict::Skip => continue,
                Verdict::Stop => break,
                Verdict::Search => 0,
                // Never reduce into quiescence: a child at depth 0 is a
                // different kind of search, not a shallower one.
                Verdict::Reduce(r) => r.min(full.saturating_sub(1)),
            };

            let mut is_pv = node_is_pv;
            // The child is a node too, so it gets the same context this one
            // was handed, one ply down with the window negated into its own
            // frame. Every call below is this with one field changed, which is
            // the whole of what a probe, a re-deepening and a widening differ
            // by.
            let child = LoopCtx {
                ply: ply + 1,
                alpha: -beta,
                beta: -alpha,
                depth: full,
            };
            let child_result = if searched == 0 {
                search_child(self, m, child)
            } else {
                is_pv = false;
                let probe = search_child(
                    self,
                    m,
                    LoopCtx {
                        alpha: child.beta - 1,
                        depth: full - reduction,
                        ..child
                    },
                );
                match probe {
                    None => None,
                    // A reduced search that beats alpha has not earned the
                    // score it reported: re-run it at full depth *before* the
                    // widening below, or a shallow number gets widened rather
                    // than re-deepened and the reduction silently becomes a
                    // change in what the engine plays.
                    Some(p) if reduction > 0 && -p > alpha => {
                        match search_child(
                            self,
                            m,
                            LoopCtx {
                                alpha: child.beta - 1,
                                ..child
                            },
                        ) {
                            Some(deep) if -deep > alpha && (-deep < beta || node_is_pv) => {
                                is_pv = true;
                                search_child(self, m, child)
                            }
                            deep => deep,
                        }
                    }
                    // Re-search whenever the probe beats alpha and either the
                    // score is still ambiguous (could be anywhere above alpha,
                    // needs the real window to pin down) or this is a PV node,
                    // where a probe failing high past beta is *not* ambiguous
                    // (fail-soft already returned its real, trustworthy score,
                    // which is why the non-PV case below doesn't bother) but
                    // still isn't PV-worthy without this: a null-window probe
                    // never sets `is_pv: true`, so without a PV-node re-search
                    // here, this move's own line in `self.pv` would never get
                    // built, and this exact move is always the one about to
                    // become `max` and get reported. Free at a PV node: this
                    // move already forces an immediate cutoff-and-break right
                    // after (`alpha` rises to at least `beta`), so there is no
                    // later sibling left in this loop that a stale re-search
                    // could ever get overwritten by.
                    Some(p) if -p > alpha && (-p < beta || node_is_pv) => {
                        is_pv = true;
                        search_child(self, m, child)
                    }
                    p => p,
                }
            };
            let Some(score) = child_result else {
                return LoopOutcome {
                    max,
                    best_move,
                    aborted: true,
                    cutoff_index,
                };
            };
            searched += 1;

            let score = -score;
            if score > max {
                max = score;
                best_move = Some(m);
                if is_pv {
                    let row = usize::from(ply).min(MAX_PV_PLY - 1);
                    // would ideally be `self.pv[row] = self.pv[row+1]; self.pv[row].shift_right::<1>([Some(m)]);` but that is behind an unstable feature
                    if row != MAX_PV_PLY - 1 {
                        for i in 0..MAX_PV_PLY - 1 {
                            self.pv[row][i + 1] = self.pv[row + 1][i];
                        }
                    }
                    self.pv[row][0] = Some(m);
                }
            }

            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                let cause = self.ordering.on_cutoff(ply, m, max);
                self.negamax_cutoffs.record(i, cause);
                cutoff_index = Some(i);
                break;
            }
        }

        LoopOutcome {
            max,
            best_move,
            aborted: false,
            cutoff_index,
        }
    }

    /// Quiescence's own move loop: the simple half of what used to be one
    /// generic [`Search::alpha_beta_loop`] shared with `negamax`/`search_root`
    /// (see that method's own doc, and `LoopCtx`'s, for why they split).
    /// Every move gets the caller's own window once, full stop -- no PVS
    /// windowing, no re-search, no PV bookkeeping, because quiescence nodes
    /// never carry `is_pv` at all. `initial_max` is `Score::MIN` for the
    /// evasion loop and `stand_pat` for the capture loop, the one place these
    /// two calls still genuinely differ.
    fn quiescence_loop<F>(
        &mut self,
        moves: &MoveList,
        ply: u8,
        alpha: Score,
        beta: Score,
        initial_max: Score,
        mut search_child: F,
    ) -> LoopOutcome
    where
        F: FnMut(&mut Self, Move, Score, Score) -> Option<Score>,
    {
        let mut alpha = alpha;
        let mut max = initial_max;
        let mut best_move = None;
        let mut cutoff_index = None;

        for (i, &m) in moves.iter().enumerate() {
            let Some(score) = search_child(self, m, -beta, -alpha) else {
                return LoopOutcome {
                    max,
                    best_move,
                    aborted: true,
                    cutoff_index,
                };
            };

            let score = -score;
            if score > max {
                max = score;
                best_move = Some(m);
            }

            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                let cause = self.ordering.on_cutoff(ply, m, max);
                self.quiescence_cutoffs.record(i, cause);
                cutoff_index = Some(i);
                break;
            }
        }

        LoopOutcome {
            max,
            best_move,
            aborted: false,
            cutoff_index,
        }
    }

    /// One ply of root move loop: like `negamax`, but remembers *which*
    /// move produced the best score, not just the score, so it's a
    /// separate small loop rather than a `ply == 0` special case buried
    /// inside `negamax`. The move loop itself is shared with `negamax` (see
    /// [`Search::alpha_beta_loop`]); what remains here is the handful of
    /// things the root genuinely does differently. It's always a PV node
    /// (there is no ancestor to have narrowed its window). It scores mate as
    /// `-MATE` rather than `Score::from(ply) - MATE`, since `ply` is zero
    /// here by definition. It neither probes nor stores the transposition
    /// table. It shuffles the move list before ordering, when randomization
    /// is on. And when the position is already a draw, an interior node can
    /// just return `0` and stop (see `negamax`'s own doc), but the root still
    /// needs a real move to hand back to UCI so play can continue, so it
    /// generates and orders the move list here instead.
    ///
    /// Returns [`RootOutcome::Aborted`] if the search was interrupted before
    /// every move at this depth could be tried. Moves already tried only ever
    /// update `max`/`best_move`/`self.pv[0]` after their subtree search
    /// returns a real score, never from a partially-searched one, so
    /// whatever they hold at the moment of abort are still trustworthy
    /// values; only the one move that was mid-flight is discarded, which is
    /// why `best_so_far` reports the finished moves' result rather than
    /// discarding it wholesale.
    fn search_root(&mut self, board: &Board, depth: u8) -> RootOutcome {
        self.nodes += 1;
        if self.should_abort() {
            return RootOutcome::Aborted { best_so_far: None };
        }

        // Once, for every path out of this node: the root neither probes nor
        // stores the table (see this function's own doc), so it orders by no
        // hash move whichever way it exits.
        self.ordering.set_hash_move(0, None);

        let mut pv: PV = [None; MAX_PV_PLY];

        if is_draw(board, &self.history, board.hash()) {
            let mut drawn_moves = legal_moves(board);
            if drawn_moves.is_empty() {
                return RootOutcome::Completed(0, pv);
            }
            self.ordering.order(board, &mut drawn_moves, 0);
            pv[0] = Some(drawn_moves[0]);
            return RootOutcome::Completed(0, pv);
        }

        let mut moves = legal_moves(board);
        if moves.is_empty() {
            return if in_check(board, board.side_to_move()) {
                RootOutcome::Completed(-MATE, pv)
            } else {
                RootOutcome::Completed(0, pv)
            };
        }

        let alpha = -MATE;
        let beta = MATE;

        // Unreachable from `search`, whose iterative deepening starts at 1,
        // but `depth` is a plain `u8` with no type-level floor, so a future
        // caller could pass 0. Handing that to quiescence is the honest
        // answer rather than searching a zero-depth tree.
        if depth == 0 {
            return self
                .quiescence(board, alpha, beta, 0, MAX_QUIESCENCE_DEPTH, Some(moves))
                .map_or(RootOutcome::Aborted { best_so_far: None }, |result| {
                    RootOutcome::Completed(result.score, pv)
                });
        }

        // Before ordering, not after: `MoveOrdering::order` sorts by a coarse priority
        // class, so shuffling first is what decides which of several moves
        // sharing a class gets tried first, while still leaving the ordering
        // itself intact.
        self.shuffle_root_moves(&mut moves);
        self.ordering.order(board, &mut moves, 0);

        let outcome = self.alpha_beta_loop(
            &moves,
            LoopCtx {
                ply: 0,
                alpha,
                beta,
                depth,
            },
            // The root searches every legal move, always. A move pruned here
            // is one the engine can never play, however good it was, so this
            // is permanent rather than a policy waiting to be filled in.
            |_, _, _| Verdict::Search,
            |s, m, c| {
                s.history.push(board.hash());
                let child = board.make_move(m);
                let result = s.negamax(&child, c.depth, c.ply, c.alpha, c.beta, c.is_pv());
                s.history.pop();
                result.map(|r| r.score)
            },
        );

        if outcome.aborted {
            let best_so_far = outcome.best_move.map(|_| (outcome.max, self.pv[0]));
            return RootOutcome::Aborted { best_so_far };
        }
        self.ordering
            .update_history(board, &moves, outcome.cutoff_index, depth);
        RootOutcome::Completed(outcome.max, self.pv[0])
    }

    /// Fail-soft negamax with alpha-beta pruning: on a beta cutoff, returns
    /// the actual score found, not the clamped `beta` bound. `ply` is the
    /// distance from the root (0 there), used for the mate-score formula on
    /// [`MATE`]'s doc and for keeping `self.history` in step with the
    /// recursion.
    ///
    /// `ply` is `u8`, same width as `depth`: both are bounded by the same
    /// `max_depth` a search was started with, so nothing is lost keeping
    /// them the same type, and `Score::from(ply)` below stays a plain
    /// widening conversion either way.
    ///
    /// Returns `None` if `should_abort()` trips; callers must propagate a
    /// `None` up immediately rather than treating it as a real score.
    ///
    /// Two ordering gotchas, easy to get backwards: `is_draw` must be
    /// checked *before* move generation, since an already-drawn position
    /// scores `0` regardless of material and shouldn't be searched further.
    /// And an empty move list is checkmate/stalemate (the [`MATE`] formula,
    /// or `0`), a different terminal case from the draw check above it, not
    /// the same `0` for a different reason.
    ///
    /// `self.tt` is probed right after those two terminal checks (so a hit doesn't cost
    /// them, but a draw/terminal score is never confused with a bounded search result the
    /// table would store), and *before* the `depth == 0` quiescence handoff: a stored
    /// result from a deeper earlier visit to this same position can still resolve a
    /// depth-0 node outright, skipping quiescence entirely, which is exactly the kind of
    /// win a transposition table exists for. Storing happens only on the path that
    /// actually ran this node's own move loop, keyed on the *original* `alpha`/`beta` this
    /// call was given, not `alpha` as the loop below narrows it; see `Bound`'s own doc for
    /// why that distinction matters. A depth-0 node that fell through to `quiescence`
    /// instead never reaches the store below, matching `Entry.depth`'s own doc: `quiescence`
    /// has no comparable notion of depth to store one under. The probed entry itself is
    /// kept around past the cutoff check (it's `Copy`, so this costs nothing): even when it
    /// doesn't license an outright cutoff, its stored move is still worth trying first in
    /// this node's own move loop, so it survives long enough to feed `MoveOrdering::order`.
    ///
    /// `is_pv`: whether this node was reached via a genuinely wide window
    /// from its parent, not a null-window probe; see [`LoopCtx`]'s own doc
    /// for the full propagation rule. Threaded straight through to
    /// [`Self::alpha_beta_loop`]'s `ctx.is_pv`.
    #[expect(
        clippy::expect_used,
        reason = "the move list's emptiness is checked and returned on well above this point, so the loop always records a best move before the store"
    )]
    fn negamax(
        &mut self,
        board: &Board,
        depth: u8,
        ply: u8,
        alpha: Score,
        beta: Score,
        is_pv: bool,
    ) -> Option<TaintedScore> {
        self.nodes += 1;
        if self.should_abort() {
            return None;
        }

        // Every return below this point that doesn't reach the move loop --
        // the draw check, a TT cutoff, checkmate/stalemate, and the depth-0
        // handoff to quiescence -- leaves without ever writing `self.pv[ply]`.
        // Clearing it here first (only when this call is actually on the PV,
        // since nothing else ever reads or writes `self.pv`) means every one
        // of those paths correctly reports "no continuation from here"
        // instead of whatever an earlier, unrelated sibling happened to leave
        // in that same shared slot; the move loop further down overwrites it
        // with the real line if it runs. Without this, a PV node whose first
        // move leads to (say) a forced mate one ply later can copy up a
        // completely unrelated earlier sibling's leftover continuation,
        // reporting an illegal move as part of the principal variation.
        if is_pv {
            self.pv[usize::from(ply).min(MAX_PV_PLY - 1)] = [None; MAX_PV_PLY];
        }

        // The fifty-move half of `is_draw` is scored the same way but never
        // taints: this suppression is scoped to the repetition half alone,
        // since mixing both would make the node-count/hashfull measurement
        // it's gated on impossible to attribute to either one.
        let repetition = is_threefold_repetition(&self.history, board.hash());
        if is_fifty_move_draw(board) || repetition {
            return Some(TaintedScore {
                score: 0,
                tainted: repetition,
            });
        }

        let hash = board.hash();
        let tt_entry = self.tt.as_deref().and_then(|tt| tt.probe(hash));
        if let Some(entry) = tt_entry {
            if let Some(score) = entry.cutoff_score(depth, alpha, beta, ply) {
                // Untainted: post-suppression, the table never holds an
                // entry stored from a draw-tainted subtree to read back.
                return Some(TaintedScore {
                    score,
                    tainted: false,
                });
            }
        }

        let mut moves = legal_moves(board);
        if moves.is_empty() {
            let score = if in_check(board, board.side_to_move()) {
                Score::from(ply) - MATE
            } else {
                0
            };
            return Some(TaintedScore {
                score,
                tainted: false,
            });
        }

        if depth == 0 {
            return self.quiescence(board, alpha, beta, ply, MAX_QUIESCENCE_DEPTH, Some(moves));
        }

        let tt_move = tt_entry.map(|entry| entry.mv);

        let original_alpha = alpha;
        self.ordering.set_hash_move(ply, tt_move);
        let runs = self.ordering.order(board, &mut moves, ply);

        let mut any_child_tainted = false;
        let outcome = self.alpha_beta_loop(
            &moves,
            LoopCtx {
                ply,
                alpha,
                beta,
                depth,
            },
            // Futility pruning and late move pruning join this match, each
            // behind its own gate. A reduction of zero is the policy declining
            // rather than a reduction of no plies, so it maps to `Search`.
            |_, _, ctx| match lmr::reduction(depth, ctx.searched, runs.is_quiet(ctx.index), is_pv) {
                0 => Verdict::Search,
                plies => Verdict::Reduce(plies),
            },
            |s, m, c| {
                s.history.push(board.hash());
                let child = board.make_move(m);
                let result = s.negamax(&child, c.depth, c.ply, c.alpha, c.beta, c.is_pv());
                s.history.pop();
                result.map(|r| {
                    any_child_tainted |= r.tainted;
                    r.score
                })
            },
        );

        if outcome.aborted {
            return None;
        }
        self.ordering
            .update_history(board, &moves, outcome.cutoff_index, depth);

        if !any_child_tainted {
            if let Some(tt) = self.tt.as_deref_mut() {
                let best_move = outcome
                    .best_move
                    .expect("moves is non-empty, so the loop always finds a best move");
                tt.store(
                    hash,
                    ply,
                    depth,
                    outcome.max,
                    original_alpha,
                    beta,
                    best_move,
                );
            }
        }

        Some(TaintedScore {
            score: outcome.max,
            tainted: any_child_tainted,
        })
    }

    /// Quiescence search: like `negamax`, but only ever considers captures
    /// and promotions while not in check, with "stand pat" (`evaluate(board)`)
    /// as the floor score, so a position with no good captures or promotions
    /// isn't forced into playing one. Promotions are in scope alongside
    /// captures because a pawn one push from queening is exactly the kind of
    /// tactical, still-unsettled position quiescence exists to resolve
    /// rather than hand to `evaluate` as if the pre-promotion material were
    /// the real story. While in check, stand pat does not apply at all:
    /// every legal reply is a forced evasion by definition, and a static
    /// score can't tell a merely-bad position from a lost one, so this
    /// searches all of them instead, with no `qdepth` cap (a check has to be
    /// resolved regardless of how deep quiescence has already gone; see
    /// `qdepth`'s own doc below), and returns [`MATE`]'s formula outright
    /// when none exist rather than a misleading material score for what is
    /// actually checkmate. Same fail-soft alpha-beta and abort propagation
    /// as `negamax` either way.
    ///
    /// Threads `self.history` the same way `negamax` does, but only checks
    /// it for a repetition on the evasion path: captures and promotions can
    /// never repeat a position (both are irreversible), so checking there
    /// too would only cost cycles for an answer that's always "no." The
    /// fifty-move clock is still out of scope on both paths, the same gap
    /// `negamax`'s own repetition-only suppression leaves open elsewhere.
    ///
    /// `ply` is `negamax`'s own distance-from-root, passed through so an
    /// empty evasion list scores `Score::from(ply) - MATE`, the identical
    /// formula `negamax` uses for its own terminal case rather than a second
    /// copy of it.
    ///
    /// `qdepth` counts down from [`MAX_QUIESCENCE_DEPTH`] and is unrelated
    /// to `ply`: it bounds how many more plies of *captures* this call will
    /// still resolve while not in check, not the distance from the root. At
    /// `qdepth == 0`, this behaves as if no captures were available, falling
    /// back to `stand_pat`. The evasion path keeps counting it down too
    /// (`saturating_sub`, since evasions can run past what would be `qdepth
    /// == 0` on the capture side), but never gates on it reaching zero:
    /// there, it exists purely so a cutoff can be weighted by how deep into
    /// quiescence as a whole (captures and evasions both) the position
    /// already is, not to bound the search itself.
    ///
    /// `moves`: `Some` when the caller (`negamax`/`search_root`) already
    /// generated the full move list for its own mate/stalemate check, so
    /// this doesn't generate the same board's moves twice; `None` (this
    /// function's own recursive calls, where `board` is a fresh child)
    /// generates here instead. On the evasion path, `moves` (once generated)
    /// already *is* the evasion list: every legal move while in check
    /// resolves that check by definition, so no further filtering is
    /// needed or correct here.
    fn quiescence(
        &mut self,
        board: &Board,
        mut alpha: Score,
        beta: Score,
        ply: u8,
        qdepth: u8,
        moves: Option<MoveList>,
    ) -> Option<TaintedScore> {
        self.nodes += 1;
        if self.should_abort() {
            return None;
        }

        // Once, above the branch: quiescence orders by no hash move on either
        // path, in check or out of it, whatever the table holds for these
        // positions.
        self.ordering.set_hash_move(ply, None);

        if in_check(board, board.side_to_move()) {
            let repetition = is_threefold_repetition(&self.history, board.hash());
            if repetition {
                return Some(TaintedScore {
                    score: 0,
                    tainted: true,
                });
            }

            let mut evasions = moves.unwrap_or_else(|| legal_moves(board));
            if evasions.is_empty() {
                return Some(TaintedScore {
                    score: Score::from(ply) - MATE,
                    tainted: false,
                });
            }

            self.ordering.order(board, &mut evasions, ply);

            let mut any_child_tainted = false;
            let outcome =
                self.quiescence_loop(&evasions, ply, alpha, beta, Score::MIN, |s, m, a, b| {
                    s.history.push(board.hash());
                    let child = board.make_move(m);
                    let result =
                        s.quiescence(&child, a, b, ply + 1, qdepth.saturating_sub(1), None);
                    s.history.pop();
                    result.map(|r| {
                        any_child_tainted |= r.tainted;
                        r.score
                    })
                });

            if outcome.aborted {
                return None;
            }
            // `.max(1)` floors only the degenerate case (a check reached
            // with the full budget still available, `qdepth ==
            // MAX_QUIESCENCE_DEPTH`) at a real, if minimal, weight instead
            // of `0`: `cutoff_delta(0)` is a silent no-op. Every other
            // value is left alone rather than uniformly shifted up, so the
            // scale still tops out at `MAX_QUIESCENCE_DEPTH` itself.
            let evasion_history_depth = MAX_QUIESCENCE_DEPTH.saturating_sub(qdepth).max(1);
            self.ordering.update_history(
                board,
                &evasions,
                outcome.cutoff_index,
                evasion_history_depth,
            );
            return Some(TaintedScore {
                score: outcome.max,
                tainted: any_child_tainted,
            });
        }

        let stand_pat = evaluate(board);
        if stand_pat >= beta {
            return Some(TaintedScore {
                score: stand_pat,
                tainted: false,
            });
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        if qdepth == 0 {
            return Some(TaintedScore {
                score: stand_pat,
                tainted: false,
            });
        }

        let mut qmoves = moves.unwrap_or_else(|| legal_moves(board));
        qmoves.retain(|m| m.flags().is_capture() || m.flags().is_promotion());

        self.ordering.order(board, &mut qmoves, ply);

        let mut any_child_tainted = false;
        let outcome = self.quiescence_loop(&qmoves, ply, alpha, beta, stand_pat, |s, m, a, b| {
            s.history.push(board.hash());
            let child = board.make_move(m);
            let result = s.quiescence(&child, a, b, ply + 1, qdepth - 1, None);
            s.history.pop();
            result.map(|r| {
                any_child_tainted |= r.tainted;
                r.score
            })
        });

        if outcome.aborted {
            return None;
        }
        Some(TaintedScore {
            score: outcome.max,
            tainted: any_child_tainted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fixtures::{capture_and_quiet_position, find_move};
    use turox_chess::types::{Color, Piece, Square};

    /// A move list of `n` distinct legal moves, for driving `alpha_beta_loop`
    /// directly. Which moves they are does not matter: every test below stubs
    /// the child search, so the loop never looks at a position.
    fn some_moves(n: usize) -> MoveList {
        let mut list = MoveList::new();
        for m in legal_moves(&Board::start_pos()).iter().take(n) {
            list.push(*m);
        }
        assert_eq!(list.len(), n, "start position has at least {n} legal moves");
        list
    }

    /// Drives `alpha_beta_loop` with stubbed policy and child search, and
    /// reports every `(move index, depth)` the loop actually searched.
    ///
    /// The child score is a fixed function of depth, so a reduced search and a
    /// full-depth one return different numbers and the re-search paths become
    /// observable rather than inferred.
    fn drive(
        moves: &MoveList,
        ctx: LoopCtx,
        mut decide: impl FnMut(&MoveCtx) -> Verdict,
        mut child: impl FnMut(usize, u8) -> Option<Score>,
    ) -> (LoopOutcome, Vec<(usize, u8)>) {
        let mut search = Search::new(Vec::new());
        let calls = std::cell::RefCell::new(Vec::new());
        let index_of = |m: Move| {
            moves
                .iter()
                .position(|&x| x == m)
                .expect("move is in the list")
        };
        let outcome = search.alpha_beta_loop(
            moves,
            ctx,
            |_, m, mctx| {
                let _ = m;
                decide(mctx)
            },
            |_, m, c| {
                let i = index_of(m);
                calls.borrow_mut().push((i, c.depth));
                child(i, c.depth)
            },
        );
        (outcome, calls.into_inner())
    }

    /// A node at `depth` with a wide window, and therefore a principal
    /// variation node: `LoopCtx::is_pv` reads the window rather than a flag,
    /// and the tests below need a window loose enough that a move does not
    /// cut off before the loop's own behaviour can be observed.
    fn ctx_at(depth: u8) -> LoopCtx {
        LoopCtx {
            ply: 1,
            alpha: -100,
            beta: 100,
            depth,
        }
    }

    #[test]
    fn a_wide_window_is_a_pv_node_and_a_null_window_is_not() {
        assert!(
            LoopCtx {
                ply: 0,
                alpha: -100,
                beta: 100,
                depth: 4
            }
            .is_pv(),
            "a window with room between the bounds can return an exact score"
        );
        // The shape every null-window probe in the loop uses: beta is exactly
        // one above alpha, so the search can only ever prove a bound.
        assert!(
            !LoopCtx {
                ply: 0,
                alpha: 41,
                beta: 42,
                depth: 4
            }
            .is_pv(),
            "a one-point window can only prove a bound, never an exact score"
        );
        // Two points apart is still wide enough to be exact, which is what an
        // aspiration window narrowed around the previous score looks like.
        assert!(
            LoopCtx {
                ply: 0,
                alpha: 40,
                beta: 42,
                depth: 4
            }
            .is_pv(),
            "a narrow window is not a null window"
        );
    }

    #[test]
    fn the_first_move_is_never_put_to_the_policy() {
        let moves = some_moves(4);
        let mut asked = Vec::new();
        let (_, searched) = drive(
            &moves,
            ctx_at(4),
            |mctx| {
                asked.push(mctx.index);
                Verdict::Stop
            },
            |_, _| Some(0),
        );
        assert_eq!(
            asked,
            vec![1],
            "policy should first be consulted on move 1, never move 0"
        );
        assert_eq!(
            searched.len(),
            1,
            "a policy that stops immediately still leaves one move searched"
        );
    }

    #[test]
    fn the_policy_sees_alpha_rise_and_the_searched_count_lag_the_index() {
        let moves = some_moves(4);
        let mut seen: Vec<(usize, Score, usize)> = Vec::new();
        let _ = drive(
            &moves,
            ctx_at(4),
            |mctx| {
                seen.push((mctx.index, mctx.alpha, mctx.searched));
                if mctx.index == 2 {
                    Verdict::Skip
                } else {
                    Verdict::Search
                }
            },
            // Negated by the loop, so later moves look strictly better and
            // alpha climbs as they are searched.
            |i, _| Some(-(Score::try_from(i).unwrap_or(0) * 10)),
        );

        let alphas: Vec<Score> = seen.iter().map(|&(_, a, _)| a).collect();
        assert!(
            alphas.windows(2).all(|w| w[1] >= w[0]),
            "alpha is the live value and only ever rises: {seen:?}"
        );
        assert!(
            alphas.first() < alphas.last(),
            "alpha actually moved during the loop: {seen:?}"
        );

        let after_skip = seen
            .iter()
            .find(|&&(i, _, _)| i == 3)
            .expect("move 3 was reached");
        assert_eq!(
            after_skip.2, 2,
            "move 2 was skipped, so only two moves had been searched by move 3: {seen:?}"
        );
    }

    #[test]
    fn skip_passes_over_a_move_without_searching_it() {
        let moves = some_moves(4);
        let (_, searched) = drive(
            &moves,
            ctx_at(4),
            |mctx| {
                if mctx.index == 1 {
                    Verdict::Skip
                } else {
                    Verdict::Search
                }
            },
            |_, _| Some(0),
        );
        let indices: Vec<usize> = searched.iter().map(|&(i, _)| i).collect();
        assert!(!indices.contains(&1), "move 1 was skipped: {indices:?}");
        assert!(indices.contains(&2), "later moves still run: {indices:?}");
    }

    #[test]
    fn stop_ends_the_loop_rather_than_skipping_one_move() {
        let moves = some_moves(5);
        let (_, searched) = drive(
            &moves,
            ctx_at(4),
            |mctx| {
                if mctx.index == 2 {
                    Verdict::Stop
                } else {
                    Verdict::Search
                }
            },
            |_, _| Some(0),
        );
        let max_index = searched
            .iter()
            .map(|&(i, _)| i)
            .max()
            .expect("searched something");
        assert!(
            max_index < 2,
            "nothing at or past the stop ran: {searched:?}"
        );
    }

    #[test]
    fn reduce_searches_shallower_first() {
        let moves = some_moves(3);
        let (_, searched) = drive(
            &moves,
            ctx_at(6),
            |_| Verdict::Reduce(2),
            // Fails low, so nothing is re-searched and the reduced depth stands.
            |_, _| Some(1000),
        );
        let depths: Vec<u8> = searched
            .iter()
            .filter(|&&(i, _)| i == 1)
            .map(|&(_, d)| d)
            .collect();
        assert_eq!(
            depths,
            vec![3],
            "move 1 should be searched once, at depth - 1 - 2"
        );
    }

    #[test]
    fn a_reduced_search_that_beats_alpha_is_re_run_at_full_depth() {
        let moves = some_moves(3);
        let (_, searched) = drive(
            &moves,
            ctx_at(6),
            |_| Verdict::Reduce(2),
            // Move 0 settles alpha at 0; move 1 then comes back better than
            // that once negated, which is what a reduced search beating alpha
            // looks like and what must not be trusted at the reduced depth.
            |i, _| Some(if i == 0 { 0 } else { -80 }),
        );
        let depths: Vec<u8> = searched
            .iter()
            .filter(|&&(i, _)| i == 1)
            .map(|&(_, d)| d)
            .collect();
        assert_eq!(
            depths.first(),
            Some(&3),
            "the first search of a reduced move is the shallow probe: {depths:?}"
        );
        // The *second* search specifically. A later widening also runs at full
        // depth, so asserting the list merely contains it would pass even with
        // the re-deepen left shallow, which is the bug this test exists for.
        assert_eq!(
            depths.get(1),
            Some(&5),
            "a reduced probe beating alpha is re-deepened before anything else: {depths:?}"
        );
    }

    #[test]
    fn a_reduction_never_reaches_into_quiescence() {
        let moves = some_moves(3);
        let (_, searched) = drive(
            &moves,
            ctx_at(2),
            |_| Verdict::Reduce(10),
            |_, _| Some(1000),
        );
        let min_depth = searched
            .iter()
            .map(|&(_, d)| d)
            .min()
            .expect("searched something");
        assert!(
            min_depth >= 1,
            "depth 0 is quiescence, not a shallower search: {searched:?}"
        );
    }

    #[test]
    fn a_policy_that_always_searches_uses_full_depth_throughout() {
        let moves = some_moves(4);
        let (_, searched) = drive(&moves, ctx_at(5), |_| Verdict::Search, |_, _| Some(1000));
        assert!(
            searched.iter().all(|&(_, d)| d == 4),
            "every child at depth - 1: {searched:?}"
        );
    }

    /// Every `history.push` in the move loop is matched by a `pop` before the
    /// `-score?` that can propagate an abort, even along the path that aborts.
    #[test]
    fn aborted_search_leaves_the_history_stack_as_it_found_it() {
        let board = Board::start_pos();
        let seed_history = vec![0xAAAA_BBBB_CCCC_DDDD, 0x1111_2222_3333_4444];

        let mut probe = Search::new(seed_history.clone());
        let unbounded = probe.search(&board, 4);

        let mut bounded = Search::new(seed_history.clone()).with_max_nodes(unbounded.nodes / 2);
        bounded.search(&board, 4);

        assert_eq!(bounded.history.len(), seed_history.len());
    }

    /// `negamax` stores using the alpha the node was *called* with, not the
    /// alpha its own move loop narrows while searching moves.
    ///
    /// `Entry::cutoff_score` with a window wider than any real score only
    /// returns `Some` for `Bound::Exact`, which is what lets this assert
    /// through the public API without exposing `Entry`'s private fields.
    #[test]
    fn negamax_stores_the_bound_using_the_alpha_this_node_was_called_with() {
        let board = capture_and_quiet_position();
        let mut tt = Tt::new(Tt::MIN_HASH_MB);
        let score = {
            let mut search = Search::new(Vec::new()).with_tt(&mut tt);
            search
                .negamax(&board, 2, 0, -MATE, MATE, true)
                .expect("no abort condition is configured, so this can't return None")
                .score
        };

        let entry = tt
            .probe(board.hash())
            .expect("depth 2 always reaches the store");
        assert_eq!(
            entry.cutoff_score(2, Score::MIN, Score::MAX, 0),
            Some(score)
        );
    }

    /// A lone king and a pawn one push from queening, against a lone king:
    /// no capture exists anywhere on the board, so quiescence's only real
    /// qmove here is the pawn's own non-capturing promotion. Searching it
    /// one ply deeper, where the queen's material swing dwarfs the
    /// pre-promotion stand-pat score, must beat standing pat outright.
    #[test]
    fn quiescence_searches_a_winning_non_capturing_promotion_instead_of_standing_pat() {
        let board = Board::try_from_fen("8/P7/8/8/8/4k3/8/4K3 w - - 0 1").expect("valid FEN");
        let mut search = Search::new(Vec::new());
        let stand_pat = evaluate(&board);

        let score = search
            .quiescence(&board, -MATE, MATE, 0, MAX_QUIESCENCE_DEPTH, None)
            .expect("no abort condition is configured, so this can't return None")
            .score;

        assert!(
            score > stand_pat,
            "quiescence must search past a legal non-capturing promotion rather than standing \
             pat on the pre-promotion material: stand_pat={stand_pat}, score={score}"
        );
    }

    // ---- Quiescence: repetition detection and taint ----

    /// White's king in check from a rook on an open file, with nothing to
    /// capture: every legal reply is a quiet king step, so this stays on
    /// the evasion path with no way to exit through a capture instead.
    fn check_with_only_quiet_evasions() -> Board {
        Board::try_from_fen("4r2k/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN")
    }

    #[test]
    fn quiescence_scores_a_seeded_repetition_as_a_tainted_draw_on_the_evasion_path() {
        let board = check_with_only_quiet_evasions();
        // Two prior occurrences of this exact position, matching
        // `is_threefold_repetition`'s own contract: the position `quiescence`
        // is about to search is itself the third.
        let history = vec![board.hash(), board.hash()];
        let mut search = Search::new(history);

        let result = search
            .quiescence(&board, -MATE, MATE, 0, MAX_QUIESCENCE_DEPTH, None)
            .expect("no abort condition is configured, so this can't return None");

        assert_eq!(
            result.score, 0,
            "a threefold repetition on the evasion path must score as a draw, not \
             recurse into evasions as if it weren't one"
        );
        assert!(
            result.tainted,
            "a repetition-drawn score must be reported tainted, the same as negamax's own"
        );
    }

    /// White's queen captures the only Black pawn while also giving check
    /// (`h5` to `f7` is a clear diagonal, and a queen on `f7` is adjacent
    /// to a king on `e8`): `board` itself is not in check (the capture
    /// path), but the position right after `Qxf7+` is, crossing from the
    /// capture branch into the evasion branch one ply into the recursion.
    #[test]
    fn quiescence_propagates_a_childs_repetition_taint_across_the_capture_to_evasion_boundary() {
        let board = Board::try_from_fen("4k3/5p2/8/7Q/8/8/8/4K3 w - - 0 1").expect("valid FEN");
        let qxf7 = find_move(&board, Square::H5, Square::F7);
        let after_qxf7 = board.make_move(qxf7);
        assert!(
            in_check(&after_qxf7, after_qxf7.side_to_move()),
            "Qxf7 must deliver check for this test to exercise the evasion path at all"
        );

        // `board` itself is not the repeated position (no prior occurrences
        // seeded), but `after_qxf7` is: two prior occurrences of *its*
        // hash, so the taint has to come from the child's own result, not
        // from `board`'s own top-of-function check (which never even runs,
        // since `board` isn't in check and never reaches that branch).
        let history = vec![after_qxf7.hash(), after_qxf7.hash()];
        let mut search = Search::new(history);

        let result = search
            .quiescence(&board, -MATE, MATE, 0, MAX_QUIESCENCE_DEPTH, None)
            .expect("no abort condition is configured, so this can't return None");

        assert!(
            result.tainted,
            "a repetition found in a child reached via the capture path must still taint \
             the parent's own result, the same `any_child_tainted` propagation negamax's \
             own move loop uses"
        );
    }

    // ---- Quiescence: evasion-depth history weighting ----

    /// A cutoff on the evasion path weights `CutoffHistory` by how much of
    /// `qdepth`'s shared budget is already spent, not a flat constant: the
    /// same forced-cutoff scenario started with less budget remaining
    /// (simulating a check reached after several captures already ran)
    /// must move the table more than the same scenario started fresh.
    #[test]
    fn quiescence_weights_an_evasion_cutoff_more_the_deeper_into_quiescence_it_is() {
        let board = check_with_only_quiet_evasions();
        // A window so narrow that whichever evasion `MoveOrdering::order` tries
        // first causes an immediate cutoff at index 0, regardless of its
        // real evaluation: deterministic without needing to know or care
        // which of the five equally-quiet king steps that turns out to be.
        // `Score::MIN` itself can't be negated (no positive counterpart;
        // see `is_mate_score`'s own doc on `saturating_abs`), so `MIN + 1`
        // is as narrow as this window can safely go.
        let alpha = Score::MIN + 1;
        let beta = Score::MIN + 2;

        let mut shallow_history = CutoffHistory::new();
        Search::new(Vec::new())
            .with_cutoff_history(&mut shallow_history)
            .quiescence(&board, alpha, beta, 0, MAX_QUIESCENCE_DEPTH, None);

        let mut deep_history = CutoffHistory::new();
        Search::new(Vec::new())
            .with_cutoff_history(&mut deep_history)
            .quiescence(&board, alpha, beta, 0, 2, None);

        let touched_cell = |history: &CutoffHistory| {
            Color::ALL.into_iter().find_map(|side| {
                Piece::ALL.into_iter().find_map(|piece| {
                    Square::ALL
                        .into_iter()
                        .find(|&to| history.score(side, piece, to) != 0)
                        .map(|to| (side, piece, to))
                })
            })
        };
        let (side, piece, to) = touched_cell(&shallow_history)
            .expect("the forced cutoff must record something in a fresh table");

        assert_eq!(
            touched_cell(&deep_history),
            Some((side, piece, to)),
            "ordering is deterministic given identical fresh state in both runs, so the \
             same move must cause the cutoff either way"
        );
        assert!(
            deep_history.score(side, piece, to).abs()
                > shallow_history.score(side, piece, to).abs(),
            "starting with less of qdepth's shared budget remaining (2 vs {MAX_QUIESCENCE_DEPTH}) \
             must weigh the same cutoff more, not the same: shallow={}, deep={}",
            shallow_history.score(side, piece, to),
            deep_history.score(side, piece, to)
        );
    }
}
