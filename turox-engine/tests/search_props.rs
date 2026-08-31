//! Property and concrete tests for `search::Search`.
//!
//! No perft equivalent exists here (there's no published ground truth for
//! search, same situation `eval` and `draw` are in), so correctness rests on
//! three independent oracles, matching this project's own naive-reference
//! discipline:
//!
//! 1. `naive_negamax`/`naive_quiescence` below: a plain, fully unpruned
//!    negamax over the same leaf logic (quiescence, mate/draw scoring) the
//!    real `Search` uses. Alpha-beta pruning (correctly implemented) always
//!    returns the *identical* score to an unpruned search exploring the same
//!    move set at the same depth; it only visits fewer nodes. This test
//!    doesn't (and can't) validate quiescence's own move selection against
//!    anything independent, since both sides call the same one; that's what
//!    the concrete horizon test at the bottom of this file is for.
//!
//!    Both sides cap capture resolution at `MAX_QUIESCENCE_DEPTH` (the same
//!    constant `quiescence` itself uses, not a hand-copied literal): without
//!    it, a sufficiently tangled `any_board()` position can make the
//!    capture tree's *breadth* blow up long before it naturally bottoms out
//!    on material, which is exactly what turned this property test into a
//!    multi-minute-per-case runaway before the cap existed. The default-gate
//!    version of the property below further restricts itself to sparse
//!    boards for the same reason, on top of the cap; the `#[ignore]`d
//!    variant restores full `any_board()` density for occasional thorough
//!    verification, the same cheap-default/thorough-ignored split
//!    `zobrist_props.rs` uses for its own perft-tree walk.
//! 2. Mate puzzles, both hand-constructed and drawn from named historical
//!    patterns (the back-rank mate, Philidor's Legacy), every one of them
//!    *engine-verified* (via `legal_moves`/`make_move` directly, not by
//!    inspection): this project has shipped hand-authored FEN bugs before,
//!    and a FEN pulled from a chess site for Philidor's Legacy turned out
//!    not to be a genuinely forced mate as given either.
//! 3. A concrete quiescence horizon test: a poisoned-pawn position where a
//!    depth-1 search without quiescence walks into losing a queen, and one
//!    with quiescence doesn't.

mod common;

use common::any_board;
use proptest::prelude::*;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use turox_engine::board::Board;
use turox_engine::eval::{evaluate, Score};
use turox_engine::move_gen::attacks::in_check;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::search::draw;
use turox_engine::search::{Search, MATE, MAX_QUIESCENCE_DEPTH};
use turox_engine::{Move, Square};

// ---- Independent reference ----

/// Unpruned negamax: always explores every legal move and takes the best,
/// never narrowing an alpha/beta window. Shares `draw`/`in_check`/
/// `legal_moves`/`evaluate` with the real implementation (those are
/// themselves already independently tested elsewhere), but not `Search`'s
/// own pruning, ordering, or abort logic.
fn naive_negamax(board: &Board, depth: u32, ply: u32, history: &mut Vec<u64>) -> Score {
    if draw::is_draw(board, history, board.hash()) {
        return 0;
    }
    let moves = legal_moves(board);
    if moves.is_empty() {
        return if in_check(board, board.side_to_move()) {
            ply as Score - MATE
        } else {
            0
        };
    }
    if depth == 0 {
        return naive_quiescence(board, MAX_QUIESCENCE_DEPTH);
    }
    history.push(board.hash());
    let mut best = Score::MIN;
    for &m in moves.as_slice() {
        let score = -naive_negamax(&board.make_move(m), depth - 1, ply + 1, history);
        best = best.max(score);
    }
    history.pop();
    best
}

/// Unpruned quiescence: stand-pat, then every legal capture, no alpha/beta
/// window. No mate/draw handling, matching the real `quiescence`'s own
/// documented simplifications. `qdepth` mirrors the real `quiescence`'s own
/// cap (seeded from the same [`MAX_QUIESCENCE_DEPTH`] constant, not a
/// hand-copied literal): without it, this reference has no bound at all on
/// how many captures deep it'll chase, and a sufficiently tangled
/// `any_board()` position can make that blow up long before comparing
/// against the real (now-capped) implementation ever gets a chance to.
fn naive_quiescence(board: &Board, qdepth: u32) -> Score {
    let mut best = evaluate(board);
    if qdepth > 0 {
        let mut captures = legal_moves(board);
        captures.retain(|m| m.flags().is_capture());
        for &m in captures.as_slice() {
            let score = -naive_quiescence(&board.make_move(m), qdepth - 1);
            best = best.max(score);
        }
    }
    best
}

fn any_board_with_legal_move() -> impl Strategy<Value = Board> {
    any_board().prop_filter("must have at least one legal move", |board| {
        !legal_moves(board).is_empty()
    })
}

/// `any_board_with_legal_move`, further restricted to at most 8 total
/// pieces (both kings plus up to 6 extra, versus `any_board()`'s own
/// 2-to-22 range). Only used by the default-gate half of
/// `alpha_beta_agrees_with_unpruned_negamax`: a sparse board keeps the
/// unpruned `naive_negamax`/`naive_quiescence` reference's capture-tree
/// breadth small, on top of `MAX_QUIESCENCE_DEPTH`'s own cap on its depth.
/// The `#[ignore]`d thorough variant of that same test restores full
/// density instead.
fn sparse_board_with_legal_move() -> impl Strategy<Value = Board> {
    any_board_with_legal_move().prop_filter(
        "keep the capture tree small enough for the default test gate",
        |board| board.occupied().count() <= 8,
    )
}

// ---- Properties ----

proptest! {
    // A reduced case count on top of the sparse-board restriction above:
    // even bounded, `naive_negamax`/`naive_quiescence`'s cost still grows
    // with depth and density, and this still exercises real alpha-beta
    // pruning against real quiescence on dozens of varied positions, which
    // is the load-bearing check; it doesn't need to also be deep or dense.
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn alpha_beta_agrees_with_unpruned_negamax(board in sparse_board_with_legal_move(), depth in 1u32..=2) {
        let expected = naive_negamax(&board, depth, 0, &mut Vec::new());
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        prop_assert_eq!(result.depth, depth, "no deadline/node budget set, so every iteration up to depth should complete");
        prop_assert_eq!(result.score, expected);
    }

    #[test]
    fn best_move_is_always_legal(board in any_board_with_legal_move(), depth in 1u32..=2) {
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        let best_move = result.best_move.expect("board has a legal move, so search must return one");
        prop_assert!(legal_moves(&board).as_slice().contains(&best_move));
    }

    #[test]
    fn search_is_deterministic(board in any_board_with_legal_move(), depth in 1u32..=2) {
        let result_a = Search::new(Vec::new()).search(&board, depth);
        let result_b = Search::new(Vec::new()).search(&board, depth);
        prop_assert_eq!(result_a, result_b);
    }
}

// A separate `proptest!` block: this one needs its own (larger) case count
// and runs `#[ignore]`d, so it can't share the default-gate block's config
// above.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Same check as `alpha_beta_agrees_with_unpruned_negamax`, but over
    /// full-density `any_board()` positions rather than the default gate's
    /// sparse restriction, for occasional deeper verification that the
    /// sparse restriction isn't hiding anything density-dependent.
    #[test]
    #[ignore = "dense-board sweep, several minutes; run with --run-ignored all"]
    fn alpha_beta_agrees_with_unpruned_negamax_at_full_density(board in any_board_with_legal_move(), depth in 1u32..=2) {
        let expected = naive_negamax(&board, depth, 0, &mut Vec::new());
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        prop_assert_eq!(result.depth, depth);
        prop_assert_eq!(result.score, expected);
    }
}

// ---- Concrete mate puzzles ----
//
// Two named, famous mating patterns rather than arbitrary constructions:
// the back-rank mate (below, in both colors) and Philidor's Legacy, the
// classic smothered mate (further down). Every FEN and every named move
// below was verified directly against this crate's own
// `legal_moves`/`make_move` before being written down here, not
// hand-analyzed or taken on faith from a source, the same discipline the
// PST orientation bug in this project's history should have gotten the
// first time: a position pulled from a chess site for Philidor's Legacy
// turned out not to be a *forced* mate as given (Black can simply capture
// the checking knight instead of walking into the trap), which this
// verification step is exactly what caught.
//
// Side to move on the back-rank puzzles is deliberately the
// `{Color}x{state}`-shaped fact this project's own history says is worth
// pinning down concretely: one puzzle for each color delivering mate, not
// just trusting the formula symmetric by inspection.

/// The back-rank mate: one of the most famous elementary mating patterns in
/// chess, a king boxed in by its own pawns with nowhere to run from a rook
/// or queen check along the back rank. `Ra1-a8` is the only legal move that
/// mates here; confirmed the unique one among 15 legal moves.
#[test]
fn white_delivers_mate_in_one() {
    let board = Board::try_from_fen("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 1);
    assert_eq!(
        result.best_move,
        Some(find_move(&board, Square::A1, Square::A8))
    );
    assert_eq!(result.score, MATE - 1);
}

/// Mirror of `white_delivers_mate_in_one`: same shape, Black to move and
/// mating, `Ra8-a1` the unique mating move among 15 legal moves.
#[test]
fn black_delivers_mate_in_one() {
    let board = Board::try_from_fen("r3k3/8/8/8/8/8/5PPP/6K1 b - - 0 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 1);
    assert_eq!(
        result.best_move,
        Some(find_move(&board, Square::A8, Square::A1))
    );
    assert_eq!(result.score, MATE - 1);
}

/// Philidor's Legacy: the classic smothered-mate combination, first
/// published by Lucena in 1497 and again by Philidor in 1749, still the
/// textbook example of the pattern today. The full combination starts
/// `1.Nf7+ Kg8 2.Nh6+ Kh8`, but move 1 isn't itself forced (Black can play
/// `1...Rxf7` instead of walking into the trap) so it isn't a valid *forced*
/// mate puzzle from that starting square; this test instead starts from the
/// position right after `2...Kh8`, where the finish genuinely is forced:
/// `3.Qg8+!!` (queen sac) `Rxg8` (Black's only legal reply) `4.Nf7#`
/// (smothered: the king can't move, block, or capture, boxed in by its own
/// rook, pawns, and the knight check itself). Both the recapture and the
/// mating move confirmed unique by brute-force search over `legal_moves`.
#[test]
fn philidors_legacy_smothered_mate() {
    let board = Board::try_from_fen("5r1k/6pp/4Q2N/8/8/8/5PPP/6K1 w - - 4 3").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    assert_eq!(
        result.best_move,
        Some(find_move(&board, Square::E6, Square::G8))
    );
    assert_eq!(result.score, MATE - 3);
}

/// A position that's already checkmate, not one move away from it: `search`
/// has nothing to search, and scores it `-MATE` exactly (the ply-0 case of
/// the formula on `MATE`'s doc), for White being mated.
#[test]
fn checkmate_scores_exactly_negative_mate_for_white() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/5PPP/r5K1 w - - 0 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    assert_eq!(result.best_move, None);
    assert_eq!(result.score, -MATE);
}

/// Mirror of the above for Black being mated: same `{Color}x{state}` shape.
#[test]
fn checkmate_scores_exactly_negative_mate_for_black() {
    let board = Board::try_from_fen("R5k1/5ppp/8/8/8/8/8/4K3 b - - 1 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    assert_eq!(result.best_move, None);
    assert_eq!(result.score, -MATE);
}

/// A genuine stalemate (Black to move, not in check, no legal moves) scores
/// exactly `0`, not `-MATE`: the two terminal cases share "no legal moves"
/// but must be told apart by `in_check`.
#[test]
fn stalemate_scores_exactly_zero() {
    let board = Board::try_from_fen("k7/2Q5/1K6/8/8/8/8/8 b - - 0 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    assert_eq!(result.best_move, None);
    assert_eq!(result.score, 0);
}

/// A seeded repetition (the position handed to `search` already occurred
/// twice earlier in the real game, per `history`'s contract) scores exactly
/// `0`, the same integration point `draw::is_draw`'s own unit tests cover in
/// isolation, now checked through `Search` end to end.
#[test]
fn seeded_repetition_scores_zero() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let history = vec![board.hash(), board.hash()];
    let mut search = Search::new(history);
    let result = search.search(&board, 2);
    assert_eq!(result.score, 0);
}

/// A root position that's already a fifty-move draw, but still has plenty of
/// legal moves (White has a king and two rooks against a lone king, dozens
/// of legal moves, completely winning material): `search` must still return
/// one of them rather than `None`, the bug issue #54 reports. The position
/// genuinely is a draw at this exact point, so the score stays `0`, same as
/// `seeded_repetition_scores_zero` above; only `best_move` differs from that
/// test, since here `search` has moves to choose from.
#[test]
fn fifty_move_draw_at_root_still_returns_a_legal_move() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/6R1/R3K3 w - - 100 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 4);
    assert_eq!(result.score, 0);
    let best_move = result
        .best_move
        .expect("dozens of legal moves exist; search must not report None");
    assert!(legal_moves(&board).as_slice().contains(&best_move));
}

/// Mirror of `fifty_move_draw_at_root_still_returns_a_legal_move`, but for a
/// genuine threefold repetition instead of the fifty-move rule: `history` is
/// seeded with two prior occurrences of `board`'s own hash, per
/// `draw::is_threefold_repetition`'s contract, rather than relying on
/// `halfmove_clock`. Same bug, independent trigger condition.
#[test]
fn threefold_repetition_at_root_still_returns_a_legal_move() {
    let board = Board::try_from_fen("4k3/8/8/8/8/8/6R1/R3K3 w - - 0 1").expect("valid FEN");
    let history = vec![board.hash(), board.hash()];
    let mut search = Search::new(history);
    let result = search.search(&board, 4);
    assert_eq!(result.score, 0);
    let best_move = result
        .best_move
        .expect("dozens of legal moves exist; search must not report None");
    assert!(legal_moves(&board).as_slice().contains(&best_move));
}

/// Iterative deepening must return the last *completed* iteration's result,
/// not a deeper iteration's partial one. `with_max_nodes` makes this
/// deterministic and repeatable: a wall-clock deadline can't reliably land
/// mid-iteration in a test.
#[test]
fn interrupted_iteration_keeps_the_last_completed_result() {
    let board = Board::try_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
        .expect("valid FEN");

    let unbounded = Search::new(Vec::new()).search(&board, 1);
    assert_eq!(
        unbounded.depth, 1,
        "sanity check: an unbounded depth-1 search must complete depth 1"
    );

    // A budget past depth 1's node count but comfortably short of depth 3's:
    // exercised directly against `unbounded.nodes` rather than a guessed
    // constant, so this stays correct if node-counting or move ordering
    // changes how many nodes depth 1 costs.
    let mut bounded = Search::new(Vec::new()).with_max_nodes(unbounded.nodes + 1);
    let result = bounded.search(&board, 3);

    assert!(
        result.depth < 3,
        "a tiny node budget must not reach the full requested depth"
    );
    assert!(
        result.best_move.is_some(),
        "the depth-1 iteration completed before the budget tripped, so its move must survive"
    );
}

/// `Search::stop_flag` hands out an `Arc<AtomicBool>` meant to be cloned
/// out *before* moving `Search` to its own thread, so a caller elsewhere
/// (UCI's `stop` handler) can still reach it. Proves the handle really is
/// shared across a thread boundary: set it from a genuinely different
/// thread, then confirm `search` (called afterward, on this thread) sees
/// it and aborts well short of the requested depth. Deterministic rather
/// than racing a running search: the atomic visibility this depends on
/// doesn't change based on when the write happens relative to the read,
/// only on whether it happens through the same shared `Arc`, which is
/// exactly what this checks.
///
/// Not asserting `depth == 0`: `should_abort` only actually reads the flag
/// every 2048th node (see its own doc), so a shallow position can complete
/// a couple of cheap iterations before node count first crosses that
/// boundary, even with the flag already set before `search` was ever
/// called. That's the same amortized-check tradeoff `max_nodes`/`deadline`
/// make too, not something specific to `stop`.
#[test]
fn stop_flag_set_from_another_thread_is_honored() {
    let board = Board::start_pos();
    let mut search = Search::new(Vec::new());
    let stop_flag = search.stop_flag();

    std::thread::spawn(move || {
        stop_flag.store(true, Ordering::Relaxed);
    })
    .join()
    .expect("stop-setting thread must not panic");

    let result = search.search(&board, 50);
    assert!(
        result.depth < 50,
        "the flag was already set before search started, so it must abort well short of depth 50, got {}",
        result.depth
    );
}

/// The one genuinely timing-sensitive test in this suite: a 100ms
/// wall-clock deadline on a branchy position (kiwipete, the same one
/// `benches/perft.rs`/`benches/search.rs` use) returns within a generous
/// tolerance rather than running away past its budget. Marked explicitly as
/// the test that can be flaky on a loaded CI runner, rather than pretending
/// a wall-clock assertion is as reliable as the rest of the suite;
/// `interrupted_iteration_keeps_the_last_completed_result` above already
/// covers the same "return the last completed iteration" behavior
/// deterministically via `with_max_nodes`, so this test's only job is
/// proving the wall-clock path specifically isn't ignored.
#[test]
fn movetime_deadline_returns_within_a_generous_tolerance() {
    let board =
        Board::try_from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .expect("valid FEN");

    let deadline = Instant::now() + Duration::from_millis(100);
    let mut search = Search::new(Vec::new()).with_deadline(deadline);

    let started = Instant::now();
    let result = search.search(&board, 50);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "elapsed {elapsed:?} should stay close to the 100ms budget, not run away past it"
    );
    assert!(
        result.nodes > 0,
        "some search work must have actually happened"
    );
    assert!(result.best_move.is_some());
}

/// A poisoned pawn: `Qd4xd5` looks like a free pawn one ply deep (White up
/// material right after capturing), but `c6xd5` recaptures the queen for a
/// pawn, a horizon effect only quiescence catches. Without quiescence, a
/// depth-1 search evaluates the position immediately after `Qxd5` with
/// nothing but static material, sees White up a pawn, and wrongly prefers
/// it over the safe quiet alternative (`Ke1-f1`, confirmed legal); with
/// quiescence, the leaf after `Qxd5` gets extended through Black's
/// recapture, correctly scoring the queen loss, so search must reject the
/// capture. Confirmed by direct inspection that `Qxd5` and `cxd5` are both
/// legal moves in their respective positions, not assumed.
#[test]
fn quiescence_avoids_a_poisoned_pawn() {
    let board = Board::try_from_fen("4k3/8/2p5/3p4/3Q4/8/8/4K3 w - - 0 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 1);

    let poisoned_capture = find_move(&board, Square::D4, Square::D5);
    assert_ne!(
        result.best_move,
        Some(poisoned_capture),
        "quiescence must see past the horizon that Qxd5 loses the queen to cxd5, not just the immediate material gain"
    );
}

/// Issue #56: iterative deepening must not start an iteration with no
/// realistic chance of finishing before `self.deadline`, wasting the
/// remaining budget on a doomed iteration that gets discarded anyway (per
/// `interrupted_iteration_keeps_the_last_completed_result` above). This
/// self-calibrates against this machine's own real timings rather than a
/// hardcoded millisecond figure, since branching factor and hardware both
/// vary: it times an unbounded `depth`-ply search on kiwipete first, then
/// hands a *fresh* search only `2x` that measured time as its whole budget
/// and asks it for `depth + 1`. Kiwipete's real per-ply branching factor
/// comfortably clears this soft limit's own `4x` safety margin (see
/// `ITERATION_TIME_SAFETY_MARGIN`'s doc), so after `depth` completes
/// (consuming close to the full budget on its own), the ~1x left over is
/// nowhere near enough for `depth + 1` by even the conservative `4x`
/// estimate; the soft limit must catch that and return `depth`'s own
/// result rather than starting `depth + 1` at all.
#[test]
fn soft_limit_skips_an_iteration_with_no_realistic_chance_of_finishing() {
    let board =
        Board::try_from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .expect("valid FEN");
    let depth = 3;

    let timed_start = Instant::now();
    let baseline = Search::new(Vec::new()).search(&board, depth);
    let depth_elapsed = timed_start.elapsed();
    assert_eq!(
        baseline.depth, depth,
        "sanity check: an unbounded search must reach the requested depth"
    );

    let deadline = Instant::now() + depth_elapsed * 2;
    let mut bounded = Search::new(Vec::new()).with_deadline(deadline);
    let result = bounded.search(&board, depth + 1);

    assert_eq!(
        result.depth, depth,
        "a budget of only ~2x depth {depth}'s own measured time must not be enough for depth {} \
         on a position whose real branching factor is well above this soft limit's 4x margin, got depth {}",
        depth + 1,
        result.depth
    );
    assert_eq!(
        result.best_move, baseline.best_move,
        "the soft limit must return depth {depth}'s own result unchanged, not some other move"
    );
}

/// The regression case this issue is really about: a `go depth N` UCI
/// command (`Search::search` with no `with_deadline` call at all, the
/// majority shape of this test suite's own calls) must still reach exactly
/// the requested depth, completely unaffected by the soft limit above.
/// Kiwipete specifically, since it's the same branchy position the soft
/// limit test above deliberately trips the limit on; here, with no
/// deadline set, growth between iterations must never stop the loop early.
#[test]
fn no_deadline_still_reaches_the_exact_requested_depth() {
    let board =
        Board::try_from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .expect("valid FEN");
    let result = Search::new(Vec::new()).search(&board, 4);
    assert_eq!(
        result.depth, 4,
        "no deadline was set, so the soft limit must never apply and depth 4 must complete"
    );
}

/// A `max_nodes`-only search (no deadline set) is equally unaffected: the
/// soft limit only ever reasons about `self.deadline`, so a generous node
/// budget that never actually trips `should_abort` must still reach the
/// exact requested depth, same as `no_deadline_still_reaches_the_exact_requested_depth`.
#[test]
fn max_nodes_only_search_unaffected_by_soft_limit() {
    let board =
        Board::try_from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .expect("valid FEN");

    let unbounded = Search::new(Vec::new()).search(&board, 4);
    assert_eq!(unbounded.depth, 4, "sanity check on the unbounded baseline");

    let mut bounded = Search::new(Vec::new()).with_max_nodes(unbounded.nodes * 2);
    let result = bounded.search(&board, 4);
    assert_eq!(
        result.depth, 4,
        "a node budget generous enough to never trip should_abort must still reach depth 4, \
         the soft limit must not apply without a deadline"
    );
}

/// Looks `from`/`to` up against `board`'s own legal moves, rather than
/// constructing `Move` values by hand: no UCI string parser exists yet
/// (that's a later, UCI-specific issue), and this stays honest about which
/// move is actually legal in the position rather than assuming one is.
fn find_move(board: &Board, from: Square, to: Square) -> Move {
    *legal_moves(board)
        .as_slice()
        .iter()
        .find(|m| m.from() == from && m.to() == to)
        .expect("move must be legal in this position")
}
