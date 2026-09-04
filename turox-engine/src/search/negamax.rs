//! `Search`: negamax with fail-soft alpha-beta over iterative deepening,
//! quiescence search at the horizon, and MVV-LVA capture ordering.
//!
//! Mirrors `move_gen`'s naive-reference discipline: there's no perft
//! equivalent for search, so `tests/search_props.rs` checks this against an
//! independent, unpruned negamax reference rather than trusting a
//! read-through.

use crate::board::Board;
use crate::eval::{evaluate, Score, PIECE_VALUES};
use crate::move_gen::attacks::in_check;
use crate::move_gen::legal::legal_moves;
use crate::move_gen::move_list::MoveList;
use crate::rng::xorshift64star;
use crate::search::draw::is_draw;
use crate::search::tt::Tt;
use crate::types::Move;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// How many plies past `negamax`'s horizon `quiescence` is allowed to keep resolving
/// captures before it gives up and falls back to stand pat, same as if no more captures
/// were available.
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

/// The safety-margin multiplier `search`'s soft time limit applies to the
/// previous iteration's own elapsed time, as its estimate of the *next*
/// iteration's cost: don't start iteration N+1 unless at least
/// `elapsed(N) * ITERATION_TIME_SAFETY_MARGIN` still remains before
/// `self.deadline`.
///
/// Real branching factors near the horizon run roughly 7-9x between
/// iterations; `4` is deliberately below that rather than matching it,
/// because the two ways this can be wrong aren't equally costly. Too high
/// (skip too eagerly) throws away reachable search depth, a direct
/// strength cost. Too low (skip too rarely) just falls back to today's
/// waste-a-doomed-iteration behavior, a time cost but not a strength one.
/// So this stays biased toward under-triggering: it only vetoes iterations
/// that would need less than half the typical growth factor to fit.
const ITERATION_TIME_SAFETY_MARGIN: u32 = 4;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchResult {
    /// `None` only when the position handed to `search` has no legal moves at all
    /// (checkmate or stalemate); every other case, including an aborted first
    /// iteration, still has a real move to report.
    pub best_move: Option<Move>,
    /// Side-to-move-relative, same convention as [`evaluate`].
    pub score: Score,
    /// The depth actually completed; see the struct doc for when this is
    /// less than the requested `max_depth`.
    pub depth: u8,
    /// Total nodes visited (negamax and quiescence both count) across every
    /// completed and aborted iteration of this call.
    pub nodes: u64,
}

/// What [`Search::search_root`] found for one depth: either the full move loop finished,
/// or an abort cut it short partway through.
enum RootOutcome {
    /// The move loop finished every move at this depth. `best_move` is `None` only for
    /// the genuine terminal case: no legal moves at all.
    Completed(Score, Option<Move>),
    /// The abort hit before every move at this depth could be tried. `best_so_far` is
    /// the best move and score among moves whose subtree had already fully resolved
    /// before the interruption, if any had; `None` when the abort landed before the
    /// move loop could report anything (the top-of-function check, or the depth-0
    /// quiescence-only path, which has no per-move loop to have made progress in).
    Aborted { best_so_far: Option<(Score, Move)> },
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
    nodes: u64,
    /// Hashes of every position on the path leading up to (but not
    /// including) the position currently being searched, per
    /// [`draw::is_threefold_repetition`]'s contract. Seeded by the caller
    /// with real game history, so repetitions that already happened in the
    /// actual game are visible, not just ones the search tree itself
    /// revisits. Grows by one push per ply descended, shrinks by one pop on
    /// backtrack: it must hold a node's own hash while searching that
    /// node's children, but never the node's own hash while checking the
    /// node itself.
    history: Vec<u64>,
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
    /// `None` by default (`negamax` searches with no transposition table at all, the same
    /// as before one existed); set via [`Search::with_tt`]. Borrowed, not owned: the table
    /// lives in `uci::session::run` (like `history` conceptually does, though `history` is
    /// actually copied in), so it survives across the separate `Search` a later `go` call
    /// rebuilds, rather than starting cold every time.
    tt: Option<&'a mut Tt>,
    /// xorshift64* state when root move randomization is on, `None` when it is
    /// off (the default, so every existing test and bench stays deterministic
    /// without knowing this exists).
    ///
    /// Only the *root* move list is shuffled. Interior nodes stay deterministic
    /// because shuffling them would fight move ordering, which is the single
    /// biggest lever on search efficiency this engine has.
    root_rng: Option<u64>,
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
            tt: None,
            root_rng: None,
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
    #[allow(
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
    /// starting each iteration past the first, `ITERATION_TIME_SAFETY_MARGIN`
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
        let mut result = SearchResult {
            best_move: None,
            score: 0,
            depth: 0,
            nodes: 0,
        };
        // Only tracked when `self.deadline` is set: a `max_nodes`-bounded or
        // fully unbounded search has nothing to estimate against, so this
        // stays `None` the whole loop and the soft-limit check below never
        // fires, leaving that path byte-for-byte unaffected by this check.
        let mut previous_iteration_elapsed: Option<Duration> = None;

        for depth in 1..=max_depth {
            if let (Some(deadline), Some(elapsed)) = (self.deadline, previous_iteration_elapsed) {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if elapsed.saturating_mul(ITERATION_TIME_SAFETY_MARGIN) > remaining {
                    break;
                }
            }

            let iteration_started = self.deadline.is_some().then(Instant::now);
            match self.search_root(board, depth) {
                RootOutcome::Completed(score, best_move) => {
                    result = SearchResult {
                        best_move,
                        score,
                        depth,
                        nodes: self.nodes(),
                    };
                    on_iteration_complete(&result);
                }
                RootOutcome::Aborted { best_so_far } => {
                    // Only depth 1 aborting can reach here with `result.best_move`
                    // still `None`: every later depth has a completed iteration
                    // already sitting in `result` to fall back on instead, per
                    // this method's own doc.
                    if result.best_move.is_none() {
                        if let Some((score, best_move)) = best_so_far {
                            result = SearchResult {
                                best_move: Some(best_move),
                                score,
                                depth: 0,
                                nodes: self.nodes(),
                            };
                        }
                    }
                    break;
                }
            }
            if let Some(started) = iteration_started {
                previous_iteration_elapsed = Some(started.elapsed());
            }
        }
        result
    }

    /// One ply of root move loop: like `negamax`, but remembers *which*
    /// move produced the best score, not just the score, so it's a
    /// separate small loop rather than a `ply == 0` special case buried
    /// inside `negamax`.
    ///
    /// Diverges from `negamax` in exactly one place: when the position is
    /// already a draw, an interior node can just return `0` and stop (see
    /// `negamax`'s own doc), but the root still needs a real move to hand
    /// back to UCI so play can continue, so it generates and orders the
    /// move list here instead. `Completed(0, None)` stays reserved for the
    /// true terminal case below: no legal moves at all.
    ///
    /// Returns [`RootOutcome::Aborted`] if the search was interrupted
    /// before every move at this depth could be tried. Moves already tried
    /// only ever update `max`/`best_move` after their subtree search
    /// returns a real score, never from a partially-searched one, so
    /// whatever `max`/`best_move` hold at the moment of abort are still
    /// trustworthy minimax values; only the one move that was mid-flight
    /// when the abort hit is genuinely unknown, hence `best_so_far` reports
    /// the finished moves' result rather than discarding it wholesale.
    fn search_root(&mut self, board: &Board, depth: u8) -> RootOutcome {
        self.nodes += 1;
        if self.should_abort() {
            return RootOutcome::Aborted { best_so_far: None };
        }

        if is_draw(board, &self.history, board.hash()) {
            let mut drawn_moves = legal_moves(board);
            if drawn_moves.is_empty() {
                return RootOutcome::Completed(0, None);
            }
            order_moves(board, &mut drawn_moves, None);
            return RootOutcome::Completed(0, Some(drawn_moves.as_slice()[0]));
        }

        let mut moves = legal_moves(board);
        if moves.is_empty() {
            return if in_check(board, board.side_to_move()) {
                RootOutcome::Completed(-MATE, None)
            } else {
                RootOutcome::Completed(0, None)
            };
        }

        let mut alpha = -MATE;
        let beta = MATE;

        // probably not necessary, but is technically possible by definition of depth being a u32 (it could be 0 even on this root step)
        if depth == 0 {
            return self
                .quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH, Some(moves))
                .map_or(RootOutcome::Aborted { best_so_far: None }, |score| {
                    RootOutcome::Completed(score, None)
                });
        }

        let mut max = Score::MIN;
        let mut best_move = None;
        // Before ordering, not after: `order_moves` sorts by a coarse priority
        // class, so shuffling first is what decides which of several moves
        // sharing a class gets tried first, while still leaving the ordering
        // itself intact.
        self.shuffle_root_moves(&mut moves);
        order_moves(board, &mut moves, None);
        for &m in &moves {
            self.history.push(board.hash());

            let child = board.make_move(m);
            let score = self.negamax(&child, depth - 1, 1, -beta, -alpha);

            self.history.pop();

            let Some(score) = score else {
                let best_so_far = best_move.map(|m| (max, m));
                return RootOutcome::Aborted { best_so_far };
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
                break;
            }
        }
        RootOutcome::Completed(max, best_move)
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
    fn negamax(
        &mut self,
        board: &Board,
        depth: u8,
        ply: u8,
        mut alpha: Score,
        beta: Score,
    ) -> Option<Score> {
        self.nodes += 1;
        if self.should_abort() {
            return None;
        }

        if is_draw(board, &self.history, board.hash()) {
            return Some(0);
        }

        let mut moves = legal_moves(board);
        if moves.is_empty() {
            return if in_check(board, board.side_to_move()) {
                Some(Score::from(ply) - MATE)
            } else {
                Some(0)
            };
        }

        let hash = board.hash();
        let tt_entry = self.tt.as_deref().and_then(|tt| tt.probe(hash));
        if let Some(entry) = tt_entry {
            if let Some(score) = entry.cutoff_score(depth, alpha, beta, ply) {
                return Some(score);
            }
        }

        if depth == 0 {
            return self.quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH, Some(moves));
        }

        let tt_move = tt_entry
            .map(|entry| Move::from_bits(entry.mv))
            .filter(|m| moves.as_slice().contains(m));

        let original_alpha = alpha;
        let mut max = Score::MIN;
        let mut best_move = None;
        order_moves(board, &mut moves, tt_move);
        for &m in &moves {
            self.history.push(board.hash());

            let child = board.make_move(m);
            let score = self.negamax(&child, depth - 1, ply + 1, -beta, -alpha);

            self.history.pop();

            let score = -score?;
            if score > max {
                max = score;
                best_move = Some(m);
            }

            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                break;
            }
        }

        if let Some(tt) = self.tt.as_deref_mut() {
            let best_move =
                best_move.expect("moves is non-empty, so the loop always finds a best move");
            tt.store(hash, ply, depth, max, original_alpha, beta, best_move);
        }

        Some(max)
    }

    /// Quiescence search: like `negamax`, but only ever considers captures,
    /// with "stand pat" (`evaluate(board)`) as the floor score, so a
    /// position with no good captures isn't forced into playing one. Same
    /// fail-soft alpha-beta and abort propagation as `negamax`. Deliberately
    /// out of scope: mate detection (an actual checkmate mid-quiescence
    /// just falls back to its, in that case misleading, stand-pat score)
    /// and repetition detection (captures are irreversible, so a genuine
    /// repetition inside a pure-capture line can't occur); neither is an
    /// oversight.
    ///
    /// `qdepth` counts down from [`MAX_QUIESCENCE_DEPTH`] and is unrelated
    /// to `negamax`'s own `ply`: it bounds how many more plies of captures
    /// this call will still resolve, not the distance from the root. At
    /// `qdepth == 0`, this behaves as if no captures were available,
    /// falling back to `stand_pat`.
    ///
    /// `moves`: `Some` when the caller (`negamax`/`search_root`) already
    /// generated the full move list for its own mate/stalemate check, so
    /// this doesn't generate the same board's moves twice; `None` (this
    /// function's own recursive calls, where `board` is a fresh child)
    /// generates here instead.
    ///
    /// No `self.history` push/pop, unlike `negamax`: this function never
    /// calls `is_draw`, so a capture-only line has nothing to need it for.
    fn quiescence(
        &mut self,
        board: &Board,
        mut alpha: Score,
        beta: Score,
        qdepth: u8,
        moves: Option<MoveList>,
    ) -> Option<Score> {
        self.nodes += 1;
        if self.should_abort() {
            return None;
        }

        let stand_pat = evaluate(board);
        if stand_pat >= beta {
            return Some(stand_pat);
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        let mut max = stand_pat;

        if qdepth > 0 {
            let mut moves = moves.unwrap_or_else(|| legal_moves(board));
            moves.retain(|m| m.flags().is_capture());

            order_moves(board, &mut moves, None);
            for m in &moves {
                let child = board.make_move(*m);
                let score = self.quiescence(&child, -beta, -alpha, qdepth - 1, None);

                let score = -score?;
                if score > max {
                    max = score;
                }

                if max > alpha {
                    alpha = max;
                }
                if alpha >= beta {
                    break;
                }
            }
        }
        Some(max)
    }
}

/// Orders `moves` in place, most promising first, so alpha-beta prunes more
/// of the tree: MVV-LVA (most valuable victim, least valuable attacker)
/// among captures, ahead of quiet moves, since a capture that wins the most
/// material with the cheapest piece is most likely to hold up and cause a
/// beta cutoff early. `tt_move`, when `Some`, ranks ahead of all of that; see
/// [`move_priority`].
///
/// En passant's victim isn't actually on `m.to()`; scored as `0` (an
/// equal-value trade) rather than looking up the true victim square, an
/// accepted ordering approximation since it only affects search order, not
/// legality or correctness.
///
/// Uses [`MoveList::as_mut_slice`] to sort in place without a second
/// allocation.
fn order_moves(board: &Board, moves: &mut MoveList, tt_move: Option<Move>) {
    moves.sort_unstable_by_key(|&m| move_priority(board, m, tt_move));
}

/// Ranked top to bottom, best move first: `#[derive(PartialOrd, Ord)]` on a
/// fieldless enum compares by declaration order, so this list *is* the
/// ranking, not a lookup table alongside it. The hand-written version this
/// replaced computed the same order through a separate `rank()` match
/// function called on every sort comparison; `benches/search.rs` found no
/// measurable throughput difference between the two at depth 6, so this
/// isn't a performance change, just removing a lookup table that a typo
/// could silently desync from this declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(
    dead_code,
    reason = "PrincipalVariation, KillerCapture, MateKiller, and Killer have no producer yet \
              (real PV tracking needs PVS's triangular PV table, killers need a per-ply killer-move \
              table; both are #38, not this issue) but the ranking scheme is designed to be complete \
              for when they land, rather than needing to be reshuffled later"
)]
enum MovePriority {
    PrincipalVariation,
    Hash,
    KillerCapture,
    WinningCapture,
    EqualCapture,
    MateKiller,
    Killer,
    LosingCapture,
    Quiet,
}

fn move_priority(board: &Board, m: Move, tt_move: Option<Move>) -> MovePriority {
    if Some(m) == tt_move {
        MovePriority::Hash
    } else if m.flags().is_capture() {
        if m.flags().is_en_passant() {
            return MovePriority::EqualCapture;
        }
        let attacker = board
            .piece_at(m.from())
            .expect("capture has an attacker")
            .piece();
        let victim = board
            .piece_at(m.to())
            .expect("capture has a victim")
            .piece();
        let score = PIECE_VALUES[victim.index()] - PIECE_VALUES[attacker.index()];
        match score.cmp(&0) {
            std::cmp::Ordering::Greater => MovePriority::WinningCapture,
            std::cmp::Ordering::Equal => MovePriority::EqualCapture,
            std::cmp::Ordering::Less => MovePriority::LosingCapture,
        }
    } else {
        MovePriority::Quiet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Square;

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

    /// Every variant, in the exact order the hand-written `rank` closure in
    /// `Ord for MovePriority` intends. This is what actually pins the
    /// ranking down: `rank`'s match arms are a hand-written lookup table
    /// (a shape this project has repeatedly gotten wrong elsewhere via a
    /// copy-paste or off-by-one in similar tables), and a typo'd or
    /// duplicated number there wouldn't fail to compile, it would just
    /// silently misorder two variants relative to each other.
    #[test]
    fn move_priority_rank_order_matches_the_intended_hierarchy() {
        use MovePriority::{
            EqualCapture, Hash, Killer, KillerCapture, LosingCapture, MateKiller,
            PrincipalVariation, Quiet, WinningCapture,
        };
        let ranked_best_to_worst = [
            PrincipalVariation,
            Hash,
            KillerCapture,
            WinningCapture,
            EqualCapture,
            MateKiller,
            Killer,
            LosingCapture,
            Quiet,
        ];
        for pair in ranked_best_to_worst.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} must rank strictly ahead of {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn move_priority_with_no_tt_hint_classifies_a_quiet_move_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(move_priority(&board, quiet, None), MovePriority::Quiet);
    }

    // MVV-LVA's whole point: a small attacker taking a large victim (pawn
    // takes queen) is the most promising capture there is, tried before a
    // big attacker taking a small victim (queen takes pawn), which is
    // comparatively unpromising. `WinningCapture`/`LosingCapture` need to
    // land on the *correct side* of that distinction, not just land on two
    // different variants (a naming swap between the two would still produce
    // "two distinct variants" and could still pass a looser test).
    #[test]
    fn move_priority_classifies_a_small_attacker_taking_a_big_victim_as_winning() {
        let board = Board::try_from_fen("4k3/8/8/8/3q4/4P3/8/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_queen = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            move_priority(&board, pawn_takes_queen, None),
            MovePriority::WinningCapture,
            "a pawn capturing a queen is the textbook winning capture"
        );
    }

    #[test]
    fn move_priority_classifies_a_big_attacker_taking_a_small_victim_as_losing() {
        let board = Board::try_from_fen("4k3/8/8/8/3p4/4Q3/8/4K3 w - - 0 1").expect("valid FEN");
        let queen_takes_pawn = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            move_priority(&board, queen_takes_pawn, None),
            MovePriority::LosingCapture,
            "a queen capturing an undefended pawn is comparatively unpromising, \
             the opposite end of the scale from a pawn capturing a queen"
        );
    }

    #[test]
    fn move_priority_classifies_an_equal_value_capture_as_equal() {
        let board = Board::try_from_fen("4k3/8/8/8/3r4/8/8/3RK3 w - - 0 1").expect("valid FEN");
        let rook_takes_rook = find_move(&board, Square::D1, Square::D4);
        assert_eq!(
            move_priority(&board, rook_takes_rook, None),
            MovePriority::EqualCapture
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
            move_priority(&winning_board, pawn_takes_queen, None)
                < move_priority(&losing_board, queen_takes_pawn, None),
            "a pawn capturing a queen must be tried before a queen capturing a pawn"
        );
    }

    #[test]
    fn move_priority_with_matching_tt_hint_is_hash_regardless_of_move_shape() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        assert_eq!(
            move_priority(&board, capture, Some(capture)),
            MovePriority::Hash
        );
        assert_eq!(
            move_priority(&board, quiet, Some(quiet)),
            MovePriority::Hash
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
            move_priority(&board, capture, Some(quiet)),
            move_priority(&board, capture, None),
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

        order_moves(&board, &mut moves, Some(quiet_hint));

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

    /// The issue's own testing note: whichever legal move is handed to
    /// `order_moves` as the hint lands at index 0, regardless of what that
    /// move actually is. Looping over every legal move in the position (the
    /// one capture and several quiet king moves) as the hint in turn checks
    /// this generically rather than pinning it to one move's own kind.
    #[test]
    fn order_moves_places_any_hinted_move_first() {
        let board = capture_and_quiet_position();
        let legal = legal_moves(&board);

        for &hint in legal.as_slice() {
            let mut moves = legal_moves(&board);
            order_moves(&board, &mut moves, Some(hint));
            assert_eq!(
                moves.as_slice()[0],
                hint,
                "hint move {hint:?} must land at index 0 regardless of whether \
                 it's a capture or a quiet move"
            );
        }
    }
}
