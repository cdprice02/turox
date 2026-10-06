//! `Search`: negamax with fail-soft alpha-beta over iterative deepening,
//! quiescence search at the horizon, and MVV-LVA capture ordering.
//!
//! Mirrors `move_gen`'s naive-reference discipline: there's no perft
//! equivalent for search, so `tests/search_props.rs` checks this against an
//! independent, unpruned negamax reference rather than trusting a
//! read-through.

use crate::eval::{evaluate, Score};
use crate::search::aspiration;
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
#[derive(Debug)]
enum RootOutcome {
    /// The move loop finished every move at this depth. `best_move` is `None` only for
    /// the genuine terminal case: no legal moves at all.
    Completed(Score, PV),
    /// Every move came back at or below the window's floor, so the score is an
    /// upper bound and nothing here is worth reporting: no move proved better
    /// than the floor, so which of them is best is exactly what the search
    /// declined to work out. Carries that bound rather than a principal
    /// variation for the same reason.
    FailedLow(Score),
    /// A move came back at or above the window's ceiling, which is a cutoff and
    /// so a lower bound: the move is at least this good, and the moves after it
    /// were never tried. Worth less than it sounds at the root, where a ceiling
    /// is a guess about the previous score rather than a sibling's real
    /// refutation, which is why this reports a bound and not a move.
    FailedHigh(Score),
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
struct LoopCtx<'a> {
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
    /// The previous iteration's line from this node on, empty off it.
    previous: PvLine<'a>,
}

impl LoopCtx<'_> {
    /// Whether this is a principal variation node: a null window can only ever
    /// prove a bound, so a node searched with one is not on the line.
    ///
    /// Rests on every wide window in the search belonging to a PV node and
    /// every null window not. A technique that searched a PV node with a null
    /// window, or a non-PV node with a wide one, would break this silently.
    const fn is_pv(self) -> bool {
        is_pv_window(self.alpha, self.beta)
    }
}

/// Whether a window is wide enough for a principal-variation node, as opposed
/// to one of PVS's null-window probes.
const fn is_pv_window(alpha: Score, beta: Score) -> bool {
    beta > alpha + 1
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

/// The previous iteration's best line, from the node being searched onward.
///
/// A slice rather than a table indexed by ply: a move is only this position's
/// best guess while the path from the root still matches the line. Off it the
/// slice is empty, which is already the same as having no hint.
type PvLine<'a> = &'a [Option<Move>];

/// The move `line` expects at the node it was handed to, if it reaches here.
const fn pv_hint(line: PvLine<'_>) -> Option<Move> {
    line.first().copied().flatten()
}

/// `line` as seen from the child reached by `m`: the tail when `m` is what the
/// line expected here, empty otherwise.
///
/// Emptying it off-path is what keeps the hint about this position. Elsewhere
/// in the tree the same ply is a different position, whose move would be
/// promoted above the hash move on nothing but a coincidence of legality.
fn pv_child(line: PvLine<'_>, m: Move) -> PvLine<'_> {
    match line.split_first() {
        Some((&Some(expected), tail)) if expected == m => tail,
        _ => &[],
    }
}

/// A substitute source of the current time.
///
/// `Arc<dyn Fn>` rather than a plain `fn` pointer because a useful substitute
/// carries state (a time a caller moves forward by hand), and `Send + Sync`
/// because `Search` is moved onto its own thread while UCI's reader thread
/// keeps running. The indirection is paid once per iteration and on every
/// 2048th node, which is where the periodic abort check already samples the
/// clock, so it costs nothing measurable.
type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

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
    /// Where [`Search::now`] reads the current time, `None` meaning the real
    /// clock.
    ///
    /// A node budget cannot stand in for a deadline everywhere: the soft limit
    /// compares an estimated duration against the time left, so it only engages
    /// when `deadline` is set and there is no node-shaped equivalent of "two
    /// seconds from now". Substituting the clock is the only way to put that
    /// decision at an exact point, rather than inferring it from two wall-clock
    /// measurements taken under whatever load the machine happened to be under.
    clock: Option<Clock>,
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
    /// What this search's aspiration windows cost. Reported on
    /// [`SearchResult`] the same way the cutoff histograms are.
    aspiration: aspiration::Stats,
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
            aspiration: aspiration::Stats::default(),
            deadline: None,
            max_nodes: None,
            clock: None,
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

    /// Reads the current time from `clock` instead of the real one.
    ///
    /// Test-only, and deliberately not part of the public API: a caller that
    /// wants the search to stop at a particular point already has
    /// [`Search::with_deadline`] and [`Search::with_max_nodes`], and neither
    /// needs to lie about what time it is.
    #[cfg(test)]
    #[must_use]
    fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// The current time, from `clock` if one was substituted.
    fn now(&self) -> Instant {
        self.clock
            .as_ref()
            .map_or_else(Instant::now, |clock| clock())
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
                || self.deadline.is_some_and(|d| self.now() >= d)
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
        let started = self.now();
        let mut result = SearchResult {
            pv: [None; MAX_PV_PLY],
            score: 0,
            depth: 0,
            nodes: 0,
            time: Duration::ZERO,
            hashfull: None,
            negamax_cutoffs: CutoffStats::default(),
            quiescence_cutoffs: CutoffStats::default(),
            aspiration: aspiration::Stats::default(),
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

        // A local, not a field: a slice of it is passed into `&mut self`.
        let mut previous: PV = [None; MAX_PV_PLY];
        // The score beside that line, and from the same place: the last
        // iteration that finished. Kept separately from `result.score`, which
        // the abort path also writes and which starts at a score no iteration
        // reported, so reading the window's centre off it would centre one on a
        // number nothing measured.
        let mut previous_score: Option<Score> = None;

        'deepening: for depth in 1..=max_depth {
            let nodes_before_this_iteration = self.nodes();

            if let (Some(deadline), Some(elapsed), Some(nodes_last)) = (
                self.deadline,
                previous_iteration_elapsed,
                previous_iteration_nodes,
            ) {
                let remaining = deadline.saturating_duration_since(self.now());
                if should_skip_next_iteration(
                    elapsed,
                    nodes_last,
                    nodes_before_previous_iteration,
                    remaining,
                ) {
                    break;
                }
            }

            let iteration_started = self.deadline.is_some().then(|| self.now());

            let mut window = aspiration::Window::new(previous_score, depth);
            loop {
                let (alpha, beta) = window.bounds();
                // Before the attempt, so a discarded one can be charged for the
                // nodes it actually spent rather than a share of the iteration.
                let nodes_before_attempt = self.nodes();
                self.aspiration.attempts = self.aspiration.attempts.saturating_add(1);
                match self.search_root(board, depth, alpha, beta, &previous) {
                    RootOutcome::Completed(score, pv) => {
                        // Completed iterations only, the rule `result` follows too.
                        previous = pv;
                        previous_score = Some(score);
                        result = SearchResult {
                            pv,
                            score,
                            depth,
                            nodes: self.nodes(),
                            time: self.now().saturating_duration_since(started),
                            hashfull: self.tt.as_deref().map(Tt::hashfull),
                            negamax_cutoffs: self.negamax_cutoffs,
                            quiescence_cutoffs: self.quiescence_cutoffs,
                            aspiration: self.aspiration,
                        };
                        on_iteration_complete(&result);
                        break;
                    }
                    // Both sides are re-seeded, not just the one that gave
                    // way. A fail-soft bound is one-sided evidence: a fail-high
                    // says the score is at least `bound` and nothing about how
                    // much more, so the ceiling has to open all the way or the
                    // next attempt fails high again against the same wall. The
                    // side that held is what would trap it: leaving a ceiling
                    // one point above the new floor is a window a search cannot
                    // answer in, narrow enough to stop being a principal
                    // variation node at all, and the attempt after it lands in
                    // the same place.
                    RootOutcome::FailedHigh(bound) => {
                        self.aspiration.fail_high = self.aspiration.fail_high.saturating_add(1);
                        self.charge_escape(nodes_before_attempt);
                        window.failed_high(bound);
                    }
                    RootOutcome::FailedLow(bound) => {
                        self.aspiration.fail_low = self.aspiration.fail_low.saturating_add(1);
                        self.charge_escape(nodes_before_attempt);
                        window.failed_low(bound);
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
                                    time: self.now().saturating_duration_since(started),
                                    hashfull: self.tt.as_deref().map(Tt::hashfull),
                                    negamax_cutoffs: self.negamax_cutoffs,
                                    quiescence_cutoffs: self.quiescence_cutoffs,
                                    aspiration: self.aspiration,
                                };
                            }
                        }
                        break 'deepening;
                    }
                }
            }
            if let Some(started) = iteration_started {
                previous_iteration_elapsed = Some(self.now().saturating_duration_since(started));
                nodes_before_previous_iteration = previous_iteration_nodes;
                previous_iteration_nodes = Some(self.nodes() - nodes_before_this_iteration);
            }
        }
        result
    }

    /// Charges the nodes an abandoned attempt spent to the aspiration counters.
    ///
    /// A method rather than the same three lines in both escape arms, which is
    /// the one thing the two directions genuinely share: what an escape cost
    /// does not depend on which way it went.
    const fn charge_escape(&mut self, nodes_before: u64) {
        self.aspiration.wasted_nodes = self
            .aspiration
            .wasted_nodes
            .saturating_add(self.nodes().saturating_sub(nodes_before));
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
    /// cutoff to the killer table and the histogram, the last of these
    /// everywhere but the root (see the cutoff test's own comment). The window is passed in
    /// already negated, so the caller never repeats negamax's sign convention
    /// either. Whether a given call sits on the principal variation is tracked
    /// per move in a local, and decides one thing only: whether that move's
    /// line is copied into [`Search::pv`]. Move 0 inherits the node's own
    /// answer (a PV node's first move stays on the variation; a cut node's
    /// first move was never on it), every later move's initial probe is not on
    /// it (a null window can only prove a bound, never hand back an exact score
    /// worth recording), and a re-search is, whatever the node was, since a
    /// move that just proved itself better than everything found so far is a
    /// live candidate for the real line. The child is not told any of this: it
    /// reads its own PV-ness off the width of the window it was handed, which
    /// is the same answer by construction, since the probe is what makes that
    /// window narrow.
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
        F: FnMut(&mut Self, Move, LoopCtx<'_>) -> Option<Score>,
    {
        let node_is_pv = ctx.is_pv();
        let LoopCtx {
            ply,
            mut alpha,
            beta,
            depth,
            previous,
        } = ctx;
        // `search_root` is the only caller that reaches this loop at ply 0,
        // since it enters `negamax` one ply down and every deeper call inherits
        // that. Quiescence's loop is a different function and carries no such
        // rule: it does run at ply 0, from the root's own depth-0 handoff.
        let is_root = ply == 0;

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
                previous: pv_child(previous, m),
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
                // The root learns nothing from its own ceiling. A cutoff at an
                // interior node means a sibling refuted this line, which is
                // what the killer table and the histogram exist to remember; at
                // the root the ceiling is a guess about what the score will be,
                // so a move that beats it has refuted a prediction rather than
                // a move, and crediting it would teach ordering a fact about
                // the window. The break still stands: there is no reason to
                // search the remaining root moves once the window is escaped.
                if !is_root {
                    let cause = self.ordering.on_cutoff(ply, m, max);
                    self.negamax_cutoffs.record(i, cause);
                }
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

    /// Searches the child `m` leads to, with `board` on the repetition stack
    /// for the length of that search.
    ///
    /// `is_threefold_repetition` reads the stack as the path to the position
    /// being searched, so a node's own hash belongs there while its children
    /// are searched and not while it is checked itself. Pairing the push with
    /// the pop here is what keeps that true along the abort path as well,
    /// where an early return is easy to add and easy to leak through.
    fn descend<F>(&mut self, board: &Board, m: Move, search: F) -> Option<TaintedScore>
    where
        F: FnOnce(&mut Self, &Board) -> Option<TaintedScore>,
    {
        self.history.push(board.hash());
        let child = board.make_move(m);
        let result = search(self, &child);
        self.history.pop();
        result
    }

    /// [`Self::descend`], folding the child's repetition taint into `tainted`.
    ///
    /// Separate rather than a flag on `descend`, because the root genuinely
    /// has no taint to track: it never stores, so it has nothing to protect,
    /// and giving it a bit to discard would read as an oversight.
    fn descend_tracking_taint<F>(
        &mut self,
        board: &Board,
        m: Move,
        tainted: &mut bool,
        search: F,
    ) -> Option<Score>
    where
        F: FnOnce(&mut Self, &Board) -> Option<TaintedScore>,
    {
        let child = self.descend(board, m, search)?;
        *tainted |= child.tainted;
        Some(child.score)
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
    fn search_root(
        &mut self,
        board: &Board,
        depth: u8,
        alpha: Score,
        beta: Score,
        previous: PvLine<'_>,
    ) -> RootOutcome {
        let hint = pv_hint(previous);
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
            self.ordering.order(board, &mut drawn_moves, 0, hint);
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
        self.ordering.order(board, &mut moves, 0, hint);

        let outcome = self.alpha_beta_loop(
            &moves,
            LoopCtx {
                ply: 0,
                alpha,
                beta,
                depth,
                previous,
            },
            // The root searches every legal move, always. A move pruned here
            // is one the engine can never play, however good it was, so this
            // is permanent rather than a policy waiting to be filled in.
            |_, _, _| Verdict::Search,
            |s, m, c| {
                s.descend(board, m, |s, child| {
                    s.negamax(child, c.depth, c.ply, c.alpha, c.beta, c.previous)
                })
                .map(|r| r.score)
            },
        );

        if outcome.aborted {
            let best_so_far = outcome.best_move.map(|_| (outcome.max, self.pv[0]));
            return RootOutcome::Aborted { best_so_far };
        }
        // Classified before the history update, not after: a result that escaped
        // its window is a statement about the window and leaves the tables as it
        // found them, so only an attempt that lands inside one gets to move
        // them.
        if outcome.max <= alpha {
            return RootOutcome::FailedLow(outcome.max);
        }
        if outcome.max >= beta {
            return RootOutcome::FailedHigh(outcome.max);
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
        previous: PvLine<'_>,
    ) -> Option<TaintedScore> {
        let is_pv = is_pv_window(alpha, beta);
        let hint = pv_hint(previous);
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
        let runs = self.ordering.order(board, &mut moves, ply, hint);

        let mut any_child_tainted = false;
        let outcome = self.alpha_beta_loop(
            &moves,
            LoopCtx {
                ply,
                alpha,
                beta,
                depth,
                previous,
            },
            // Futility pruning and late move pruning join this match, each
            // behind its own gate. A reduction of zero is the policy declining
            // rather than a reduction of no plies, so it maps to `Search`.
            |_, _, ctx| match lmr::reduction(depth, ctx.searched, runs.is_quiet(ctx.index), is_pv) {
                0 => Verdict::Search,
                plies => Verdict::Reduce(plies),
            },
            |s, m, c| {
                s.descend_tracking_taint(board, m, &mut any_child_tainted, |s, child| {
                    s.negamax(child, c.depth, c.ply, c.alpha, c.beta, c.previous)
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

            self.ordering.order(board, &mut evasions, ply, None);

            let mut any_child_tainted = false;
            let outcome =
                self.quiescence_loop(&evasions, ply, alpha, beta, Score::MIN, |s, m, a, b| {
                    s.descend_tracking_taint(board, m, &mut any_child_tainted, |s, child| {
                        s.quiescence(child, a, b, ply + 1, qdepth.saturating_sub(1), None)
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

        self.ordering.order(board, &mut qmoves, ply, None);

        let mut any_child_tainted = false;
        let outcome = self.quiescence_loop(&qmoves, ply, alpha, beta, stand_pat, |s, m, a, b| {
            s.descend_tracking_taint(board, m, &mut any_child_tainted, |s, child| {
                s.quiescence(child, a, b, ply + 1, qdepth - 1, None)
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
    use std::sync::Mutex;
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
    fn ctx_at(depth: u8) -> LoopCtx<'static> {
        LoopCtx {
            ply: 1,
            alpha: -100,
            beta: 100,
            depth,
            previous: &[],
        }
    }

    #[test]
    fn a_wide_window_is_a_pv_node_and_a_null_window_is_not() {
        assert!(
            LoopCtx {
                ply: 0,
                alpha: -100,
                beta: 100,
                depth: 4,
                previous: &[],
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
                depth: 4,
                previous: &[],
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
                depth: 4,
                previous: &[],
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

    /// These two functions are everything the recursion does with the line.
    #[test]
    fn a_line_hints_only_the_move_it_expects_and_only_along_its_own_path() {
        let moves = legal_moves(&Board::start_pos());
        let (expected, other) = (moves[0], moves[1]);

        let line = [Some(expected), Some(other), Some(expected)];

        assert_eq!(
            pv_hint(&line),
            Some(expected),
            "the head is this node's hint"
        );
        assert_eq!(
            pv_child(&line, expected).len(),
            2,
            "following the expected move keeps the rest of the line"
        );
        assert_eq!(
            pv_hint(pv_child(&line, expected)),
            Some(other),
            "and the child's hint is the line's next move, not this one again"
        );

        assert!(
            pv_child(&line, other).is_empty(),
            "any other move leaves the path, and a node off the path gets no hint"
        );
        assert_eq!(
            pv_hint(pv_child(&line, other)),
            None,
            "which is what stops an unrelated position's move being promoted"
        );
    }

    /// Reaching the end of a line and never having one both read as no hint.
    #[test]
    fn a_line_that_has_run_out_hints_nothing() {
        let m = legal_moves(&Board::start_pos())[0];

        assert_eq!(pv_hint(&[]), None, "no line at all");
        assert!(
            pv_child(&[], m).is_empty(),
            "and descending from nothing stays nothing"
        );

        // `None` marks where a line stopped, as it does in `PV`.
        assert_eq!(pv_hint(&[None, Some(m)]), None, "a line that ended here");
        assert!(
            pv_child(&[None, Some(m)], m).is_empty(),
            "and nothing past its end is reachable, however the moves line up"
        );

        let single = [Some(m)];
        assert!(
            pv_child(&single, m).is_empty(),
            "the last move on the line leaves its child with none"
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
                .negamax(&board, 2, 0, -MATE, MATE, &[])
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

    /// A repetition's availability belongs to the path, and the table is keyed
    /// on the position alone, so a score drawn from a repeating subtree must
    /// not be stored. See ADR 0002.
    ///
    /// The control is load-bearing: without it this passes equally well when
    /// nothing is stored at all.
    #[test]
    fn a_score_from_a_repeating_subtree_is_never_stored() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let child = board.make_move(quiet);

        // Two prior occurrences, so the child searched here is the third.
        let repeating = vec![child.hash(), child.hash()];

        let mut control_tt = Tt::new(Tt::MIN_HASH_MB);
        Search::new(Vec::new())
            .with_tt(&mut control_tt)
            .negamax(&board, 2, 0, -MATE, MATE, &[])
            .expect("no abort condition is configured, so this can't return None");
        assert!(
            control_tt.probe(board.hash()).is_some(),
            "without a repetition this node does store, so the assertion below \
             is about the taint"
        );

        let mut tainted_tt = Tt::new(Tt::MIN_HASH_MB);
        Search::new(repeating)
            .with_tt(&mut tainted_tt)
            .negamax(&board, 2, 0, -MATE, MATE, &[])
            .expect("no abort condition is configured, so this can't return None");
        assert!(
            tainted_tt.probe(board.hash()).is_none(),
            "a score whose subtree contained a threefold repetition must not \
             reach the table"
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
    /// The position the widening path is tested against, and the reason it was
    /// chosen: its score moves far enough between consecutive deep iterations to
    /// land outside any window a narrow search would open, in both directions.
    /// Depths 5 through 10 report 105, 104, 155, 125, 105, 105 centipawns, so
    /// the seventh iteration escapes upward from a window centred on the sixth
    /// and the eighth escapes downward from one centred on the seventh.
    ///
    /// Measured with no transposition table and no cutoff history, which is how
    /// the tests here search. The same position under a warm table and the
    /// session's own history table reports a different curve, so a fixture
    /// picked by watching the engine play would not be the fixture these tests
    /// get.
    const WIDEN_FIXTURE: &str = "r1bqkbnr/pp2pppp/2n5/3p4/3NP3/8/PPP2PPP/RNBQKB1R w KQkq d6 0 5";

    // Node budgets for the fixed-depth searches of `WIDEN_FIXTURE` below, each
    // roughly twice what its own depth costs.
    //
    // A budget rather than no bound at all because what these searches cost is
    // a function of how well pruning works, and pruning that stops working
    // makes a search slow rather than wrong. Unbounded, that failure arrives as
    // a test that never finishes, which is the one result carrying no
    // information about what broke.
    //
    // Twice, not more: the counts are deterministic, so the margin is headroom
    // for retuning rather than for a loaded machine, and retuning moves a node
    // count by tens of percent rather than by multiples. A budget is also a
    // cost whenever it is reached, since the search runs until it gets there,
    // and these tests run once per mutant under `cargo mutants` on a two-core
    // runner. A margin wide enough to absorb a doubled tree is wide enough to
    // make a bounded test slower than the harness will wait for, which turns a
    // caught mutant back into the timeout the budget existed to prevent.

    /// Twice the 2.0M nodes depth 8 costs, which is the depth both the score
    /// curve and the attempt count below are measured at.
    const WIDEN_DEPTH_8_BUDGET: u64 = 4_100_000;

    /// Twice the 33k nodes depth 4 costs.
    const WIDEN_DEPTH_4_BUDGET: u64 = 70_000;

    /// Twice the 177k nodes depth 6 costs.
    const WIDEN_DEPTH_6_BUDGET: u64 = 360_000;

    /// Twice what a search of `WIDEN_FIXTURE` to `depth` costs, for the window
    /// sweep below.
    ///
    /// Per depth rather than one flat figure, because that sweep runs fourteen
    /// searches and a flat budget is spent fourteen times over. Sized for its
    /// deepest, a budget is forty times what depth 1 needs, so a mutant that
    /// inflates the tree makes every one of the fourteen spend the deepest
    /// search's allowance: around 18M nodes against the 1.9M the sweep actually
    /// costs. Scaled, the same worst case is under 4M.
    const fn widen_sweep_budget(depth: u8) -> u64 {
        match depth {
            0..=2 => 10_000,
            3 => 20_000,
            4 => WIDEN_DEPTH_4_BUDGET,
            5 => 150_000,
            6 => WIDEN_DEPTH_6_BUDGET,
            _ => 1_300_000,
        }
    }

    /// Searches to `depth` with aspiration windows on or off, and is the one
    /// place the switch is wired: the comparison below is written against this
    /// helper so that turning the windows on does not touch the test itself.
    /// Both answers come from the same full-window search while no narrower one
    /// exists.
    fn aspirated_score(board: &Board, depth: u8, _aspiration: bool) -> Score {
        Search::new(Vec::new())
            .with_max_nodes(widen_sweep_budget(depth))
            .search(board, depth)
            .score
    }

    /// Guards the fixture rather than the search. A widening re-search is only
    /// reachable from a position whose score actually escapes its window, and
    /// that is a property of the position: an evaluation change could flatten
    /// the curve and leave a passing test that no longer exercises the branch it
    /// exists for.
    ///
    /// The two floors differ because the two swings do. The rise is 51
    /// centipawns and is checked against 40, comfortably clear of any window
    /// half-width worth opening. The fall is 30 and is checked against 20,
    /// which is the looser of the two on purpose: it leaves the fall close to a
    /// plausible half-width, so a fixture with more room on the fail-low side
    /// is still worth finding. Neither floor is compared against the delta the
    /// implementation uses, which would only assert that someone edited two
    /// places at once.
    #[test]
    fn the_widen_fixture_still_moves_the_score_between_deep_iterations() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        let mut scores: Vec<(u8, Score)> = Vec::new();
        Search::new(Vec::new())
            .with_max_nodes(WIDEN_DEPTH_8_BUDGET)
            .search_with_info(&board, 8, |r| scores.push((r.depth, r.score)));

        assert_eq!(
            scores.last().map(|(depth, _)| *depth),
            Some(8),
            "the search stopped short of depth 8, so it reached WIDEN_DEPTH_8_BUDGET: the tree at this depth has outgrown the budget, {scores:?}"
        );

        let at = |d: u8| {
            scores
                .iter()
                .find(|(depth, _)| *depth == d)
                .map(|(_, s)| *s)
                .expect("every iteration up to the requested depth completes here")
        };
        let rise = at(7) - at(6);
        let fall = at(7) - at(8);
        assert!(
            rise > 40,
            "the seventh iteration must rise far enough to escape upward: {scores:?}"
        );
        assert!(
            fall > 20,
            "the eighth iteration must fall far enough to escape downward: {scores:?}"
        );
    }

    /// A narrower window may only change how much of the tree gets searched,
    /// never the answer: a result that escapes the window is re-searched wider
    /// until it does not, so the score that comes back is the one a full window
    /// would have returned.
    ///
    /// No transposition table on either side. A table is the one thing that can
    /// break this property for reasons of its own, by storing a bound derived
    /// from a narrow window and handing it to a later probe, and whether it does
    /// is a question with its own tests rather than this one's to answer.
    ///
    /// Stops at depth 7, which is two iterations past where a window would first
    /// narrow. Every depth here costs a full search on both sides, so the range
    /// is worth widening only once the two sides genuinely differ.
    #[test]
    fn a_narrow_window_changes_the_tree_and_not_the_score() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        for depth in 1..=7u8 {
            assert_eq!(
                aspirated_score(&board, depth, true),
                aspirated_score(&board, depth, false),
                "depth {depth}: a narrow window returned a different score than a full one"
            );
        }
    }
    /// Both escape directions, on one position, asserted against a window placed
    /// deliberately on the wrong side of the true score.
    ///
    /// Worth a test of its own rather than trusting the comparison by reading
    /// it: a floor and a ceiling crossed with a lower and an upper bound is the
    /// shape that has produced scrambled results in this engine before, and a
    /// swapped pair still typechecks, still terminates, and still returns a
    /// number. What it does instead is widen away from the answer, which looks
    /// like a slow search rather than a wrong one.
    ///
    /// Each window is placed relative to the score a full-window search
    /// reports, so the test does not encode what the position is worth and
    /// survives any evaluation change that keeps it a search rather than a
    /// rewrite. Each is ten points wide rather than one: a one-point window
    /// would escape just as reliably and is also a window no caller may open,
    /// since nothing can be answered inside it, so the test would be resting on
    /// a shape the search is entitled to reject.
    #[test]
    fn a_score_outside_its_window_is_reported_on_the_side_it_escaped() {
        let board = capture_and_quiet_position();
        let depth = 4;
        let full = Search::new(Vec::new()).search_root(&board, depth, -MATE, MATE, &[]);
        let RootOutcome::Completed(truth, _) = full else {
            panic!("a full window cannot be escaped, got {full:?}");
        };

        // Ceiling below the answer: the search proves it can do better than the
        // ceiling and stops there, so the bound it reports is a floor under the
        // true score rather than the score itself.
        let ceiling = truth - 50;
        let high = Search::new(Vec::new()).search_root(&board, depth, ceiling - 10, ceiling, &[]);
        let RootOutcome::FailedHigh(bound) = high else {
            panic!(
                "a ceiling of {ceiling} below the true score {truth} must fail high, got {high:?}"
            );
        };
        assert!(
            bound >= ceiling,
            "a fail-high reports a lower bound, so it cannot sit under the ceiling it escaped: \
             bound {bound}, ceiling {ceiling}"
        );

        // Floor above the answer: nothing reaches the floor, so the bound is a
        // ceiling over the true score and no move earned a report.
        let floor = truth + 50;
        let low = Search::new(Vec::new()).search_root(&board, depth, floor, floor + 10, &[]);
        let RootOutcome::FailedLow(bound) = low else {
            panic!("a floor of {floor} above the true score {truth} must fail low, got {low:?}");
        };
        assert!(
            bound <= floor,
            "a fail-low reports an upper bound, so it cannot sit above the floor nothing \
             reached: bound {bound}, floor {floor}"
        );
    }

    /// A window wide enough to hold the answer is not escaped, which is the
    /// control the two failures above are only meaningful against: a
    /// classification that answered one of them unconditionally would satisfy
    /// half of that test, and this one catches it.
    #[test]
    fn a_score_inside_its_window_completes() {
        let board = capture_and_quiet_position();
        let depth = 4;
        let full = Search::new(Vec::new()).search_root(&board, depth, -MATE, MATE, &[]);
        let RootOutcome::Completed(truth, _) = full else {
            panic!("a full window cannot be escaped, got {full:?}");
        };

        let narrow =
            Search::new(Vec::new()).search_root(&board, depth, truth - 30, truth + 30, &[]);
        let RootOutcome::Completed(score, pv) = narrow else {
            panic!("a window centred on the true score must complete, got {narrow:?}");
        };
        assert_eq!(
            score, truth,
            "a window the score sits inside returns the same score a full one does"
        );
        assert!(
            pv[0].is_some(),
            "a completed root iteration always reports a move"
        );
    }
    /// Every iteration is one attempt unless its window was escaped, so the
    /// attempts past the depth reached are exactly the escapes. Worth asserting
    /// because the counter is the only thing that reports a re-search happened,
    /// and a miscount there is invisible: the search still returns the right
    /// move, and the number that would have said what it cost is simply wrong.
    #[test]
    fn attempts_past_the_depth_reached_are_exactly_the_escapes() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        // Depth 8 rather than deeper: it is the shallowest depth at which this
        // fixture escapes its window in both directions, so it exercises the
        // whole counter for 2.0M nodes where depth 9 wants 3.6M for one more
        // escape of a direction already covered.
        let depth = 8;
        let result = Search::new(Vec::new())
            .with_max_nodes(WIDEN_DEPTH_8_BUDGET)
            .search(&board, depth);
        assert_eq!(
            result.depth, depth,
            "the search stopped at depth {} of {depth}, so it reached WIDEN_DEPTH_8_BUDGET: the tree at this depth has outgrown the budget",
            result.depth
        );

        let stats = result.aspiration;
        let escapes = stats.fail_low + stats.fail_high;

        assert!(
            escapes > 0,
            "this fixture is chosen to escape its window, so a run that never does \
             is measuring nothing: {stats:?}"
        );
        assert_eq!(
            stats.attempts,
            u32::from(depth) + escapes,
            "one attempt per iteration, plus one per escape: {stats:?}"
        );
        assert!(
            stats.wasted_nodes > 0,
            "an escape throws away the attempt that caused it, so it cost nodes: {stats:?}"
        );
    }

    /// A clock the test moves by hand.
    ///
    /// The soft limit compares an estimated duration against the time left
    /// before the deadline, so driving it from a real clock means asserting
    /// against two wall-clock measurements taken at different moments on a
    /// shared machine. That is a comparison of load conditions rather than of
    /// behaviour, and it fails in both directions: an inflated measurement
    /// makes the budget generous enough for the iteration under test, and a
    /// run that meets heavier load than the measurement cannot finish the
    /// iteration before it either.
    #[derive(Clone)]
    struct ManualClock(Arc<Mutex<Instant>>);

    impl ManualClock {
        fn new(at: Instant) -> Self {
            Self(Arc::new(Mutex::new(at)))
        }

        /// Moves the clock forward, charging the time to whatever the search is
        /// in the middle of. Called from `search_with_info`'s callback, which
        /// fires after an iteration completes and before its elapsed time is
        /// read, so an advance there is exactly "this iteration cost that long".
        fn advance(&self, by: Duration) {
            *self
                .0
                .lock()
                .expect("a test holding this lock never panics") += by;
        }

        fn source(&self) -> Arc<dyn Fn() -> Instant + Send + Sync> {
            let inner = Arc::clone(&self.0);
            Arc::new(move || *inner.lock().expect("a test holding this lock never panics"))
        }
    }

    /// Iterative deepening must not start an iteration it cannot finish before
    /// the deadline.
    ///
    /// Two iterations are allowed to complete, so the decision runs against
    /// `should_skip_next_iteration`'s measured-ratio estimate rather than its
    /// no-data fallback, and the second one is charged almost the entire
    /// budget. The estimate is floored at a ratio of one, so a 58-second
    /// iteration cannot be believed to fit in the second that remains no matter
    /// what the node counts did, which is what keeps the assertion exact rather
    /// than dependent on this position's growth.
    #[test]
    fn the_soft_limit_skips_an_iteration_that_cannot_finish_in_the_time_left() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        let start = Instant::now();
        let clock = ManualClock::new(start);
        let mut search = Search::new(Vec::new())
            .with_deadline(start + Duration::from_secs(60))
            .with_max_nodes(WIDEN_DEPTH_6_BUDGET)
            .with_clock(clock.source());

        let mut completed = Vec::new();
        let result = search.search_with_info(&board, 6, |r| {
            completed.push(r.depth);
            clock.advance(if r.depth == 2 {
                Duration::from_secs(58)
            } else {
                Duration::from_secs(1)
            });
        });

        assert_eq!(
            completed,
            vec![1, 2],
            "depth 3 must never be started: 58 seconds of measured cost does not fit in the 1 second left"
        );
        assert_eq!(
            result.depth, 2,
            "the skipped iteration must leave depth 2's own result in place"
        );
    }

    /// The control for the test above: a budget nothing can exhaust must not
    /// stop the loop early. Without this, a soft limit that always fired would
    /// pass that test and be caught by nothing in it.
    #[test]
    fn the_soft_limit_leaves_a_search_with_time_to_spare_alone() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        let start = Instant::now();
        let clock = ManualClock::new(start);
        let mut search = Search::new(Vec::new())
            .with_deadline(start + Duration::from_secs(3600))
            .with_max_nodes(WIDEN_DEPTH_4_BUDGET)
            .with_clock(clock.source());

        let result = search.search_with_info(&board, 4, |_| {
            clock.advance(Duration::from_secs(1));
        });

        assert_eq!(
            result.depth, 4,
            "four iterations costing a second each cannot exhaust an hour, so every one must run"
        );
    }

    /// The counters stay at rest when the technique never engages, which is the
    /// control for the test above: an `attempts` that counted something other
    /// than root calls would still satisfy the arithmetic there while being
    /// wrong here.
    #[test]
    fn a_search_too_shallow_to_aspirate_reports_one_attempt_per_iteration() {
        let board = Board::try_from_fen(WIDEN_FIXTURE).expect("fixture FEN is valid");
        let depth = aspiration::MIN_DEPTH - 1;
        let stats = Search::new(Vec::new())
            .with_max_nodes(WIDEN_DEPTH_4_BUDGET)
            .search(&board, depth)
            .aspiration;

        assert_eq!(
            stats,
            aspiration::Stats {
                attempts: u32::from(depth),
                fail_low: 0,
                fail_high: 0,
                wasted_nodes: 0,
            },
            "below the depth floor every window opens full and cannot be escaped"
        );
    }
}
