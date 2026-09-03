//! Concrete scenario tests for `search::Search`'s public API.
//!
//! `tests/search_props.rs` has the unpruned-negamax-oracle property coverage;
//! these are mate puzzles, score conventions, and time/node-budget behavior
//! that a property over arbitrary boards wouldn't exercise on its own. Every
//! FEN and every named move below was verified directly against this crate's
//! own `legal_moves`/`make_move` before being written down here, not
//! hand-analyzed or taken on faith from a source: this project has shipped
//! hand-authored FEN bugs before, and a FEN pulled from a chess site for
//! Philidor's Legacy turned out not to be a genuinely forced mate as given
//! either.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::search::{Search, MATE};
use turox_engine::{Move, Square};

// ---- Concrete mate puzzles ----
//
// Two named, famous mating patterns rather than arbitrary constructions:
// the back-rank mate (below, in both colors) and Philidor's Legacy, the
// classic smothered mate (further down).
//
// Side to move on the back-rank puzzles is deliberately pinned down
// concretely: one puzzle for each color delivering mate, not just trusting
// the formula symmetric by inspection.

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

/// Mirror of the above for Black being mated.
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

/// A root position that's already a fifty-move draw, but still has plenty
/// of legal moves (a king and two rooks against a lone king): `search`
/// must still return one of them, not `None`. Score stays `0`, same as
/// `seeded_repetition_scores_zero` above.
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

/// Mirror of `fifty_move_draw_at_root_still_returns_a_legal_move`, but a
/// genuine threefold repetition (`history` seeded with two prior
/// occurrences) instead of the fifty-move rule.
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

/// Iterative deepening must not start an iteration with no realistic
/// chance of finishing before `self.deadline`. Self-calibrates against
/// this machine's own timings rather than a hardcoded duration: times an
/// unbounded `depth`-ply search first, then gives a fresh search only `2x`
/// that as its whole budget and asks for `depth + 1`. Kiwipete's real
/// branching factor clears the `4x` safety margin easily, so the ~1x left
/// after `depth` completes is nowhere near enough for `depth + 1`; the
/// soft limit must catch that and return `depth`'s own result.
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

/// The regression case that matters most: `go depth N` (no `with_deadline`
/// call, the majority shape of this suite's own calls) must still reach
/// exactly the requested depth. Kiwipete specifically, the same branchy
/// position the soft limit test above deliberately trips the limit on.
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

/// A `max_nodes`-only search (no deadline) is equally unaffected: the soft
/// limit only ever reasons about `self.deadline`.
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
