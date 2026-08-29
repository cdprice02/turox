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
use std::time::Instant;

/// The score magnitude of a certain checkmate. A node where the side to
/// move has no legal moves and is in check scores `ply as Score - MATE`:
/// very negative, and more negative the *smaller* `ply` is (a mate reached
/// in fewer plies from the root is a faster, more forced mate against that
/// side). One ply up, negamax's sign flip turns that into `MATE - ply` for
/// the side that just delivered it, so a shorter forced mate always
/// outscores a longer one. This exact ply direction is a classic place for
/// an off-by-one to hide silently and look plausible; `tests/search_props.rs`'s
/// mate puzzles are what actually pin the formula down, not this comment.
pub const MATE: Score = 30_000;

/// How many plies past `negamax`'s horizon `quiescence` is allowed to keep
/// resolving captures before it gives up and falls back to stand pat, same
/// as if no more captures were available. Without a cap, a sufficiently
/// tangled position (many mutually-en-prise pieces) can make the *breadth*
/// of the capture tree blow up long before it naturally bottoms out on
/// material alone: `tests/search_props.rs`'s `alpha_beta_agrees_with_unpruned_negamax`
/// property test hit exactly this on an `any_board()`-generated position,
/// burning minutes of CPU on a single case. A handful of plies is typical;
/// too low and captures get cut off mid-exchange again, just one horizon
/// further out, defeating quiescence's own purpose.
///
/// `pub`, not private: `tests/search_props.rs`'s own `naive_quiescence`
/// oracle needs the identical cap, not a hand-copied literal that could
/// drift out of sync and silently turn the property test into a comparison
/// between two different search depths.
pub const MAX_QUIESCENCE_DEPTH: u32 = 8;

/// One completed call to [`Search::search`]: the best move and score found,
/// and the depth actually reached. `depth` can be less than the requested
/// `max_depth` if the search was aborted (deadline, node budget, or
/// `request_stop`) before a deeper iteration finished; see `Search::search`'s
/// doc for why a partial iteration's own result is discarded rather than
/// returned.
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

/// Mutable search state threaded through one [`Search::search`] call: the
/// node counter and abort conditions the periodic check reads, and the
/// repetition hash stack. A struct rather than several parameters threaded
/// through the recursion is what makes the abort check and the draw check
/// cheap to reach at every node.
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
    /// (see `tests/search_props.rs`). UCI's real-clock wiring (a later
    /// issue) is expected to layer `deadline` on top of this mechanism, not
    /// replace it.
    max_nodes: Option<u64>,
    stop: bool,
}

impl Search {
    /// A new search seeded with `history`: the real game's position hashes
    /// so far, not including the position `search` will be called on. No
    /// deadline or node budget by default, so `search` runs every
    /// iteration up to `max_depth` to completion; see `with_deadline`/
    /// `with_max_nodes` to bound it.
    pub fn new(history: Vec<u64>) -> Self {
        Self {
            nodes: 0,
            history,
            deadline: None,
            max_nodes: None,
            stop: false,
        }
    }

    /// Aborts the search once `nodes` reaches `max_nodes`, checked on the
    /// same periodic schedule `deadline` and `stop` are.
    pub fn with_max_nodes(mut self, max_nodes: u64) -> Self {
        self.max_nodes = Some(max_nodes);
        self
    }

    /// Aborts the search once `Instant::now()` passes `deadline`, checked
    /// on the same periodic schedule `max_nodes` and `stop` are.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Requests that the search stop as soon as it's next checked. A later
    /// issue's UCI `stop` handler is expected to call this from another
    /// thread via a shared flag; this synchronous version is the plumbing
    /// that wires into, not the cross-thread mechanism itself.
    pub fn request_stop(&mut self) {
        self.stop = true;
    }

    /// Total nodes visited so far by this `Search`.
    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Whether the search should abort right now. Only actually reads
    /// `stop`/`deadline`/`max_nodes` every 2048th node
    /// (`self.nodes & 2047 == 0`), so the clock read (and even the branch)
    /// is amortized to roughly nothing per node rather than paid on every
    /// single one. Call this immediately after incrementing `self.nodes` at
    /// the top of `negamax`/`quiescence`.
    fn should_abort(&self) -> bool {
        self.nodes & 2047 == 0
            && (self.stop
                || self.deadline.is_some_and(|d| Instant::now() >= d)
                || self.max_nodes.is_some_and(|max| self.nodes >= max))
    }

    /// Iterative deepening driver: repeatedly searches the root at
    /// increasing depths, keeping only the result of the last iteration
    /// that ran to completion.
    ///
    /// Implementation gotchas:
    /// - An iteration interrupted partway through (`search_root` returns
    ///   `None`) must not overwrite the previous iteration's `SearchResult`;
    ///   stop the loop there and return what the depths before it already
    ///   found. Returning a partial iteration's in-progress best move is
    ///   the classic bug here.
    /// - Iterative deepening starts at depth 1, not 0.
    /// - If the position handed in has no legal moves at all, there's
    ///   nothing to deepen into: `search_root` reports that directly (see
    ///   its own doc) rather than `search` special-casing it up front.
    pub fn search(&mut self, board: &Board, max_depth: u32) -> SearchResult {
        let mut result = SearchResult {
            best_move: None,
            score: 0,
            depth: 0,
            nodes: 0,
        };
        for depth in 1..=max_depth {
            match self.search_root(board, depth) {
                None => break,
                Some((score, best_move)) => {
                    result = SearchResult {
                        best_move,
                        score,
                        depth,
                        nodes: self.nodes(),
                    };
                }
            }
        }
        result
    }

    /// One ply of root move loop. Unlike interior `negamax` nodes, the root
    /// needs to remember *which* move produced the best score, not just the
    /// score itself, so it's a separate small loop rather than a `ply == 0`
    /// special case buried inside `negamax`.
    ///
    /// Checks apply to the root exactly as they do to any interior
    /// `negamax` node, same order as `negamax`'s doc:
    /// - `draw::is_draw(board, &self.history, board.hash())` first: a
    ///   position handed to `search` that's already a draw (the caller's
    ///   seeded `history` already has two occurrences of it) scores `0`
    ///   immediately, whatever `depth` was requested.
    /// - Then, no legal moves at all: returns `Some((score, None))` without
    ///   searching anything, `score` being `-MATE` (in check) or `0`
    ///   (stalemate), the ply-0 case of the formula on [`MATE`]'s doc, not a
    ///   separately hand-written literal.
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
            return Some((0, None));
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
                self.quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH)?,
                None,
            ));
        }

        let mut max = Score::MIN;
        let mut best_move = None;
        order_moves(board, &mut moves);
        for &m in moves.iter() {
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
    /// the actual score found, not the clamped `beta` bound (fail-soft, not
    /// fail-hard). `ply` is the distance from the root (0 there), used only
    /// for the mate-score formula on [`MATE`]'s doc and for keeping
    /// `self.history` in step with the recursion.
    ///
    /// Returns `None` if `should_abort()` trips; callers must propagate a
    /// `None` up immediately rather than treating it as a real score.
    ///
    /// Order of operations, each one a real gotcha:
    /// 1. Increment `self.nodes`, then check `should_abort()`.
    /// 2. Check `draw::is_draw(board, &self.history, board.hash())` *before*
    ///    generating moves: an already-drawn position scores `0` regardless
    ///    of material, and shouldn't be searched further.
    /// 3. Generate legal moves. An empty list is checkmate (if
    ///    `in_check(board, board.side_to_move())`) or stalemate, not a draw
    ///    in the fifty-move/repetition sense, and uses the [`MATE`] formula
    ///    at this node's `ply`, not step 2's `0`.
    /// 4. `depth == 0` hands off to `quiescence` rather than returning
    ///    `evaluate(board)` directly, so a capture sequence hanging over the
    ///    horizon gets resolved instead of cut off mid-exchange.
    /// 5. Otherwise, order the moves (`order_moves`) and negamax each child:
    ///    push `board.hash()` onto `self.history` before recursing into a
    ///    child, pop it after, matching `self.history`'s contract on the
    ///    struct doc.
    fn negamax(
        &mut self,
        board: &Board,
        depth: u32,
        ply: u32,
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
                Some(ply as Score - MATE)
            } else {
                Some(0)
            };
        }

        if depth == 0 {
            return self.quiescence(board, alpha, beta, MAX_QUIESCENCE_DEPTH);
        }

        let mut max = Score::MIN;
        order_moves(board, &mut moves);
        for &m in moves.iter() {
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
    /// position with no good captures available doesn't get forced into
    /// playing one. Same fail-soft alpha-beta and abort propagation as
    /// `negamax`. Deliberately out of scope here: mate detection (a
    /// position that's actually checkmate mid-quiescence just falls back to
    /// its, in that case misleading, stand-pat score) and repetition
    /// detection (captures are irreversible, so a genuine repetition inside
    /// a pure-capture line can't occur). Both are noted simplifications, not
    /// oversights.
    ///
    /// `qdepth` counts down from [`MAX_QUIESCENCE_DEPTH`] (set by the
    /// `negamax` call that hands off here) and is unrelated to `negamax`'s
    /// own `ply`: it bounds how many *more* plies of captures this call is
    /// still willing to resolve, not the distance from the root. At
    /// `qdepth == 0`, capture resolution stops and this behaves exactly as
    /// if no more captures were available, falling back to `stand_pat`
    /// (already `max`'s seed value) rather than searching further.
    fn quiescence(
        &mut self,
        board: &Board,
        mut alpha: Score,
        beta: Score,
        qdepth: u32,
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
            let mut moves = legal_moves(board);
            moves.retain(|m| m.flags().is_capture());

            order_moves(board, &mut moves);
            for m in moves.iter() {
                self.history.push(board.hash());

                let child = board.make_move(*m);
                let score = self.quiescence(&child, -beta, -alpha, qdepth - 1);

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
        }
        Some(max)
    }
}

/// Orders `moves` in place, most promising first, so alpha-beta prunes more
/// of the tree: MVV-LVA (most valuable victim, least valuable attacker)
/// among captures, ahead of quiet moves, since a capture that wins the most
/// material with the cheapest piece is the one most likely to hold up and
/// cause a beta cutoff early.
///
/// `PIECE_VALUES[piece as usize]` (re-exported `pub(crate)` from `eval` for
/// exactly this) gives the value scale for both victim and attacker.
/// `board.piece_at(m.to())` is the victim (`None` for a quiet move);
/// `board.piece_at(m.from())` is the attacker (always `Some`: a pseudolegal
/// move always has a piece on its own `from` square). En passant is the one
/// capture whose victim isn't actually on `m.to()`; decide deliberately
/// whether that's worth special-casing here or an acceptable ordering
/// approximation, and say which in a comment either way.
///
/// Uses [`MoveList::as_mut_slice`] (added for exactly this in the
/// `MoveList` mutable-access work) to sort in place without a second
/// allocation.
fn order_moves(board: &Board, moves: &mut MoveList) {
    // purposefully inverted values from "standard" mvv_lva because `sort_unstable_by_key` sorts in ascending order
    let inv_mvv_lva = |m: &Move| {
        let flags = m.flags();
        if flags.is_capture() {
            if flags.is_en_passant() {
                // en passant special case with equal victim and attacker value
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
                PIECE_VALUES[attacker as usize] - PIECE_VALUES[victim as usize]
            }
        } else {
            // non captures are considered less important (at this stage)
            i32::MAX
        }
    };
    moves.sort_unstable_by_key(|a| inv_mvv_lva(a));
}
