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
use crate::search::draw::is_draw;
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
pub const MAX_QUIESCENCE_DEPTH: u32 = 8;

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
/// than returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchResult {
    /// `None` only when the position handed to `search` itself has no legal
    /// moves (checkmate or stalemate); there is nothing to search.
    pub best_move: Option<Move>,
    /// Side-to-move-relative, same convention as [`evaluate`].
    pub score: Score,
    /// The depth actually completed; see the struct doc for when this is
    /// less than the requested `max_depth`.
    pub depth: u32,
    /// Total nodes visited (negamax and quiescence both count) across every
    /// completed and aborted iteration of this call.
    pub nodes: u64,
}

/// Mutable search state threaded through one [`Search::search`] call: the node counter
/// and abort conditions the periodic check reads, and the repetition hash stack.
///
/// A struct rather than several parameters threaded through the recursion is what makes
/// the abort check and the draw check cheap to reach at every node.
pub struct Search {
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
}

impl Search {
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
    /// An iteration interrupted partway through (`search_root` returns
    /// `None`) must not overwrite the previous iteration's result;
    /// returning a partial iteration's in-progress best move is the
    /// classic bug here. Before starting each iteration past the first,
    /// `ITERATION_TIME_SAFETY_MARGIN` may also stop the loop early rather
    /// than start a doomed one; see its own doc.
    pub fn search(&mut self, board: &Board, max_depth: u32) -> SearchResult {
        self.search_with_info(board, max_depth, |_| {})
    }

    /// Same iterative deepening driver as [`Search::search`], but calls
    /// `on_iteration_complete` with the result of every iteration that
    /// runs to completion, not just the last one; `search` is this method
    /// with a no-op callback. Kept as a separate method (rather than
    /// threading an `Option<impl FnMut>` through `search` itself) so
    /// existing callers that only want a final result (`benches/search.rs`,
    /// most of `tests/search_props.rs`) don't need to know callbacks exist.
    pub fn search_with_info<F: FnMut(&SearchResult)>(
        &mut self,
        board: &Board,
        max_depth: u32,
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
                None => break,
                Some((score, best_move)) => {
                    result = SearchResult {
                        best_move,
                        score,
                        depth,
                        nodes: self.nodes(),
                    };
                    on_iteration_complete(&result);
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
    /// move list here instead. `Some((0, None))` stays reserved for the
    /// true terminal case below: no legal moves at all.
    ///
    /// Returns `None` if the search was aborted before every move at this
    /// depth could be tried; `search` discards a `None` result rather than
    /// returning a partial best move.
    fn search_root(&mut self, board: &Board, depth: u32) -> Option<(Score, Option<Move>)> {
        self.nodes += 1;
        if self.should_abort() {
            return None;
        }

        if is_draw(board, &self.history, board.hash()) {
            let mut drawn_moves = legal_moves(board);
            if drawn_moves.is_empty() {
                return Some((0, None));
            }
            order_moves(board, &mut drawn_moves);
            return Some((0, Some(drawn_moves.as_slice()[0])));
        }

        let mut moves = legal_moves(board);
        if moves.is_empty() {
            return if in_check(board, board.side_to_move()) {
                Some((-MATE, None))
            } else {
                Some((0, None))
            };
        }

        let mut alpha = -MATE;
        let beta = MATE;

        // probably not necessary, but is technically possible by definition of depth being a u32 (it could be 0 even on this root step)
        if depth == 0 {
            return Some((
                self.quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH, Some(moves))?,
                None,
            ));
        }

        let mut max = Score::MIN;
        let mut best_move = None;
        order_moves(board, &mut moves);
        for &m in &moves {
            self.history.push(board.hash());

            let child = board.make_move(m);
            let score = self.negamax(&child, depth - 1, 1, -beta, -alpha);

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
        Some((max, best_move))
    }

    /// Fail-soft negamax with alpha-beta pruning: on a beta cutoff, returns
    /// the actual score found, not the clamped `beta` bound. `ply` is the
    /// distance from the root (0 there), used for the mate-score formula on
    /// [`MATE`]'s doc and for keeping `self.history` in step with the
    /// recursion.
    ///
    /// `ply` is `u16`, not `u32` like `depth`, deliberately: it feeds
    /// `Score::from(ply)` below, and `Score` is `i32`, so `u16` is the widest
    /// type that conversion covers losslessly. Widening `ply` to `u32` to
    /// match `depth` would need an `as` cast right back at that call site.
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
    fn negamax(
        &mut self,
        board: &Board,
        depth: u32,
        ply: u16,
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

        if depth == 0 {
            return self.quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH, Some(moves));
        }

        let mut max = Score::MIN;
        order_moves(board, &mut moves);
        for &m in &moves {
            self.history.push(board.hash());

            let child = board.make_move(m);
            let score = self.negamax(&child, depth - 1, ply + 1, -beta, -alpha);

            self.history.pop();

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
        qdepth: u32,
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

            order_moves(board, &mut moves);
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
/// beta cutoff early.
///
/// En passant's victim isn't actually on `m.to()`; scored as `0` (an
/// equal-value trade) rather than looking up the true victim square, an
/// accepted ordering approximation since it only affects search order, not
/// legality or correctness.
///
/// Uses [`MoveList::as_mut_slice`] to sort in place without a second
/// allocation.
fn order_moves(board: &Board, moves: &mut MoveList) {
    // Inverted from "standard" MVV-LVA: `sort_unstable_by_key` sorts
    // ascending, so the most promising move needs the smallest key.
    let inv_mvv_lva = |m: &Move| {
        let flags = m.flags();
        if flags.is_capture() {
            if flags.is_en_passant() {
                0
            } else {
                let attacker = board
                    .piece_at(m.from())
                    .expect("capture has an attacker")
                    .piece();
                let victim = board
                    .piece_at(m.to())
                    .expect("capture has a victim")
                    .piece();
                PIECE_VALUES[attacker.index()] - PIECE_VALUES[victim.index()]
            }
        } else {
            // non captures are considered less important (at this stage)
            i32::MAX
        }
    };
    moves.sort_unstable_by_key(|a| inv_mvv_lva(a));
}
