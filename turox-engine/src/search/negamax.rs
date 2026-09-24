//! `Search`: negamax with fail-soft alpha-beta over iterative deepening,
//! quiescence search at the horizon, and MVV-LVA capture ordering.
//!
//! Mirrors `move_gen`'s naive-reference discipline: there's no perft
//! equivalent for search, so `tests/search_props.rs` checks this against an
//! independent, unpruned negamax reference rather than trusting a
//! read-through.

use crate::eval::{evaluate, Score, PIECE_VALUES};
use crate::search::cutoff_history::CutoffHistory;
use crate::search::draw::{is_draw, is_fifty_move_draw, is_threefold_repetition};
use crate::search::killers::KillerTable;
use crate::search::lmr;
use crate::search::time::should_skip_next_iteration;
use crate::search::tt::Tt;
use crate::search::MAX_TRACKED_PLY;
use std::cmp::Reverse;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use turox_chess::board::Board;
use turox_chess::move_gen::attacks::in_check;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::move_gen::move_list::MoveList;
use turox_chess::types::Move;
use turox_chess::MoveFlags;
use turox_chess::Piece;
use turox_macros::Ordinal;
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

/// Ply bound for [`PV`] and `Search::pv`, deliberately not [`MAX_TRACKED_PLY`]: that
/// constant's own justification is defensive slack for pathological recursion depth
/// (quiescence's uncapped in-check evasion chases), which has nothing to do with how
/// deep a *reported* principal variation can realistically go. A PV line's real ceiling
/// is `search_with_info`'s own iterative-deepening bound (`uci::session`'s
/// `DEFAULT_MAX_DEPTH` is `64`), so bounding it there instead saves the same factor of
/// `MAX_TRACKED_PLY / MAX_PV_PLY` squared on `Search::pv`, since that one is triangular
/// (`[PV; MAX_PV_PLY]`, one row per ply) rather than flat.
const MAX_PV_PLY: usize = 64;

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
    pub by_cause: [u64; CutoffCause::ALL.len()],
    /// `cutoff_index[i]` counts cutoffs at move index `i`, for `i < 15`. Index `15` is an
    /// overflow bucket for the 16th move onward, so a long tail of rare late cutoffs can't
    /// make this array itself unbounded; summing the whole array always equals
    /// `fail_high_nodes`.
    pub cutoff_index: [u64; 16],
}

impl CutoffStats {
    /// Records a cutoff at `index` (0-based position in the already-ordered
    /// move list), attributed to `cause`.
    fn record(&mut self, index: usize, cause: CutoffCause) {
        self.fail_high_nodes += 1;
        self.cutoff_index[index.min(15)] += 1;
        self.by_cause[cause.index()] += 1;
    }
}

/// A principal variation, one move per ply starting from wherever it was read: `None`
/// past however deep the line actually runs, the same "untouched slot" convention
/// `Search`'s other per-ply tables carry. Sized to [`MAX_PV_PLY`], not
/// [`MAX_TRACKED_PLY`]; see that constant's own doc for why the two bounds differ.
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
#[derive(Debug, Clone, Copy, Eq)]
pub struct SearchResult {
    /// The principal variation searched
    pub pv: PV,
    /// Side-to-move-relative, same convention as [`evaluate`].
    pub score: Score,
    /// The depth actually completed; see the struct doc for when this is
    /// less than the requested `max_depth`.
    pub depth: u8,
    /// Total nodes visited (negamax and quiescence both count) across every
    /// completed and aborted iteration of this call.
    pub nodes: u64,
    /// Wall-clock time spent since [`Search::search`] was entered, covering
    /// every iteration so far rather than just the one this result came from.
    /// Measured even when no deadline is set, since UCI reports it regardless
    /// of what bounded the search.
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
impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        self.pv == other.pv
            && self.score == other.score
            && self.depth == other.depth
            && self.nodes == other.nodes
            && self.hashfull == other.hashfull
            && self.negamax_cutoffs == other.negamax_cutoffs
            && self.quiescence_cutoffs == other.quiescence_cutoffs
    }
}

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
// exists. In a normal build nothing returns these yet, and this expectation
// fails the build as soon as something does, so it removes itself.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the policy returning these arrives with the first pruning technique; the loop's handling of them is already tested"
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "read by the policy that arrives with the first pruning technique; this expectation fails the build once one does"
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
    /// without one. Callers feed this straight into [`Search::update_cutoff_history`],
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
    /// Scratch for one ordering pass: every move paired with the key it sorts
    /// on, so the key is computed once per move instead of once per comparison.
    ///
    /// Lives here rather than in [`Self::order_moves`]'s own frame because one
    /// buffer is enough, ordering never re-entering itself, and a two-kilobyte
    /// local inlined into `negamax` would ride on every frame of a deep
    /// recursion. Only the first `moves.len()` entries are ever live: the tail
    /// holds whatever a longer list left behind and is never read.
    prioritized_moves: [(Reverse<(MovePriority, Score)>, Move); MoveList::CAPACITY],
    /// `None` by default (`negamax` searches with no transposition table at all, the same
    /// as before one existed); set via [`Search::with_tt`]. Borrowed, not owned: the table
    /// lives in `uci::session::run` (like `history` conceptually does, though `history` is
    /// actually copied in), so it survives across the separate `Search` a later `go` call
    /// rebuilds, rather than starting cold every time.
    tt: Option<&'a mut Tt>,
    /// This search tree's killer moves, per ply.
    ///
    /// Tree-scoped rather than session-scoped like `tt`: a killer is a
    /// refutation specific to this tree's shape, not a fact about a position
    /// that is still true the next time `go` runs. By the same reasoning it is
    /// not reset between iterative-deepening iterations, which re-explore one
    /// tree at increasing depth.
    killers: KillerTable,
    /// The move this ply's move loop ordered by, if any: the transposition
    /// table's move for the position being searched there.
    ///
    /// Per ply for the same reason `killers` is, and clamped the same way: one
    /// position is being searched at each ply at a time, so a ply's slot is
    /// only ever about its own node, and a child writing its own slot cannot
    /// disturb an ancestor's.
    ///
    /// It holds what the loop *ordered by*, not what the table knows. Those
    /// differ: quiescence never orders by a hash move even where the table has
    /// one, so it writes `None` and its cutoffs are never attributed to the
    /// table. Attribution has to follow the ordering, or it credits a technique
    /// that did not put the move first.
    hash_moves: [Option<Move>; MAX_TRACKED_PLY],
    /// The triangular PV table.
    pv: [PV; MAX_PV_PLY],
    /// The previous iterative-deepening iteration's completed best line, one move
    /// per ply, frozen once that iteration finishes and consulted (never mutated)
    /// by [`Search::move_priority`] for the whole of the next. `None` past however
    /// deep that line actually ran, the same convention `hash_moves`/`killers`
    /// carry for an untouched ply, and unconditionally `None` at ply 0 of the very
    /// first iteration, when there is no previous iteration yet.
    ///
    /// Deliberately flat, not the triangular structure PVS's own in-progress PV
    /// tracking needs: once an iteration completes, its whole best line collapses
    /// to exactly one move per ply, so this only ever needs to answer "what was
    /// the move at this ply," never "what is the rest of the line from here."
    /// Trusted ahead of `hash_moves` for that ply (see [`MovePriority`]'s own
    /// doc): a hash move can be a different, more recently searched, position's
    /// entry if the table's replacement scheme has since overwritten this one, but
    /// this line is this search's own, uncorrupted record of what actually played
    /// out.
    previous_pv_line: [Option<Move>; MAX_TRACKED_PLY],
    /// Quiet-move ordering scores independent of any one search tree, unlike `killers`:
    /// it accumulates over many more nodes than two killer slots ever see, so it is
    /// threaded in from `uci::session::run` rather than owned here, the same way `tt` is.
    /// `None` by default (`negamax` orders and records quiets exactly as if history
    /// didn't exist): set via [`Search::with_cutoff_history`].
    cutoff_history: Option<&'a mut CutoffHistory>,
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
            prioritized_moves: [(Reverse((MovePriority::Quiet, 0)), Move::SENTINEL);
                MoveList::CAPACITY],
            tt: None,
            killers: KillerTable::new(),
            hash_moves: [None; MAX_TRACKED_PLY],
            pv: [[None; MAX_PV_PLY]; MAX_PV_PLY],
            previous_pv_line: [None; MAX_TRACKED_PLY],
            cutoff_history: None,
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
        self.cutoff_history = Some(cutoff_history);
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

    /// Records what this ply's move loop is ordering by, which every loop must
    /// do before running, including the loops that order by nothing.
    ///
    /// Passing `None` is not a formality: it is what keeps a ply's slot about
    /// its own node rather than whatever last occupied that depth.
    fn set_hash_move(&mut self, ply: u8, m: Option<Move>) {
        self.hash_moves[usize::from(ply).min(MAX_TRACKED_PLY - 1)] = m;
    }

    /// This ply's hash move; see [`Self::set_hash_move`].
    fn hash_move(&self, ply: u8) -> Option<Move> {
        self.hash_moves[usize::from(ply).min(MAX_TRACKED_PLY - 1)]
    }

    /// This ply's move in the previous iteration's completed best line; see
    /// [`Self::previous_pv_line`]. No setter alongside this one the way
    /// `hash_move` has `set_hash_move`: populating this is PVS's own job, done
    /// once per completed iteration from whatever in-progress structure it
    /// builds the line in, not once per node the way a hash move is recorded.
    fn previous_pv_move(&self, ply: u8) -> Option<Move> {
        self.previous_pv_line[usize::from(ply).min(MAX_TRACKED_PLY - 1)]
    }

    /// Called whenever a move causes a beta cutoff: updates the killer and
    /// mate-killer tables and reports which ordering technique put `m` early
    /// enough to cut off.
    ///
    /// Classification lives here rather than at the call site because this is
    /// already the one place that knows what each technique holds, and the
    /// techniques queued behind killers and mate killers (history,
    /// countermoves) each add their own table to `Search` and a branch here,
    /// not another return value threaded out to the loop.
    ///
    /// The hash move is read from [`Self::hash_move`] like the killers are
    /// read from their own table, so every ordering input this classifies
    /// against lives on `Self`. Checked first, because a stored move that is
    /// *also* a killer was tried early because the table named it, and
    /// crediting the killer table would overstate what it does. The mate
    /// killer is checked next, ahead of the ordinary killer table, matching
    /// [`MovePriority`]'s own ranking: a move can sit in both tables at once
    /// (mate killers are recorded into the ordinary table too, since neither
    /// recording is conditional on the other), and when it does, the mate
    /// killer is the more specific, higher-value fact about it.
    ///
    /// `score` is `m`'s own resulting score, the same value the caller's
    /// `max` already holds at the point it decides `alpha >= beta`: what
    /// decides whether this cutoff also populates the mate-killer table,
    /// via [`is_mate_score`].
    ///
    /// Recording is a no-op for anything that isn't a plain quiet move:
    /// captures and promotions are already ordered by MVV-LVA/material gain,
    /// so recording them as killers would duplicate that and waste a slot on
    /// information the ordering already has. Such a move can still *cause* a
    /// cutoff, and is still classified.
    fn note_cutoff_move(&mut self, ply: u8, m: Move, score: Score) -> CutoffCause {
        // Recording happens whatever the cause turns out to be. Skipping it for
        // a hash move would quietly change which tables hold what, and so the
        // move ordering and the tree, which classification must not.
        let (was_already_a_killer, was_already_the_mate_killer) = if m.flags().is_material_neutral()
        {
            let held = self.killers.record(ply, m, is_mate_score(score));
            (held.killer, held.mate_killer)
        } else {
            (false, false)
        };

        if self.hash_move(ply) == Some(m) {
            CutoffCause::HashMove
        } else if was_already_the_mate_killer {
            CutoffCause::MateKiller
        } else if was_already_a_killer {
            CutoffCause::Killer
        } else {
            CutoffCause::Other
        }
    }

    /// Feeds every material-neutral move this node's loop tried into `self.cutoff_history`:
    /// a bonus for whichever one caused the cutoff (if any, and if it's material-neutral), a
    /// malus for every other material-neutral move searched before it. If the loop never
    /// found a cutoff at all (an all-node), every material-neutral move in `moves` gets the
    /// malus instead, since none of them caused one. A no-op when `self.cutoff_history` is
    /// `None`, same convention as `self.tt`.
    ///
    /// `moves` is the same already-ordered list the caller's own loop iterated
    /// (`search_root`'s, `negamax`'s, or quiescence's evasion loop's), so this needs no
    /// separate bookkeeping of which quiets were tried: everything up to `cutoff_index` (or
    /// everything, if there wasn't one) was tried by construction. `cutoff_index` is `None`
    /// when the loop ran to completion without a cutoff, `Some(i)` when it broke at index `i`.
    ///
    /// `depth` is the caller's own remaining-depth parameter for `search_root`/`negamax`;
    /// quiescence's evasion loop has no such quantity and passes a depth-equivalent derived
    /// from `qdepth`'s own countdown instead; see `quiescence`'s own doc for `qdepth`.
    /// Called independently of, and after, `note_cutoff_move`: the two tables answer
    /// different questions over the same event stream and neither is scoped by the other's
    /// outcome.
    #[expect(
        clippy::expect_used,
        reason = "moves is the position's own already-ordered move list, so a move's from-square always holds the piece that made it"
    )]
    fn update_cutoff_history(
        &mut self,
        board: &Board,
        moves: &MoveList,
        cutoff_index: Option<usize>,
        depth: u8,
    ) {
        let Some(cutoff_history) = self.cutoff_history.as_deref_mut() else {
            return;
        };
        let side = board.side_to_move();
        let searched_end = cutoff_index.unwrap_or(moves.len());
        for &searched in &moves[..searched_end] {
            if searched.flags().is_material_neutral() {
                let piece = board
                    .piece_at(searched.from())
                    .expect("move has a piece")
                    .piece();
                cutoff_history.record_no_cutoff(side, piece, searched.to(), depth);
            }
        }
        if let Some(i) = cutoff_index {
            let cutoff_move = moves[i];
            if cutoff_move.flags().is_material_neutral() {
                let piece = board
                    .piece_at(cutoff_move.from())
                    .expect("move has a piece")
                    .piece();
                cutoff_history.record_cutoff(side, piece, cutoff_move.to(), depth);
            }
        }
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
                let cause = self.note_cutoff_move(ply, m, max);
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
                let cause = self.note_cutoff_move(ply, m, max);
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
        self.set_hash_move(0, None);

        let mut pv: PV = [None; MAX_PV_PLY];

        if is_draw(board, &self.history, board.hash()) {
            let mut drawn_moves = legal_moves(board);
            if drawn_moves.is_empty() {
                return RootOutcome::Completed(0, pv);
            }
            self.order_moves(board, &mut drawn_moves, 0);
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

        // Before ordering, not after: `order_moves` sorts by a coarse priority
        // class, so shuffling first is what decides which of several moves
        // sharing a class gets tried first, while still leaving the ordering
        // itself intact.
        self.shuffle_root_moves(&mut moves);
        self.order_moves(board, &mut moves, 0);

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
        self.update_cutoff_history(board, &moves, outcome.cutoff_index, depth);
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
    /// this node's own move loop, so it survives long enough to feed `order_moves`.
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
        self.set_hash_move(ply, tt_move);
        let runs = self.order_moves(board, &mut moves, ply);

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
        self.update_cutoff_history(board, &moves, outcome.cutoff_index, depth);

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
        self.set_hash_move(ply, None);

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

            self.order_moves(board, &mut evasions, ply);

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
            self.update_cutoff_history(
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

        self.order_moves(board, &mut qmoves, ply);

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

impl Search<'_> {
    /// Orders `moves` in place, most promising first, so alpha-beta prunes more
    /// of the tree: MVV-LVA (most valuable victim, least valuable attacker)
    /// among captures, promotions interleaved onto that same material scale,
    /// ahead of quiet moves, since a capture or promotion that wins the most
    /// material is most likely to hold up and cause a beta cutoff early.
    /// `ply`'s previous-iteration PV move and hash move rank ahead of all of
    /// that; see [`Self::move_priority`]. `ply`'s two killer slots rank below
    /// captures but above the remaining quiet moves; see [`Self::move_priority`].
    /// `self.cutoff_history`, when `Some`, breaks ties among the remaining
    /// quiets by their stored score; `None` orders them exactly as if it
    /// didn't exist (every untouched quiet ties at `0`).
    ///
    /// En passant's victim isn't actually on `m.to()`; scored as `0` (an
    /// equal-value trade) rather than looking up the true victim square, an
    /// accepted ordering approximation since it only affects search order, not
    /// legality or correctness.
    ///
    /// Decorate, sort, undecorate, by way of [`Self::prioritized_moves`].
    /// Sorting the move list directly has `sort_unstable_by_key` recompute
    /// [`Self::move_priority`] on every comparison rather than once per move,
    /// and that key does a killer lookup, a cutoff-history lookup and
    /// piece-value arithmetic each time it runs.
    ///
    /// The sort keys on the pair's priority alone and never on the pair, since
    /// the tuple's derived ordering would add the move itself as a final
    /// tiebreak and so break ties among equal priorities differently from a
    /// sort over the moves on their own.
    ///
    /// Returns where each priority's run falls in the ordered result, which is
    /// the half of the work a plain sort computes and throws away.
    fn order_moves(&mut self, board: &Board, moves: &mut MoveList, ply: u8) -> PriorityRuns {
        // Counted here rather than scanned off the sorted result, because
        // this loop already visits every move.
        let mut counts = [0u16; MovePriority::COUNT];
        for (i, &m) in moves.iter().enumerate() {
            let priority = self.move_priority(board, m, ply);
            self.prioritized_moves[i] = (Reverse(priority), m);
            counts[priority.0.rank()] += 1;
        }

        // The live prefix only: past it sit the leavings of a longer list.
        self.prioritized_moves[..moves.len()].sort_unstable_by_key(|entry| entry.0);

        for i in 0..moves.len() {
            moves[i] = self.prioritized_moves[i].1;
        }

        PriorityRuns::from_counts(&counts)
    }

    /// Captures and promotions interleave on one material-gain scale rather than
    /// promotions getting a fixed rank: a capturing promotion (e.g. a pawn
    /// taking a rook while queening) is worth strictly more than the same
    /// promotion alone, by the captured piece's value, and only a shared scale
    /// can express that. `PIECE_VALUES` gives every promotion piece strictly
    /// more value than a pawn (the smallest gain, to a knight, is still 220), so
    /// `gain` can never be zero or negative for a promotion: it always resolves
    /// to `WinningCapture`, never `EqualCapture` or `LosingCapture`, and because
    /// that check happens before any killer-table lookup, a promotion can never
    /// fall through to `Quiet`, `Killer`, or `MateKiller` either, with or
    /// without a capture attached.
    ///
    /// Returns `(MovePriority, Score)`, both halves naturally bigger-is-better and
    /// neither wrapped in `Reverse`: `MovePriority` is declared worst-first so its
    /// derived `Ord` already agrees with `Score`'s own ordering, and
    /// [`Self::order_moves`] applies the one flip `sort_unstable_by_key`'s
    /// ascending sort needs at its own call site, not here. Tiers with no real
    /// number to break ties by (`PrincipalVariation`, `Hash`, `EqualCapture`,
    /// `Killer`) carry a placeholder `0` that never actually competes against
    /// anything: every move landing in one of those tiers already ties on the
    /// first element of the tuple by definition of "same tier."
    ///
    /// `ply` scopes every per-node lookup this makes (`self.previous_pv_move`,
    /// `self.hash_move`, `self.killers`); `self.cutoff_history` isn't
    /// ply-scoped, the same way it isn't ply-scoped on `self` itself.
    #[expect(
        clippy::expect_used,
        reason = "the flags decide which arm runs, so a promotion arm always has a promotion piece and a capture arm always has a victim; the from-square always holds the moving piece"
    )]
    fn move_priority(&self, board: &Board, m: Move, ply: u8) -> (MovePriority, Score) {
        if Some(m) == self.previous_pv_move(ply) {
            return (MovePriority::PrincipalVariation, 0);
        }
        if Some(m) == self.hash_move(ply) {
            return (MovePriority::Hash, 0);
        }

        let flags = m.flags();
        let promotion_delta = || {
            PIECE_VALUES[flags
                .promotion_piece()
                .expect("move is a promotion")
                .index()]
                - PIECE_VALUES[Piece::Pawn.index()]
        };
        let capture_delta = || {
            let attacker = board.piece_at(m.from()).expect("move has a piece").piece();
            let victim = board
                .piece_at(m.to())
                .expect("capture has a victim")
                .piece();
            PIECE_VALUES[victim.index()] - PIECE_VALUES[attacker.index()]
        };
        let score_delta = match flags {
            MoveFlags::Quiet
            | MoveFlags::DoublePawnPush
            | MoveFlags::KingCastle
            | MoveFlags::QueenCastle => {
                if self.killers.holds_mate(ply, m) {
                    return (MovePriority::MateKiller, 0);
                }
                if self.killers.holds(ply, m) {
                    return (MovePriority::Killer, 0);
                }
                let history_score = self.cutoff_history.as_deref().map_or(0, |history| {
                    let piece = board.piece_at(m.from()).expect("move has a piece").piece();
                    history.score(board.side_to_move(), piece, m.to())
                });
                return (MovePriority::Quiet, history_score);
            }
            MoveFlags::Capture => capture_delta(),
            MoveFlags::EnPassant => 0,
            MoveFlags::PromoteKnight
            | MoveFlags::PromoteBishop
            | MoveFlags::PromoteRook
            | MoveFlags::PromoteQueen => promotion_delta(),
            MoveFlags::PromoteCaptureKnight
            | MoveFlags::PromoteCaptureBishop
            | MoveFlags::PromoteCaptureRook
            | MoveFlags::PromoteCaptureQueen => capture_delta() + promotion_delta(),
        };
        match score_delta.cmp(&0) {
            std::cmp::Ordering::Greater => (MovePriority::WinningCapture, score_delta),
            std::cmp::Ordering::Equal => (MovePriority::EqualCapture, 0),
            std::cmp::Ordering::Less => (MovePriority::LosingCapture, score_delta),
        }
    }
}

/// Ranked bottom to top, worst move first: `#[derive(PartialOrd, Ord)]` on the
/// enum compares by declaration order, so this declaration *is* the ranking,
/// not a lookup table alongside it. Declared worst-first, the reverse of how
/// it reads in prose, so a move's raw `Ord` already agrees with `Score`'s own
/// "bigger is better": [`Search::move_priority`] returns `(MovePriority,
/// Score)` with neither half wrapped in `Reverse`, and the one flip
/// `sort_unstable_by_key`'s ascending sort needs happens once, in
/// [`Search::order_moves`], instead of being smuggled into half the tuple.
/// Carries no payload of its own: `move_priority`'s tuple has a second element
/// for that, so the fine-grained tiebreak *within* a tier (MVV-LVA's delta
/// among captures, [`CutoffHistory`]'s score among quiets) has one shared
/// place to live rather than a separate payload per variant that needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Ordinal)]
#[repr(u8)]
enum MovePriority {
    /// A capture losing material, ordered by how much. Below `Quiet` because a
    /// move that hangs a piece is worse than an untried ordinary move.
    LosingCapture,
    /// Everything not otherwise classified.
    Quiet,
    /// A quiet move that caused a beta cutoff at a sibling of this ply.
    Killer,
    /// A quiet move that refuted a sibling *with a mate score*. Ranked above
    /// ordinary killers because a forced mate is worth more than material.
    /// See [`Search::note_cutoff_move`].
    MateKiller,
    /// An even trade. Gain is exactly `0` by definition, so unlike the winning and losing
    /// tiers it needs no tiebreak beyond ordinary declaration order.
    EqualCapture,
    /// A capture winning material, ordered by how much; see [`Search::move_priority`]'s
    /// own doc for why the ordering value lives in the tuple this enum is half of, not a
    /// payload on the variant.
    WinningCapture,
    /// The transposition table's stored move for this position: already proved
    /// best by a deeper or equal search, so nothing cheaper predicts better.
    Hash,
    /// This ply's move in the previous iteration's completed best line (see
    /// [`Search::previous_pv_line`]). Ranked above `Hash` even though the two
    /// usually agree: a hash-table entry for this exact position can belong to
    /// a different, more recently searched line if replacement has since
    /// overwritten it, where this search's own recorded line cannot.
    PrincipalVariation,
}

impl MovePriority {
    /// How many tiers there are, which is the width of a per-tier count.
    const COUNT: usize = Self::ALL.len();

    /// This tier's position in the order [`Search::order_moves`] produces, best
    /// first. The inverse of `index`, which numbers by declaration and so runs
    /// worst first.
    const fn rank(self) -> usize {
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
struct PriorityRuns {
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
    fn from_counts(counts: &[u16; MovePriority::COUNT]) -> Self {
        let mut bounds = [0u16; MovePriority::COUNT + 1];
        let mut total = 0u16;
        for (slot, count) in bounds.iter_mut().skip(1).zip(counts) {
            total += *count;
            *slot = total;
        }
        Self { bounds }
    }

    /// The half-open index range `tier` occupies, empty if no move has it.
    fn range(self, tier: MovePriority) -> Range<usize> {
        let rank = tier.rank();
        usize::from(self.bounds[rank])..usize::from(self.bounds[rank + 1])
    }

    /// Whether the move at `index` is quiet: not a capture, not a promotion,
    /// and not lifted above `Quiet` by the hash move, the previous principal
    /// variation or a killer. This is the predicate late move reductions,
    /// futility pruning and late move pruning all ask, and the reason this type
    /// exists at all.
    fn is_quiet(self, index: usize) -> bool {
        self.range(MovePriority::Quiet).contains(&index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::cutoff_history::CutoffHistory;
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

    /// `move_priority` is private to this module, and so is `order_moves`;
    /// both are pure enough (no board mutation, no search recursion) to test
    /// directly here rather than only through `Search::search` end to end,
    /// matching this crate's convention of unit-testing a private pure
    /// function in-module and reserving `tests/search.rs`/`search_props.rs`
    /// for `Search`'s own public API.
    ///
    /// A position with exactly one capture on offer (`Qe4xd5`) alongside
    /// several quiet king moves, so the direction tests below have a real
    /// capture to out-rank a hint against, not just a pair of quiet moves
    /// that would already tie regardless of which one is hinted.
    fn capture_and_quiet_position() -> Board {
        Board::try_from_fen("4k3/8/8/3n4/4Q3/8/8/4K3 w - - 0 1").expect("valid FEN")
    }

    fn find_move(board: &Board, from: Square, to: Square) -> Move {
        *legal_moves(board)
            .as_slice()
            .iter()
            .find(|m| m.from() == from && m.to() == to)
            .unwrap_or_else(|| panic!("{from:?}{to:?} must be a legal move in this position"))
    }

    /// `find_move` alone is ambiguous for a promotion square: a pawn reaching the
    /// back rank has up to four legal moves sharing the same `from`/`to`, one per
    /// promotion piece, so the piece has to be part of the match.
    fn find_promotion_move(board: &Board, from: Square, to: Square, piece: Piece) -> Move {
        *legal_moves(board)
            .as_slice()
            .iter()
            .find(|m| {
                m.from() == from && m.to() == to && m.flags().promotion_piece() == Some(piece)
            })
            .unwrap_or_else(|| {
                panic!("{from:?}{to:?}={piece:?} must be a legal promotion in this position")
            })
    }

    /// A `Search` seeded with exactly the ply-0 ordering inputs a test wants to
    /// exercise, so each `move_priority`/`order_moves` test below can stay a
    /// short, direct call the way it was when these were free functions,
    /// rather than repeating this setup inline everywhere.
    fn priority_search(tt_move: Option<Move>, killers: &[Move]) -> Search<'static> {
        let mut search = Search::new(Vec::new());
        search.set_hash_move(0, tt_move);
        // Recorded oldest first, so the slice reads most-recent-first the way
        // the table orders its own slots.
        for &m in killers.iter().rev() {
            search.killers.record(0, m, false);
        }
        search
    }

    /// Every variant, in the exact order the intended hierarchy requires:
    /// `#[derive(Ord)]` on the enum compares by declaration order, so this list
    /// *is* the ranking, not a lookup table alongside it. A typo'd or reordered
    /// variant here wouldn't fail to compile, it would just silently misorder two
    /// tiers relative to each other. `WinningCapture`/`LosingCapture` carry a
    /// same-magnitude tiebreak value in this test since only cross-variant order
    /// is being checked; within-tier ordering has its own tests below.
    #[test]
    fn move_priority_rank_order_matches_the_intended_hierarchy() {
        use MovePriority::{
            EqualCapture, Hash, Killer, LosingCapture, MateKiller, PrincipalVariation, Quiet,
            WinningCapture,
        };
        let ranked_worst_to_best = [
            LosingCapture,
            Quiet,
            Killer,
            MateKiller,
            EqualCapture,
            WinningCapture,
            Hash,
            PrincipalVariation,
        ];
        for pair in ranked_worst_to_best.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} must rank strictly behind {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn move_priority_with_no_tt_hint_classifies_a_quiet_move_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0)
        );
    }

    // MVV-LVA's whole point: a small attacker taking a large victim (pawn
    // takes queen) is the most promising capture there is, tried before a
    // big attacker taking a small victim (queen takes pawn), which is
    // comparatively unpromising. `WinningCapture`/`LosingCapture` need to
    // land on the *correct side* of that distinction, not just land on two
    // different variants (a naming swap between the two would still produce
    // "two distinct variants" and could still pass a looser test). Pinning
    // the exact tiebreak value, not just the variant, also catches the
    // tiebreak's sign getting negated relative to the raw victim-minus-
    // attacker delta: it should carry that delta un-negated.
    #[test]
    fn move_priority_classifies_a_small_attacker_taking_a_big_victim_as_winning() {
        let board = Board::try_from_fen("4k3/8/8/8/3q4/4P3/8/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_queen = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, pawn_takes_queen, 0),
            (MovePriority::WinningCapture, 800),
            "a pawn capturing a queen is the textbook winning capture, gain = 900 - 100"
        );
    }

    #[test]
    fn move_priority_classifies_a_big_attacker_taking_a_small_victim_as_losing() {
        let board = Board::try_from_fen("4k3/8/8/8/3p4/4Q3/8/4K3 w - - 0 1").expect("valid FEN");
        let queen_takes_pawn = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, queen_takes_pawn, 0),
            (MovePriority::LosingCapture, -800),
            "a queen capturing an undefended pawn is comparatively unpromising, \
             the opposite end of the scale from a pawn capturing a queen, gain = 100 - 900"
        );
    }

    #[test]
    fn move_priority_classifies_an_equal_value_capture_as_equal() {
        let board = Board::try_from_fen("4k3/8/8/8/3r4/8/8/3RK3 w - - 0 1").expect("valid FEN");
        let rook_takes_rook = find_move(&board, Square::D1, Square::D4);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, rook_takes_rook, 0),
            (MovePriority::EqualCapture, 0)
        );
    }

    /// `RxQ` and `PxN` are both winning captures, so a priority that carried no
    /// payload would rank them equal and throw away MVV-LVA's whole point. This
    /// pins that the material delta on `WinningCapture` actually breaks the tie,
    /// rather than both moves merely landing in the same broad tier.
    #[test]
    fn move_priority_does_not_conflate_two_different_winning_captures() {
        let rxq_board = Board::try_from_fen("q5k1/8/8/8/8/8/8/R3K3 w - - 0 1").expect("valid FEN");
        let rook_takes_queen = find_move(&rxq_board, Square::A1, Square::A8);

        let pxn_board =
            Board::try_from_fen("6k1/8/8/8/8/2n5/1P6/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_knight = find_move(&pxn_board, Square::B2, Square::C3);

        let rxq_priority =
            priority_search(None, &[]).move_priority(&rxq_board, rook_takes_queen, 0);
        let pxn_priority =
            priority_search(None, &[]).move_priority(&pxn_board, pawn_takes_knight, 0);

        assert_ne!(
            rxq_priority, pxn_priority,
            "RxQ (gain 400) and PxN (gain 220) must not compare equal"
        );
        assert!(
            rxq_priority > pxn_priority,
            "RxQ must outrank PxN: rook-for-queen is a bigger material swing \
             than pawn-for-knight, even though both are winning captures"
        );
        assert_eq!(rxq_priority, (MovePriority::WinningCapture, 400));
        assert_eq!(pxn_priority, (MovePriority::WinningCapture, 220));
    }

    /// A losing capture must sort behind a quiet move, not ahead of it: hanging
    /// a piece is worse than an untried ordinary move. This is what
    /// `LosingCapture`'s position in the declaration order buys, and the
    /// derived `Ord` makes that position load-bearing rather than cosmetic.
    #[test]
    fn move_priority_ranks_a_losing_capture_behind_a_quiet_move() {
        let board = capture_and_quiet_position();
        let losing_capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        assert!(
            priority_search(None, &[]).move_priority(&board, quiet, 0)
                > priority_search(None, &[]).move_priority(&board, losing_capture, 0),
            "a losing capture must sort behind a quiet move, not ahead of it"
        );
    }

    /// The direction, stated as its own property rather than two isolated
    /// classifications: whichever variants "winning" and "losing" end up
    /// being, a small-attacker/big-victim capture must outrank a
    /// big-attacker/small-victim one on the same squares. This is the
    /// property the two classification tests above only check indirectly
    /// (through whatever `MovePriority::Ord` says those variants are worth);
    /// this one checks the actual consequence.
    #[test]
    fn small_attacker_takes_big_victim_ranks_above_big_attacker_takes_small_victim() {
        let winning_board =
            Board::try_from_fen("4k3/8/8/8/3q4/4P3/8/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_queen = find_move(&winning_board, Square::E3, Square::D4);

        let losing_board =
            Board::try_from_fen("4k3/8/8/8/3p4/4Q3/8/4K3 w - - 0 1").expect("valid FEN");
        let queen_takes_pawn = find_move(&losing_board, Square::E3, Square::D4);

        assert!(
            priority_search(None, &[]).move_priority(&winning_board, pawn_takes_queen, 0)
                > priority_search(None, &[]).move_priority(&losing_board, queen_takes_pawn, 0),
            "a pawn capturing a queen must be tried before a queen capturing a pawn"
        );
    }

    #[test]
    fn move_priority_with_matching_tt_hint_is_hash_regardless_of_move_shape() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        assert_eq!(
            priority_search(Some(capture), &[]).move_priority(&board, capture, 0),
            (MovePriority::Hash, 0)
        );
        assert_eq!(
            priority_search(Some(quiet), &[]).move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0)
        );
    }

    #[test]
    fn move_priority_with_non_matching_tt_hint_falls_back_to_normal_classification() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        // `quiet` is hinted, but `capture` is the move being scored: the
        // hint shouldn't affect a move it doesn't match.
        assert_eq!(
            priority_search(Some(quiet), &[]).move_priority(&board, capture, 0),
            priority_search(None, &[]).move_priority(&board, capture, 0),
            "a tt hint for a different move must not affect this move's own ordering"
        );
    }

    /// The direction check: a lazier stub that only satisfies the
    /// `move_priority`-level tests above (e.g. one that *deprioritizes* the
    /// hinted move instead of prioritizing it) could still pass every one of
    /// them if it never actually checked which way "outranks" needs to go.
    /// Ordering a real capture against a real, unrelated quiet hint is what
    /// would actually catch that: a backwards implementation sends the
    /// quiet hint to the back, not the front.
    #[test]
    fn order_moves_ranks_an_unrelated_tt_hint_above_a_good_capture() {
        let board = capture_and_quiet_position();
        let mut moves = legal_moves(&board);

        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet_hint = find_move(&board, Square::E1, Square::D1);

        priority_search(Some(quiet_hint), &[]).order_moves(&board, &mut moves, 0);

        assert_eq!(
            moves.as_slice()[0],
            quiet_hint,
            "the tt-hinted quiet move must sort first, ahead of the available capture"
        );
        assert_ne!(
            moves.as_slice()[0],
            capture,
            "the capture must not outrank an unrelated tt hint"
        );
    }

    /// Whichever legal move is handed to `order_moves` as the hint lands at
    /// index 0, whatever kind of move it is. Looping over every legal move in
    /// the position (the one capture and several quiet king moves) as the hint
    /// in turn checks that generically, rather than pinning it to one move's
    /// own kind and passing for the wrong reason.
    #[test]
    fn order_moves_places_any_hinted_move_first() {
        let board = capture_and_quiet_position();
        let legal = legal_moves(&board);

        for &hint in legal.as_slice() {
            let mut moves = legal_moves(&board);
            priority_search(Some(hint), &[]).order_moves(&board, &mut moves, 0);
            assert_eq!(
                moves.as_slice()[0],
                hint,
                "hint move {hint:?} must land at index 0 regardless of whether \
                 it's a capture or a quiet move"
            );
        }
    }

    /// A lone pawn one push from queening, nothing else on the board that
    /// could capture: the only loud move available is the promotion itself.
    fn promotion_no_capture_position() -> Board {
        Board::try_from_fen("7k/4P3/8/8/8/8/8/K7 w - - 0 1").expect("valid FEN")
    }

    /// Same pawn, same promotion square, but a rook sits on the capture
    /// diagonal so every promotion piece has a capturing and a non-capturing
    /// counterpart to compare against.
    fn promotion_with_capture_position() -> Board {
        Board::try_from_fen("5r1k/4P3/8/8/8/8/8/K7 w - - 0 1").expect("valid FEN")
    }

    #[test]
    fn move_priority_classifies_a_non_capture_promotion_as_a_winning_capture() {
        let board = promotion_no_capture_position();
        let queen_promo = find_promotion_move(&board, Square::E7, Square::E8, Piece::Queen);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, queen_promo, 0),
            (MovePriority::WinningCapture, 800),
            "a non-capture queen promotion must outrank quiet moves, not tie with \
             them: gain = 900 - 100, the same tier a good capture lands in"
        );
    }

    /// The reason to interleave promotions with captures on one material
    /// scale, rather than giving promotions their own fixed tier: a
    /// promotion that also captures a piece is worth strictly more than the
    /// same promotion alone, by exactly the captured piece's value.
    #[test]
    fn move_priority_ranks_a_capturing_promotion_above_a_plain_promotion_of_the_same_piece() {
        let plain_board = promotion_no_capture_position();
        let plain_promo = find_promotion_move(&plain_board, Square::E7, Square::E8, Piece::Queen);

        let capture_board = promotion_with_capture_position();
        let capturing_promo =
            find_promotion_move(&capture_board, Square::E7, Square::F8, Piece::Queen);

        let plain_priority = priority_search(None, &[]).move_priority(&plain_board, plain_promo, 0);
        let capturing_priority =
            priority_search(None, &[]).move_priority(&capture_board, capturing_promo, 0);

        assert!(
            capturing_priority > plain_priority,
            "capturing a rook while promoting must outrank promoting alone"
        );
        assert_eq!(plain_priority, (MovePriority::WinningCapture, 800));
        assert_eq!(
            capturing_priority,
            (MovePriority::WinningCapture, 1200),
            "gain = (900 - 100) promotion + (500 - 100) capture"
        );
    }

    /// `PIECE_VALUES` gives every promotion piece strictly more value than a
    /// pawn (the smallest promotion gain, to a knight, is still 220), so
    /// `move_priority`'s combined gain can never be zero or negative for a
    /// promotion. That's what lets the branch order check material gain
    /// before any killer-table lookup: a promotion can never fall through to
    /// `Quiet`, `Killer`, or `MateKiller`, with or without a capture attached,
    /// and this is a fact about the value table, not about any one FEN.
    #[test]
    fn every_promotion_piece_classifies_as_winning_never_quiet() {
        let plain_board = promotion_no_capture_position();
        let capture_board = promotion_with_capture_position();

        for piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
            let plain = find_promotion_move(&plain_board, Square::E7, Square::E8, piece);
            assert!(
                matches!(
                    priority_search(None, &[]).move_priority(&plain_board, plain, 0),
                    (MovePriority::WinningCapture, _)
                ),
                "{piece:?} promotion alone must classify as WinningCapture, never Quiet"
            );

            let capturing = find_promotion_move(&capture_board, Square::E7, Square::F8, piece);
            assert!(
                matches!(
                    priority_search(None, &[]).move_priority(&capture_board, capturing, 0),
                    (MovePriority::WinningCapture, _)
                ),
                "{piece:?} promotion with a capture must classify as WinningCapture, never Quiet"
            );
        }
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
        // A window so narrow that whichever evasion `order_moves` tries
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
            "order_moves is deterministic given identical fresh state in both runs, so the \
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

    // ---- Killer-move classification ----
    //
    // `Search::move_priority`'s own `killer_slots(ply)` lookup carries the two
    // slots for the ply the move is being classified at. A killer only outranks
    // a quiet move, never a capture, so these check the boundary in both
    // directions rather than only that a match is recognised.

    #[test]
    fn move_priority_with_matching_first_killer_slot_classifies_as_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_search(None, &[quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Killer, 0)
        );
    }

    #[test]
    fn move_priority_with_matching_second_killer_slot_classifies_as_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let other_quiet = find_move(&board, Square::E1, Square::F1);
        assert_eq!(
            priority_search(None, &[other_quiet, quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Killer, 0),
            "both slots must be checked, not just the first"
        );
    }

    #[test]
    fn move_priority_with_no_matching_killer_slot_classifies_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let unrelated = find_move(&board, Square::E1, Square::F1);
        assert_eq!(
            priority_search(None, &[unrelated]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0),
            "a killer slot holding a different move must not affect this move's own classification"
        );
    }

    #[test]
    fn move_priority_with_empty_killer_slots_classifies_a_quiet_move_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_search(None, &[]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0)
        );
    }

    /// A killer slot is looked up at the ply `move_priority` is asked about, so
    /// a move recorded as a killer at one ply must not leak into a
    /// classification call for a different ply's slots. This is now a real
    /// property of `ply` itself, not (as when `killers` was a bare parameter)
    /// just a labeling convention on whatever array the caller happened to pass.
    #[test]
    fn move_priority_trusts_whichever_ply_it_is_asked_about() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        search.killers.record(5, quiet, false);
        assert_eq!(
            search.move_priority(&board, quiet, 5),
            (MovePriority::Killer, 0)
        );
        assert_eq!(
            search.move_priority(&board, quiet, 9),
            (MovePriority::Quiet, 0),
            "ply 9's own (empty) killer slots must be consulted, not ply 5's"
        );
    }

    #[test]
    fn move_priority_prefers_hash_over_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_search(Some(quiet), &[quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0),
            "the tt hint must outrank a killer slot for the same move"
        );
    }

    // ---- Mate-killer classification ----
    //
    // Same shape as the ordinary killer tests above, seeding the killer table
    // rather than driving a real search to populate it: `move_priority`'s own
    // lookup is what is under test, not `note_cutoff_move`'s population logic,
    // which has its own section below.

    #[test]
    fn move_priority_with_matching_mate_killer_classifies_as_mate_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        search.killers.record(0, quiet, true);
        assert_eq!(
            search.move_priority(&board, quiet, 0),
            (MovePriority::MateKiller, 0)
        );
    }

    #[test]
    fn move_priority_with_no_matching_mate_killer_falls_back_to_ordinary_classification() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let unrelated = find_move(&board, Square::E1, Square::F1);
        let mut search = Search::new(Vec::new());
        search.killers.record(0, unrelated, true);
        assert_eq!(
            search.move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0),
            "a mate-killer slot holding a different move must not affect this move's own \
             classification"
        );
    }

    /// The whole reason `MateKiller` is a separate rank from `Killer`: a move
    /// sitting in both tables at once (recording one is never conditional on
    /// the other; see `note_cutoff_move`) must classify by the more specific,
    /// higher-value fact about it.
    #[test]
    fn move_priority_prefers_mate_killer_over_ordinary_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = priority_search(None, &[quiet]);
        search.killers.record(0, quiet, true);
        assert_eq!(
            search.move_priority(&board, quiet, 0),
            (MovePriority::MateKiller, 0),
            "a move sitting in both tables must classify as the mate killer, not the ordinary one"
        );
    }

    #[test]
    fn move_priority_prefers_hash_over_mate_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = priority_search(Some(quiet), &[]);
        search.killers.record(0, quiet, true);
        assert_eq!(
            search.move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0),
            "the tt hint must outrank a mate-killer slot for the same move"
        );
    }

    /// Ply-scoping, the same property `move_priority_trusts_whichever_ply_it_is_asked_about`
    /// pins for the ordinary killer table: a mate killer recorded at one ply
    /// must not leak into a classification call for a different ply.
    #[test]
    fn move_priority_trusts_whichever_ply_its_mate_killer_is_asked_about() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        search.killers.record(5, quiet, true);
        assert_eq!(
            search.move_priority(&board, quiet, 5),
            (MovePriority::MateKiller, 0)
        );
        assert_eq!(
            search.move_priority(&board, quiet, 9),
            (MovePriority::Quiet, 0),
            "ply 9's own (empty) mate-killer slot must be consulted, not ply 5's"
        );
    }

    // ---- Mate-killer recording (`note_cutoff_move`) ----
    //
    // `move_priority`'s own lookup is covered above; these instead drive
    // `note_cutoff_move` itself, the write side, directly. `note_cutoff_move`
    // is private and pure enough (no recursion, no board mutation beyond
    // `self`'s own tables) to test the same way `move_priority`/`order_moves`
    // are, per this module's convention.

    #[test]
    fn note_cutoff_move_records_a_mate_killer_on_a_quiet_move_with_a_mate_score() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        let cause = search.note_cutoff_move(0, quiet, MATE - 1);
        assert!(
            search.killers.holds_mate(0, quiet),
            "a quiet move causing a cutoff with a mate score must populate this ply's mate killer"
        );
        assert_eq!(
            cause,
            CutoffCause::Other,
            "the first time a move is recorded, nothing had predicted it yet: crediting the \
             mate-killer technique here would overstate what it did, the same reasoning \
             `note_cutoff_move`'s own doc gives for the ordinary killer table"
        );
    }

    #[test]
    fn note_cutoff_move_credits_mate_killer_once_the_same_move_repeats_at_the_same_ply() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        search.note_cutoff_move(0, quiet, MATE - 1);
        let cause = search.note_cutoff_move(0, quiet, MATE - 1);
        assert_eq!(
            cause,
            CutoffCause::MateKiller,
            "a move already sitting in this ply's mate-killer slot must be credited to it \
             on its next cutoff"
        );
    }

    #[test]
    fn note_cutoff_move_does_not_record_a_mate_killer_for_an_ordinary_score() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = Search::new(Vec::new());
        search.note_cutoff_move(0, quiet, 100);
        assert!(
            !search.killers.holds_mate(0, quiet),
            "an ordinary (non-mate) cutoff score must never populate the mate-killer slot"
        );
    }

    #[test]
    fn note_cutoff_move_never_records_a_mate_killer_for_a_capture() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let mut search = Search::new(Vec::new());
        search.note_cutoff_move(0, capture, MATE - 1);
        assert!(
            !search.killers.holds_mate(0, capture),
            "captures are already ordered by MVV-LVA; recording one as a mate killer would \
             waste the slot the same way recording it as an ordinary killer would"
        );
    }

    #[test]
    fn note_cutoff_move_mate_killer_slot_always_replaces() {
        let board = capture_and_quiet_position();
        let first = find_move(&board, Square::E1, Square::D1);
        let second = find_move(&board, Square::E1, Square::F1);
        let mut search = Search::new(Vec::new());
        search.note_cutoff_move(0, first, MATE - 1);
        search.note_cutoff_move(0, second, MATE - 1);
        assert!(
            search.killers.holds_mate(0, second) && !search.killers.holds_mate(0, first),
            "one slot, always-replace: the most recent mate-scoring cutoff move wins and the \
             previous one is gone, with no promote/shift policy the way the two-slot ordinary \
             killer table has"
        );
    }

    // ---- Previous-iteration PV classification ----
    //
    // `PrincipalVariation` outranks `Hash` (see `MovePriority`'s own doc for
    // why): a hash-table entry can belong to a different, more recently
    // searched line once replacement has overwritten it, where
    // `Search::previous_pv_line` is this search's own uncorrupted record.
    // `PVS`'s own recursion is what will ever populate that line for a real
    // search; these tests poke it directly, the same way the killer tests
    // above poke `killers` directly rather than running a real search to get
    // a slot filled.

    #[test]
    fn move_priority_prefers_previous_pv_move_over_hash_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut search = priority_search(Some(quiet), &[]);
        search.previous_pv_line[0] = Some(quiet);
        assert_eq!(
            search.move_priority(&board, quiet, 0),
            (MovePriority::PrincipalVariation, 0),
            "the previous iteration's own pv move must outrank a same-move hash hint"
        );
    }

    /// The direction check, same shape as the tt-hint/killer versions above:
    /// ordering a real capture against a real, unrelated previous-pv hint is
    /// what would catch a backwards or no-op implementation, not just the
    /// `move_priority`-level classification test above.
    #[test]
    fn order_moves_places_the_previous_pv_move_first() {
        let board = capture_and_quiet_position();
        let mut moves = legal_moves(&board);

        let capture = find_move(&board, Square::E4, Square::D5);
        let pv_hint = find_move(&board, Square::E1, Square::D1);

        let mut search = priority_search(None, &[]);
        search.previous_pv_line[0] = Some(pv_hint);
        search.order_moves(&board, &mut moves, 0);

        assert_eq!(
            moves.as_slice()[0],
            pv_hint,
            "the previous iteration's pv move must sort first, ahead of the available capture"
        );
        assert_ne!(
            moves.as_slice()[0],
            capture,
            "the capture must not outrank an unrelated pv hint"
        );
    }

    /// Captures return from `move_priority` before the killer-slot lookup is
    /// ever reached, so a killer slot happening to hold the same `Move` value
    /// as a capture (impossible in practice: a capture and a quiet move to
    /// the same square carry different `MoveFlags`, so they're different
    /// `Move` values, but this pins the guarantee down directly rather than
    /// relying on that being true by construction elsewhere) still can't
    /// promote it out of the capture tiers.
    #[test]
    fn move_priority_never_classifies_a_capture_as_killer() {
        let board = capture_and_quiet_position();
        // Queen takes knight: a losing trade by this heuristic (attacker
        // outvalues victim), which is exactly why it's the right move to use
        // here. If a capture's own killer-slot lookup were ever reachable,
        // this is the tier that would most plausibly get displaced by it:
        // `LosingCapture` sits right next to `Killer` in the ranking (see
        // `move_priority_rank_order_matches_the_intended_hierarchy`), so a
        // bug that let a losing capture fall through to a killer check would
        // be invisible on a winning capture, which is nowhere near that
        // boundary.
        let capture = find_move(&board, Square::E4, Square::D5);
        assert!(matches!(
            priority_search(None, &[capture]).move_priority(&board, capture, 0),
            (MovePriority::LosingCapture, _)
        ));
    }

    /// A pawn one capture from a free knight, alongside several quiet king
    /// moves: unlike `capture_and_quiet_position`'s queen-takes-knight
    /// (a losing trade), `Pxd5` here is a genuine winning capture, which is
    /// what the killer-ordering test below needs to outrank a killer with.
    fn winning_capture_and_quiet_position() -> Board {
        Board::try_from_fen("4k3/8/8/3n4/4P3/8/8/4K3 w - - 0 1").expect("valid FEN")
    }

    /// The direction check, same shape as the tt-hint version above: a
    /// stub that only satisfies the `move_priority`-level tests could still
    /// pass them without ever actually reordering a real move list. Ordering
    /// a real capture against a real killer is what would catch a backwards
    /// implementation (one that sends the killer to the back instead of
    /// ahead of unrelated quiets).
    #[test]
    fn order_moves_places_a_killer_ahead_of_an_unrelated_quiet_but_behind_a_capture() {
        let board = winning_capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let killer = find_move(&board, Square::E1, Square::D1);
        let other_quiet = find_move(&board, Square::E1, Square::F1);

        let mut moves = legal_moves(&board);
        priority_search(None, &[killer]).order_moves(&board, &mut moves, 0);

        let killer_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == killer)
            .expect("killer is a legal move in this position");
        let capture_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == capture)
            .expect("capture is a legal move in this position");
        let other_quiet_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == other_quiet)
            .expect("other_quiet is a legal move in this position");

        assert!(
            capture_index < killer_index,
            "the capture must still outrank the killer"
        );
        assert!(
            killer_index < other_quiet_index,
            "the killer must outrank an unrelated quiet move"
        );
    }

    /// Classifying a move by its index, which is how the per-move policy seam's
    /// callers ask whether a move is quiet, is sound only if each
    /// [`MovePriority`] occupies one unbroken run of the ordered list. That
    /// holds because [`Search::move_priority`]'s tuple sorts on the tier first,
    /// so it is a property of the sort rather than of any position.
    ///
    /// Worth its own test because every way of breaking it is silent: a
    /// secondary sort criterion that crossed tiers, a reordered `MovePriority`
    /// variant, or a new tier whose membership is not a function of the tuple's
    /// first element would all leave the existing ordering tests green while
    /// making an index-derived classification lie.
    #[test]
    fn ordering_leaves_every_priority_tier_in_one_contiguous_run() {
        // Kiwipete, which offers captures across the whole value range at once,
        // alongside the start position and a lone-king endgame where almost
        // everything is quiet.
        let fens = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "4k3/8/8/3n4/4Q3/8/8/4K3 w - - 0 1",
        ];

        let mut richest = 0;
        for fen in fens {
            let board = Board::try_from_fen(fen).expect("valid FEN");
            let mut moves = legal_moves(&board);
            let quiets: Vec<Move> = moves
                .iter()
                .filter(|m| !m.flags().is_capture() && !m.flags().is_promotion())
                .copied()
                .collect();
            assert!(
                quiets.len() >= 3,
                "{fen}: need three quiets to seed a hash hint and two killers"
            );

            let mut search = Search::new(Vec::new());
            search.set_hash_move(0, Some(quiets[0]));
            search.killers.record(0, quiets[1], false);
            search.killers.record(0, quiets[2], true);
            search.order_moves(&board, &mut moves, 0);

            let mut runs: Vec<MovePriority> = Vec::new();
            for &m in &moves {
                let tier = search.move_priority(&board, m, 0).0;
                if runs.last() != Some(&tier) {
                    runs.push(tier);
                }
            }

            let mut distinct = runs.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(
                runs.len(),
                distinct.len(),
                "{fen}: a tier appears in more than one run: {runs:?}"
            );
            richest = richest.max(distinct.len());
        }

        assert!(
            richest >= 4,
            "no position produced enough tiers for the property to mean anything: {richest}"
        );
    }

    // ---- Move ordering's priority runs ----

    /// Positions paired with the exact order `order_moves` puts them in, all
    /// four seeded identically by [`seeded_search`].
    ///
    /// The orders were captured from the implementation rather than derived by
    /// hand, so on their own they assert only that ordering has not changed.
    /// That is the point: a rewrite of how the sort carries its keys has to
    /// come out move-identical, and this catches a difference in the unit test
    /// suite rather than only in `tools/refactor-gate.sh`.
    ///
    /// Every position is legal, checked with `in_check` against the side not to
    /// move. `capture_and_quiet_position` is deliberately not among them: the
    /// side not to move is in check there, which makes it an illegal position
    /// whose move list contains a king capture.
    const ORDERING_FIXTURES: &[(&str, &str)] = &[
        (
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "a2a3 g2h3 d5e6 e2a6 b2b3 a2a4 f3f4 c3b1 c3d1 c3a4 c3b5 e5d3 e5c4 \
             e5g4 e5c6 e1d1 e1f1 d2c1 d2e3 d2f4 d2g5 d2h6 d5d6 g2g4 e2f1 e2d3 \
             e2c4 e2b5 a1b1 a1c1 a1d1 h1f1 h1g1 f3d3 f3e3 f3g3 g2g3 f3g4 f3f5 \
             f3h5 e1g1 e1c1 e2d1 e5d7 e5f7 e5g6 f3f6 f3h3",
        ),
        (
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "a2a3 b2b3 a2a4 b2b4 c2c3 c2c4 d2d3 d2d4 e2e3 e2e4 f2f3 f2f4 g2g3 \
             g2g4 h2h3 h2h4 b1a3 b1c3 g1f3 g1h3",
        ),
        (
            "3k4/8/8/3n4/4Q3/8/8/4K3 w - - 0 1",
            "e1d1 e1d2 e1f1 e1e2 e1f2 e4d3 e4b1 e4h1 e4c2 e4e2 e4g2 e4e3 e4f3 \
             e4a4 e4b4 e4c4 e4d4 e4f4 e4g4 e4h4 e4e5 e4f5 e4e6 e4g6 e4e7 e4h7 \
             e4e8 e4d5",
        ),
        (
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "e2e3 g2g3 e2e4 g2g4 a5a4 a5a6 b4b1 b4b2 b4b3 b4a4 b4c4 b4d4 b4e4 \
             b4f4",
        ),
    ];

    /// A `Search` whose ply-0 hints are seeded from this position's own quiet
    /// moves, so a fixture's expected order is reproducible from the FEN alone
    /// rather than from a table of hand-picked hint moves.
    fn seeded_search(board: &Board) -> Search<'static> {
        let quiets: Vec<Move> = legal_moves(board)
            .iter()
            .filter(|m| !m.flags().is_capture() && !m.flags().is_promotion())
            .copied()
            .collect();
        assert!(
            quiets.len() >= 3,
            "a fixture needs three quiet moves to seed a hash hint, a killer and a mate killer"
        );
        let mut search = Search::new(Vec::new());
        search.set_hash_move(0, Some(quiets[0]));
        search.killers.record(0, quiets[1], false);
        search.killers.record(0, quiets[2], true);
        search
    }

    /// The fixture's position, its ordered moves, the runs over them, and the
    /// search that produced all three.
    fn order_fixture(fen: &str) -> (Board, MoveList, PriorityRuns, Search<'static>) {
        let board = Board::try_from_fen(fen).expect("valid FEN");
        let mut search = seeded_search(&board);
        let mut moves = legal_moves(&board);
        let runs = search.order_moves(&board, &mut moves, 0);
        (board, moves, runs, search)
    }

    fn rendered(moves: &MoveList) -> String {
        moves
            .iter()
            .map(|m| format!("{:?}{:?}", m.from(), m.to()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn every_ordering_fixture_is_a_legal_position() {
        for (fen, _) in ORDERING_FIXTURES {
            let board = Board::try_from_fen(fen).expect("valid FEN");
            assert!(
                !in_check(&board, board.side_to_move().flip()),
                "{fen}: the side not to move is in check, so this position cannot arise"
            );
        }
    }

    #[test]
    fn order_moves_produces_the_order_it_always_has() {
        for (fen, expected) in ORDERING_FIXTURES {
            let (_, moves, _, _) = order_fixture(fen);
            let expected: Vec<&str> = expected.split_whitespace().collect();
            assert_eq!(
                rendered(&moves),
                expected.join(" "),
                "{fen}: the ordering changed"
            );
        }
    }

    #[test]
    fn the_runs_place_every_move_in_its_own_tier() {
        for (fen, _) in ORDERING_FIXTURES {
            let (board, moves, runs, search) = order_fixture(fen);
            for (i, &m) in moves.iter().enumerate() {
                let tier = search.move_priority(&board, m, 0).0;
                assert!(
                    runs.range(tier).contains(&i),
                    "{fen}: move {i} is {tier:?}, but the runs put that tier at {:?}",
                    runs.range(tier)
                );
            }
            let covered: usize = MovePriority::ALL
                .into_iter()
                .map(|t| runs.range(t).len())
                .sum();
            assert_eq!(
                covered,
                moves.len(),
                "{fen}: the runs must cover every move exactly once"
            );
        }
    }

    #[test]
    fn is_quiet_agrees_with_the_tier_move_priority_assigns() {
        for (fen, _) in ORDERING_FIXTURES {
            let (board, moves, runs, search) = order_fixture(fen);
            for (i, &m) in moves.iter().enumerate() {
                let tier = search.move_priority(&board, m, 0).0;
                assert_eq!(
                    runs.is_quiet(i),
                    tier == MovePriority::Quiet,
                    "{fen}: move {i} is {tier:?}"
                );
            }
        }
    }

    /// An absent tier has to answer as an empty range rather than as a missing
    /// one, because a caller indexes it unconditionally. The start position has
    /// no captures at all, so three tiers are absent at once.
    #[test]
    fn a_tier_no_move_belongs_to_is_an_empty_range() {
        let (_, _, runs, _) = order_fixture(ORDERING_FIXTURES[1].0);
        for tier in [
            MovePriority::WinningCapture,
            MovePriority::EqualCapture,
            MovePriority::LosingCapture,
        ] {
            assert!(
                runs.range(tier).is_empty(),
                "the start position offers no captures, so {tier:?} must be empty, got {:?}",
                runs.range(tier)
            );
        }
        assert!(
            !runs.range(MovePriority::Quiet).is_empty(),
            "the start position is almost all quiet moves"
        );
    }

    #[test]
    fn an_empty_move_list_leaves_every_run_empty() {
        let board = Board::try_from_fen(ORDERING_FIXTURES[1].0).expect("valid FEN");
        let mut search = Search::new(Vec::new());
        let mut moves = MoveList::new();
        let runs = search.order_moves(&board, &mut moves, 0);
        for tier in MovePriority::ALL {
            assert!(
                runs.range(tier).is_empty(),
                "no moves, so {tier:?} must be empty"
            );
        }
        assert!(
            !runs.is_quiet(0),
            "no move is quiet when there is no move at all"
        );
    }

    /// Ordering is about to grow a scratch buffer that outlives a single call.
    /// Anything left in it by one position must not reach the next, and the
    /// failure mode is a wrong classification rather than a crash, so it needs
    /// pinning before the buffer exists rather than after.
    #[test]
    fn ordering_one_position_does_not_leak_into_the_next() {
        let board = Board::try_from_fen(ORDERING_FIXTURES[3].0).expect("valid FEN");
        let mut alone = seeded_search(&board);
        let mut expected_moves = legal_moves(&board);
        let expected_runs = alone.order_moves(&board, &mut expected_moves, 0);

        let other = Board::try_from_fen(ORDERING_FIXTURES[0].0).expect("valid FEN");
        let mut reused = seeded_search(&board);
        let mut other_moves = legal_moves(&other);
        let _ = reused.order_moves(&other, &mut other_moves, 0);
        let mut moves = legal_moves(&board);
        let runs = reused.order_moves(&board, &mut moves, 0);

        assert_eq!(
            rendered(&moves),
            rendered(&expected_moves),
            "ordering a larger position first changed the order of this one"
        );
        assert_eq!(
            runs, expected_runs,
            "ordering a larger position first changed this one's runs"
        );
    }
}
