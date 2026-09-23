//! PROTOTYPE. Throwaway, not part of the workspace, not to be merged.
//!
//! Question (issue #249): what per-move decision should `alpha_beta_loop`
//! expose, and what should it keep?
//!
//! Run:
//!   rustc --edition 2021 -O tools/proto/per-move-policy.rs -o /tmp/pmp && /tmp/pmp
//!
//! Three real clients are written against each candidate below: late move
//! reductions, futility pruning, and late move pruning. The stubs are just
//! enough to make the borrow checker and the control flow honest; the scores
//! are fake and deterministic, so both designs must produce the identical
//! decision trace over the identical input. If they do, they differ only in
//! interface, which is what this is comparing.

type Score = i32;
type Mv = u16;

const MATE: Score = 30_000;

// ---------------------------------------------------------------------------
// Stubs standing in for the engine
// ---------------------------------------------------------------------------

/// The facts a pruning or reduction heuristic needs about one move. In the
/// real engine each of these costs something: `gives_check` needs a make_move
/// or an attack test, and `see` walks the whole capture sequence on a square.
/// Who computes these, and how many get computed, is one of the things that
/// separates the candidates.
#[derive(Clone, Copy, Debug)]
struct MoveFacts {
    is_quiet: bool,
    is_killer: bool,
    gives_check: bool,
    see: Score,
}

struct Board {
    /// Fake: the facts are canned per move index rather than derived.
    facts: Vec<MoveFacts>,
    /// Counts how many times a fact was actually computed, which is the cost
    /// an eager design pays and a lazy one does not.
    probes: std::cell::Cell<usize>,
}

impl Board {
    fn facts(&self, m: Mv) -> MoveFacts {
        self.probes.set(self.probes.get() + 1);
        self.facts[usize::from(m)]
    }
}

#[derive(Default)]
struct Search {
    nodes: u64,
    /// The decision trace, which is what gets compared between designs.
    trace: Vec<String>,
}

impl Search {
    /// Stands in for `negamax` recursing into a child. Deterministic, and
    /// depth-sensitive so a reduced search returns a different answer from a
    /// full-depth one, which is what makes the re-search logic observable.
    fn child(&mut self, m: Mv, depth: u8, _alpha: Score, _beta: Score) -> Option<Score> {
        self.nodes += 1;
        let base = Score::from(m) * 7 % 40 - 20;
        Some(base + Score::from(depth) * 2)
    }
}

#[derive(Clone, Copy)]
struct LoopCtx {
    ply: u8,
    alpha: Score,
    beta: Score,
    is_pv: bool,
    depth: u8,
}

#[derive(Debug, Default, PartialEq)]
struct Outcome {
    max: Score,
    best: Option<Mv>,
    cutoff_index: Option<usize>,
}

/// The node-level knobs every candidate needs, however it receives them.
#[derive(Clone, Copy)]
struct NodeState {
    static_eval: Score,
    in_check: bool,
    /// The root never prunes or reduces: it must return a real move for every
    /// legal option, and a pruned root move is one the engine can never play.
    allow_pruning: bool,
}

fn reduction_for(index: usize, depth: u8) -> u8 {
    if index < 4 || depth < 3 {
        0
    } else {
        1 + u8::try_from(index / 8).unwrap_or(2).min(2)
    }
}

fn futility_margin(depth: u8) -> Score {
    Score::from(depth) * 150
}

fn lmp_threshold(depth: u8) -> usize {
    3 + usize::from(depth) * usize::from(depth)
}

// ===========================================================================
// DESIGN A: the caller returns a per-move verdict, lazily
// ===========================================================================
//
// Two closures. `decide` is asked about each move as the loop reaches it, and
// receives the loop's *current* alpha. `search_child` still only searches.
// The loop owns the windowing, the re-search, the at-least-one-move guarantee
// and the bookkeeping.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    Search,
    Reduce(u8),
    Skip,
    Stop,
}

/// What `decide` is told about the move it is ruling on. `alpha` is the live
/// value, raised by every earlier move in this loop, which is the reason this
/// cannot be hoisted out of the loop: futility's own condition reads it.
struct MoveCtx {
    index: usize,
    alpha: Score,
    searched: usize,
}

impl Search {
    fn loop_a<D, F>(
        &mut self,
        moves: &[Mv],
        ctx: LoopCtx,
        mut decide: D,
        mut search_child: F,
    ) -> Outcome
    where
        D: FnMut(&mut Self, Mv, &MoveCtx) -> Verdict,
        F: FnMut(&mut Self, Mv, Score, Score, bool, u8) -> Option<Score>,
    {
        let LoopCtx { alpha: mut alpha, beta, is_pv: node_is_pv, depth, .. } = ctx;
        let mut max = Score::MIN;
        let mut best = None;
        let mut cutoff_index = None;
        let mut searched = 0usize;

        for (i, &m) in moves.iter().enumerate() {
            // The at-least-one-move guarantee lives here rather than in every
            // caller: the stalemate trap futility carries is a property of the
            // loop, not of any one heuristic, and a node that pruned every
            // move is indistinguishable from one with no legal moves.
            let verdict = if searched == 0 {
                Verdict::Search
            } else {
                decide(self, m, &MoveCtx { index: i, alpha, searched })
            };
            let reduction = match verdict {
                Verdict::Skip => {
                    self.trace.push(format!("{i}:skip"));
                    continue;
                }
                Verdict::Stop => {
                    self.trace.push(format!("{i}:stop"));
                    break;
                }
                Verdict::Search => 0,
                Verdict::Reduce(r) => r.min(depth.saturating_sub(1)),
            };

            let mut is_pv = node_is_pv;
            let full = depth.saturating_sub(1);
            let reduced = full.saturating_sub(reduction);

            let raw = if searched == 0 {
                self.trace.push(format!("{i}:full"));
                search_child(self, m, -beta, -alpha, is_pv, full)
            } else {
                is_pv = false;
                // Null window, at the reduced depth if this move was reduced.
                let probe = search_child(self, m, -alpha - 1, -alpha, is_pv, reduced);
                self.trace.push(if reduction > 0 {
                    format!("{i}:reduced({reduction})")
                } else {
                    format!("{i}:probe")
                });
                match probe {
                    None => None,
                    // A reduced search that beats alpha is not trustworthy at
                    // that depth: re-run it at full depth before believing it.
                    // This has to happen before the PVS widening below, or a
                    // reduced score gets widened rather than re-deepened.
                    Some(p) if reduction > 0 && -p > alpha => {
                        self.trace.push(format!("{i}:re-deepen"));
                        let deep = search_child(self, m, -alpha - 1, -alpha, false, full);
                        match deep {
                            Some(d) if -d > alpha && (-d < beta || node_is_pv) => {
                                is_pv = true;
                                self.trace.push(format!("{i}:re-widen"));
                                search_child(self, m, -beta, -alpha, is_pv, full)
                            }
                            other => other,
                        }
                    }
                    Some(p) if -p > alpha && (-p < beta || node_is_pv) => {
                        is_pv = true;
                        self.trace.push(format!("{i}:re-widen"));
                        search_child(self, m, -beta, -alpha, is_pv, full)
                    }
                    p => p,
                }
            };

            let Some(score) = raw else { break };
            searched += 1;
            let score = -score;
            if score > max {
                max = score;
                best = Some(m);
            }
            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                cutoff_index = Some(i);
                self.trace.push(format!("{i}:cutoff"));
                break;
            }
        }
        Outcome { max, best, cutoff_index }
    }
}

// ===========================================================================
// DESIGN B: the loop owns the heuristics, the caller hands it facts up front
// ===========================================================================
//
// One closure. The caller precomputes a `MoveFacts` for every move and passes
// the array; the loop applies late move reductions, futility and late move
// pruning itself. Smallest interface of the three, and the loop is the only
// place the heuristics interact.

impl Search {
    fn loop_b<F>(
        &mut self,
        moves: &[Mv],
        facts: &[MoveFacts],
        ctx: LoopCtx,
        node: NodeState,
        mut search_child: F,
    ) -> Outcome
    where
        F: FnMut(&mut Self, Mv, Score, Score, bool, u8) -> Option<Score>,
    {
        let LoopCtx { alpha: mut alpha, beta, is_pv: node_is_pv, depth, .. } = ctx;
        let mut max = Score::MIN;
        let mut best = None;
        let mut cutoff_index = None;
        let mut searched = 0usize;

        for (i, &m) in moves.iter().enumerate() {
            let f = facts[i];
            let mut reduction = 0u8;

            if searched > 0 && node.allow_pruning && !node.in_check && !node_is_pv {
                // Late move pruning: stop taking quiet moves once enough have
                // been tried at low depth.
                if f.is_quiet && !f.is_killer && !f.gives_check && i >= lmp_threshold(depth) {
                    self.trace.push(format!("{i}:stop"));
                    break;
                }
                // Futility: reads the live alpha, which is why this cannot be
                // hoisted above the loop.
                if f.is_quiet
                    && depth <= 2
                    && node.static_eval + futility_margin(depth) < alpha
                {
                    self.trace.push(format!("{i}:skip"));
                    continue;
                }
                // Late move reductions, with the conventional exemptions.
                if !f.is_killer && !f.gives_check && f.see >= 0 {
                    reduction = reduction_for(i, depth).min(depth.saturating_sub(1));
                }
            }

            let mut is_pv = node_is_pv;
            let full = depth.saturating_sub(1);
            let reduced = full.saturating_sub(reduction);

            let raw = if searched == 0 {
                self.trace.push(format!("{i}:full"));
                search_child(self, m, -beta, -alpha, is_pv, full)
            } else {
                is_pv = false;
                let probe = search_child(self, m, -alpha - 1, -alpha, is_pv, reduced);
                self.trace.push(if reduction > 0 {
                    format!("{i}:reduced({reduction})")
                } else {
                    format!("{i}:probe")
                });
                match probe {
                    None => None,
                    Some(p) if reduction > 0 && -p > alpha => {
                        self.trace.push(format!("{i}:re-deepen"));
                        let deep = search_child(self, m, -alpha - 1, -alpha, false, full);
                        match deep {
                            Some(d) if -d > alpha && (-d < beta || node_is_pv) => {
                                is_pv = true;
                                self.trace.push(format!("{i}:re-widen"));
                                search_child(self, m, -beta, -alpha, is_pv, full)
                            }
                            other => other,
                        }
                    }
                    Some(p) if -p > alpha && (-p < beta || node_is_pv) => {
                        is_pv = true;
                        self.trace.push(format!("{i}:re-widen"));
                        search_child(self, m, -beta, -alpha, is_pv, full)
                    }
                    p => p,
                }
            };

            let Some(score) = raw else { break };
            searched += 1;
            let score = -score;
            if score > max {
                max = score;
                best = Some(m);
            }
            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                cutoff_index = Some(i);
                self.trace.push(format!("{i}:cutoff"));
                break;
            }
        }
        Outcome { max, best, cutoff_index }
    }
}

// ===========================================================================
// DESIGN C: design B, but the loop holds the board and looks facts up lazily
// ===========================================================================
//
// The fair version of B: it does not pay for facts it never uses. The price
// is that `alpha_beta_loop` now knows what a Board is, what a capture is, and
// what SEE is, so it is no longer a generic move loop that quiescence could
// also drive.

impl Search {
    fn loop_c<F>(
        &mut self,
        moves: &[Mv],
        board: &Board,
        ctx: LoopCtx,
        node: NodeState,
        mut search_child: F,
    ) -> Outcome
    where
        F: FnMut(&mut Self, Mv, Score, Score, bool, u8) -> Option<Score>,
    {
        let LoopCtx { mut alpha, beta, is_pv: node_is_pv, depth, .. } = ctx;
        let mut max = Score::MIN;
        let mut best = None;
        let mut cutoff_index = None;
        let mut searched = 0usize;

        for (i, &m) in moves.iter().enumerate() {
            let mut reduction = 0u8;
            if searched > 0 && node.allow_pruning && !node.in_check && !node_is_pv {
                let f = board.facts(m);
                if f.is_quiet && !f.is_killer && !f.gives_check && i >= lmp_threshold(depth) {
                    self.trace.push(format!("{i}:stop"));
                    break;
                }
                if f.is_quiet && depth <= 2 && node.static_eval + futility_margin(depth) < alpha {
                    self.trace.push(format!("{i}:skip"));
                    continue;
                }
                if !f.is_killer && !f.gives_check && f.see >= 0 {
                    reduction = reduction_for(i, depth).min(depth.saturating_sub(1));
                }
            }

            let mut is_pv = node_is_pv;
            let full = depth.saturating_sub(1);
            let reduced = full.saturating_sub(reduction);

            let raw = if searched == 0 {
                self.trace.push(format!("{i}:full"));
                search_child(self, m, -beta, -alpha, is_pv, full)
            } else {
                is_pv = false;
                let probe = search_child(self, m, -alpha - 1, -alpha, is_pv, reduced);
                self.trace.push(if reduction > 0 {
                    format!("{i}:reduced({reduction})")
                } else {
                    format!("{i}:probe")
                });
                match probe {
                    None => None,
                    Some(p) if reduction > 0 && -p > alpha => {
                        self.trace.push(format!("{i}:re-deepen"));
                        let deep = search_child(self, m, -alpha - 1, -alpha, false, full);
                        match deep {
                            Some(d) if -d > alpha && (-d < beta || node_is_pv) => {
                                is_pv = true;
                                self.trace.push(format!("{i}:re-widen"));
                                search_child(self, m, -beta, -alpha, is_pv, full)
                            }
                            other => other,
                        }
                    }
                    Some(p) if -p > alpha && (-p < beta || node_is_pv) => {
                        is_pv = true;
                        self.trace.push(format!("{i}:re-widen"));
                        search_child(self, m, -beta, -alpha, is_pv, full)
                    }
                    p => p,
                }
            };

            let Some(score) = raw else { break };
            searched += 1;
            let score = -score;
            if score > max {
                max = score;
                best = Some(m);
            }
            if max > alpha {
                alpha = max;
            }
            if alpha >= beta {
                cutoff_index = Some(i);
                self.trace.push(format!("{i}:cutoff"));
                break;
            }
        }
        Outcome { max, best, cutoff_index }
    }
}

// ---------------------------------------------------------------------------
// Drive all three over the same node
// ---------------------------------------------------------------------------

fn make_board(n: usize) -> Board {
    let facts = (0..n)
        .map(|i| MoveFacts {
            is_quiet: i % 3 != 0,
            is_killer: i == 5,
            gives_check: i == 9,
            see: if i % 7 == 0 { -120 } else { 15 },
        })
        .collect();
    Board { facts, probes: std::cell::Cell::new(0) }
}

fn run_a(moves: &[Mv], board: &Board, ctx: LoopCtx, node: NodeState) -> (Outcome, Vec<String>, u64, usize) {
    let mut s = Search::default();
    // The real call site captures `board` and `depth` and mutates a local
    // taint flag, so the prototype does too: that capture pattern is what the
    // borrow checker has to accept alongside a second `&mut Self` closure.
    let mut tainted = false;
    let out = s.loop_a(
        moves,
        ctx,
        |_s, m, mctx| {
            if mctx.searched == 0 || !node.allow_pruning || node.in_check || ctx.is_pv {
                return Verdict::Search;
            }
            let f = board.facts(m);
            if f.is_quiet && !f.is_killer && !f.gives_check && mctx.index >= lmp_threshold(ctx.depth) {
                return Verdict::Stop;
            }
            if f.is_quiet && ctx.depth <= 2 && node.static_eval + futility_margin(ctx.depth) < mctx.alpha {
                return Verdict::Skip;
            }
            if !f.is_killer && !f.gives_check && f.see >= 0 {
                let r = reduction_for(mctx.index, ctx.depth);
                if r > 0 {
                    return Verdict::Reduce(r);
                }
            }
            Verdict::Search
        },
        |s, m, a, b, _pv, d| {
            tainted |= m % 11 == 0;
            s.child(m, d, a, b)
        },
    );
    let probes = board.probes.get();
    board.probes.set(0);
    let _ = tainted;
    (out, s.trace, s.nodes, probes)
}

fn run_b(moves: &[Mv], board: &Board, ctx: LoopCtx, node: NodeState) -> (Outcome, Vec<String>, u64, usize) {
    let mut s = Search::default();
    // Design B needs every move's facts before the loop starts.
    let facts: Vec<MoveFacts> = moves.iter().map(|&m| board.facts(m)).collect();
    let mut tainted = false;
    let out = s.loop_b(moves, &facts, ctx, node, |s, m, a, b, _pv, d| {
        tainted |= m % 11 == 0;
        s.child(m, d, a, b)
    });
    let probes = board.probes.get();
    board.probes.set(0);
    let _ = tainted;
    (out, s.trace, s.nodes, probes)
}

fn run_c(moves: &[Mv], board: &Board, ctx: LoopCtx, node: NodeState) -> (Outcome, Vec<String>, u64, usize) {
    let mut s = Search::default();
    let mut tainted = false;
    let out = s.loop_c(moves, board, ctx, node, |s, m, a, b, _pv, d| {
        tainted |= m % 11 == 0;
        s.child(m, d, a, b)
    });
    let probes = board.probes.get();
    board.probes.set(0);
    let _ = tainted;
    (out, s.trace, s.nodes, probes)
}

fn main() {
    let n = 24;
    let moves: Vec<Mv> = (0..n as Mv).collect();
    let board = make_board(n);

    for (label, ctx, node) in [
        (
            "depth 6, non-PV, pruning on",
            LoopCtx { ply: 3, alpha: -50, beta: 50, is_pv: false, depth: 6 },
            NodeState { static_eval: -400, in_check: false, allow_pruning: true },
        ),
        (
            "depth 2, non-PV, futility live",
            LoopCtx { ply: 5, alpha: 200, beta: 260, is_pv: false, depth: 2 },
            NodeState { static_eval: -400, in_check: false, allow_pruning: true },
        ),
        (
            "root: pruning off",
            LoopCtx { ply: 0, alpha: -MATE, beta: MATE, is_pv: true, depth: 6 },
            NodeState { static_eval: 0, in_check: false, allow_pruning: false },
        ),
    ] {
        let (oa, ta, na, pa) = run_a(&moves, &board, ctx, node);
        let (ob, tb, nb, pb) = run_b(&moves, &board, ctx, node);
        let (oc, tc, nc, pc) = run_c(&moves, &board, ctx, node);

        println!("\n=== {label} ===");
        println!("A (verdict closure)   outcome={oa:?} nodes={na} fact-probes={pa}");
        println!("B (facts up front)    outcome={ob:?} nodes={nb} fact-probes={pb}");
        println!("C (loop holds board)  outcome={oc:?} nodes={nc} fact-probes={pc}");
        let agree = oa == ob && ob == oc && ta == tb && tb == tc;
        println!("all three agree on outcome and trace: {agree}");
        if !agree {
            println!("  A: {ta:?}");
            println!("  B: {tb:?}");
            println!("  C: {tc:?}");
        }
    }
}
