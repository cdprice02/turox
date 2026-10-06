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

#![expect(
    clippy::expect_used,
    reason = "`clippy.toml`'s allow-expect-in-tests reaches `#[test]` functions and `#[cfg(test)]` modules, but not plain helpers in an integration test or bench, where a failed fixture should abort the run"
)]

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use turox_chess::board::Board;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::{Move, Square};
use turox_engine::search::{is_mate_score, CutoffCause, Search, MATE, MAX_QUIESCENCE_DEPTH};

// ---- Shared fixture ----

/// Kiwipete, the wide-open middlegame position from the perft suite that
/// `benches/perft.rs` and `benches/search.rs` also use, and what the tests
/// below reach for when they need a real tree rather than a constructed
/// position. It is dense enough that an affordable depth still costs orders of
/// magnitude more than any stop bound allows, so "it stopped" and "it ran to
/// completion" cannot be mistaken for each other here.
const OPEN_MIDDLEGAME: &str =
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

fn open_middlegame() -> Board {
    Board::try_from_fen(OPEN_MIDDLEGAME).expect("valid FEN")
}

/// More depth than a stopped search reaches, and no more than that.
///
/// A test of a stop mechanism has to ask for a depth the search cannot reach
/// while the mechanism works, or it is not testing the mechanism. What it must
/// not do is ask for a depth the search cannot reach at all. Depth 50 reads as
/// "effectively infinite" and behaves that way: with the bound's own check
/// mutated away, the search never comes back, so the test hangs rather than
/// fails and the mutant can only be reported as a timeout, which is what a
/// merely slow machine also looks like. Depth 7 here costs 3.1M nodes, seconds
/// rather than forever, so a search that ignores its bound finishes and fails
/// an assertion instead.
const DEPTH_NO_BOUNDED_SEARCH_REACHES: u8 = 7;

/// The same bound for the start position, which is cheaper per ply: 1.2M nodes
/// at depth 9 against 3.1M at depth 7 of `open_middlegame`.
const START_POS_DEPTH_NO_BOUNDED_SEARCH_REACHES: u8 = 9;

/// A node budget for a depth-6 search of `open_middlegame`, roughly twice the
/// 858k nodes one actually costs.
///
/// What it buys is a failure mode. A mutant that flattens the evaluation or
/// scrambles move ordering does not break termination, it inflates the tree by
/// an order of magnitude or two, and an unbounded test answers that with the
/// right result eventually, which `cargo mutants` can only report as a timeout.
/// Under a budget the same mutant comes back shallow and the assertions fail,
/// which is evidence rather than the absence of it.
///
/// Twice, not more. Node counts here are deterministic, so the multiple is
/// headroom for retuning rather than for a loaded machine, and retuning moves a
/// count by tens of percent rather than by multiples. The other half is that a
/// budget costs what it allows whenever it is reached: these tests run once per
/// mutant on a two-core runner, and a margin wide enough to absorb a doubled
/// tree is wide enough to push a bounded test past what the harness will wait
/// for, turning a caught mutant back into the timeout the budget exists to
/// prevent.
const OPEN_MIDDLEGAME_DEPTH_6_BUDGET: u64 = 1_800_000;

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
        result.best_move(),
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
        result.best_move(),
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
        result.best_move(),
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
    assert_eq!(result.best_move(), None);
    assert_eq!(result.score, -MATE);
}

/// Mirror of the above for Black being mated.
#[test]
fn checkmate_scores_exactly_negative_mate_for_black() {
    let board = Board::try_from_fen("R5k1/5ppp/8/8/8/8/8/4K3 b - - 1 1").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    assert_eq!(result.best_move(), None);
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
    assert_eq!(result.best_move(), None);
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
        .best_move()
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
        .best_move()
        .expect("dozens of legal moves exist; search must not report None");
    assert!(legal_moves(&board).as_slice().contains(&best_move));
}

/// When depth 1 itself is the iteration that gets interrupted, there is no
/// earlier completed iteration to fall back on the way
/// `interrupted_iteration_keeps_the_last_completed_result` below relies on;
/// `search` still must not report `best_move: None`, since real legal moves
/// exist.
///
/// `should_abort` only actually samples `max_nodes` every 2048th node (its
/// own doc comment), so a budget merely one short of the true cost, the
/// trick the sibling test below uses, doesn't reliably land mid-loop: for a
/// tree under 2048 nodes total, that checkpoint is never reached at all and
/// the search just runs to completion regardless of the budget. This
/// position is the one from the issue that exposed this bug specifically
/// because its depth-1 tree exceeds 2048 nodes; budgeting half its true
/// cost keeps the next 2048-node checkpoint above the budget yet still
/// strictly below the tree's actual size, so the abort is real and
/// deterministic without needing to know precisely where that checkpoint
/// lands.
#[test]
fn interrupted_first_iteration_still_returns_a_partial_bestmove() {
    let board =
        Board::try_from_fen("r1bqk2r/ppp2ppp/2n5/3np1N1/1bBP4/2P5/PP3PPP/RNBQK2R b KQkq - 0 1")
            .expect("valid FEN");

    let unbounded = Search::new(Vec::new()).search(&board, 1);
    assert_eq!(
        unbounded.depth, 1,
        "sanity check: an unbounded depth-1 search must complete depth 1"
    );
    assert!(
        unbounded.nodes > 2048,
        "this test relies on depth 1's own tree crossing at least one \
         2048-node checkpoint past the halfway budget below; got {} nodes",
        unbounded.nodes
    );

    let mut bounded = Search::new(Vec::new()).with_max_nodes(unbounded.nodes / 2);
    let result = bounded.search(&board, 1);

    let best_move = result
        .best_move()
        .expect("depth 1 aborted, but some moves fully resolved before the abort hit");
    assert_eq!(
        result.depth, 0,
        "depth 1 never actually finished, so the fallback reports depth 0, not 1"
    );
    assert!(legal_moves(&board).as_slice().contains(&best_move));
}

/// Iterative deepening must return the last *completed* iteration's result,
/// not a deeper iteration's partial one. `with_max_nodes` makes this
/// deterministic and repeatable: a wall-clock deadline can't reliably land
/// mid-iteration in a test.
///
/// Depth 4, not depth 3: `should_abort`'s once-per-2048-nodes sampling (see
/// the sibling test above) means a budget only trips if the search's total
/// node count actually crosses a 2048 checkpoint. Move-ordering
/// improvements keep shrinking how many nodes a fixed shallow depth costs
/// from the start position (depth 3 dropped under 2048 entirely once killer
/// moves landed, which is what turned this test flaky), so the budget below
/// is derived from `unbounded.nodes / 2`, the same halfway trick and guard
/// assertion the sibling test uses, rather than a fixed offset from a
/// smaller, faster-shrinking depth.
#[test]
fn interrupted_iteration_keeps_the_last_completed_result() {
    let board = Board::try_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
        .expect("valid FEN");

    let unbounded = Search::new(Vec::new()).search(&board, 4);
    assert_eq!(
        unbounded.depth, 4,
        "sanity check: an unbounded depth-4 search must complete depth 4"
    );
    assert!(
        unbounded.nodes > 2048,
        "this test relies on depth 4's own tree crossing at least one \
         2048-node checkpoint past the halfway budget below; got {} nodes",
        unbounded.nodes
    );

    let mut bounded = Search::new(Vec::new()).with_max_nodes(unbounded.nodes / 2);
    let result = bounded.search(&board, 4);

    assert!(
        result.depth < 4,
        "a tiny node budget must not reach the full requested depth"
    );
    assert!(
        result.best_move().is_some(),
        "an earlier iteration completed before the budget tripped, so its move must survive"
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

    let result = search.search(&board, START_POS_DEPTH_NO_BOUNDED_SEARCH_REACHES);
    assert!(
        result.depth < START_POS_DEPTH_NO_BOUNDED_SEARCH_REACHES,
        "the flag was already set before search started, so it must abort well short of depth {START_POS_DEPTH_NO_BOUNDED_SEARCH_REACHES}, got {}",
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
    let board = open_middlegame();

    let deadline = Instant::now() + Duration::from_millis(100);
    let mut search = Search::new(Vec::new()).with_deadline(deadline);

    let started = Instant::now();
    let result = search.search(&board, DEPTH_NO_BOUNDED_SEARCH_REACHES);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "elapsed {elapsed:?} should stay close to the 100ms budget, not run away past it"
    );
    assert!(
        result.nodes > 0,
        "some search work must have actually happened"
    );
    assert!(result.best_move().is_some());
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
        result.best_move(),
        Some(poisoned_capture),
        "quiescence must see past the horizon that Qxd5 loses the queen to cxd5, not just the immediate material gain"
    );
}

/// The regression case that matters most: `go depth N` (no `with_deadline`
/// call, the majority shape of this suite's own calls) must still reach
/// exactly the requested depth. Kiwipete specifically, the same branchy
/// position the soft limit test above deliberately trips the limit on.
#[test]
fn no_deadline_still_reaches_the_exact_requested_depth() {
    let board = open_middlegame();
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
    let board = open_middlegame();

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

/// The behavioural half of root randomization, pinned with fixed seeds so it
/// cannot flake. `tests/uci_session.rs` covers the session defaulting it *on*,
/// which is inherently a sampling question; this covers what the shuffle
/// actually does, which is not.
///
/// Sweeping a fixed seed range rather than drawing one: a seed decides the
/// permutation outright, so the whole sweep is reproducible, and a failure
/// means the tie set at this depth collapsed rather than that a run got
/// unlucky. That distinction is the entire point of not reading the clock here.
#[test]
fn root_randomization_varies_the_chosen_move_across_seeds() {
    let board = Board::start_pos();

    let chosen: BTreeSet<String> = (1u64..=64)
        .filter_map(|seed| {
            Search::new(Vec::new())
                .with_root_randomization(seed)
                .search(&board, 3)
                .best_move()
                .map(Move::to_uci)
        })
        .collect();

    assert!(
        chosen.len() > 1,
        "root randomization must be able to pick different moves among equal ones, got {chosen:?}"
    );
}

/// The other half: a given seed is reproducible. Without this, the sweep above
/// could pass on a shuffle that ignored its seed entirely and simply varied.
#[test]
fn root_randomization_is_reproducible_for_a_given_seed() {
    let board = Board::start_pos();

    let run = || {
        Search::new(Vec::new())
            .with_root_randomization(0xC0FF_EE12_3456_789A)
            .search(&board, 4)
    };

    let first = run();
    let second = run();
    assert_eq!(
        first.best_move().map(Move::to_uci),
        second.best_move().map(Move::to_uci),
        "the same seed must produce the same move"
    );
    assert_eq!(
        first.score, second.score,
        "the same seed must produce the same score"
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

/// `far`/`near` are the same King+Queen-vs-King position at two different
/// halfmove clocks: `far` nowhere near a fifty-move draw, `near` four
/// half-moves short of it (depth 4 is the shallowest depth that reaches the
/// `96 + 4 == 100` boundary at all). The table's key carries no halfmove
/// clock, so searching `far` first and reusing that table for `near` can
/// serve `near` a stale, decisively-winning score instead of the real draw.
/// Expected to fail. It asserts the behaviour the engine *owes* this
/// position, not the behaviour it has, so the day it passes is the day the
/// halfmove clock reached the key and the ADR recording that gap is stale.
///
/// Kept out of ordinary runs twice over, because no single mechanism reaches
/// every runner: the `accepted_gap_` prefix is what the nextest default
/// filter excludes, and `#[ignore]` is what `cargo test` honours, which is
/// what `cargo llvm-cov` and `cargo mutants` actually shell out to.
#[test]
#[ignore = "expected to fail; pins an accepted transposition-table gap, run \
            via the accepted-gaps nextest profile"]
fn accepted_gap_shared_table_leaks_a_stale_score_across_a_fifty_move_boundary() {
    use turox_engine::search::tt::Tt;

    let far = Board::try_from_fen("7k/8/8/8/8/8/8/K6Q w - - 0 60").expect("valid FEN");
    let near = Board::try_from_fen("7k/8/8/8/8/8/8/K6Q w - - 96 60").expect("valid FEN");
    let depth = 4;

    let mut shared_tt = Tt::new(16);
    Search::new(Vec::new())
        .with_tt(&mut shared_tt)
        .search(&far, depth);
    let near_shared = Search::new(Vec::new())
        .with_tt(&mut shared_tt)
        .search(&near, depth);

    let mut fresh_tt = Tt::new(16);
    let near_fresh = Search::new(Vec::new())
        .with_tt(&mut fresh_tt)
        .search(&near, depth);

    assert_eq!(near_shared.score, near_fresh.score);
}

// ---- Quiescence in check ----
//
// In check, `quiescence` generates evasions and recurses without a depth cap
// rather than standing pat on a capture list, which is an unbounded check
// extension. Out of check it is the capture-only, depth-capped stand-pat
// search.
//
// `expected_quiescence` is an independent reference for both halves, not a
// copy of the real one: it has no alpha/beta window and no transposition
// table, so it agrees with `Search::search` only where the real pruning is
// sound. It calls the real `evaluate`, so these assert an exact match rather
// than a hand-computed number that retuning eval could silently invalidate.
//
// Both scenarios reach their position through one real search move, so
// quiescence is handed it directly. `negamax` resolves its own board's
// terminal case before ever calling quiescence, so routing through it would
// test the wrong function.

fn expected_quiescence(board: &Board, ply: u8, qdepth: u8, history: &mut Vec<u64>) -> i16 {
    use turox_chess::move_gen::attacks::in_check;
    use turox_engine::eval::evaluate;
    use turox_engine::search::draw::is_draw;

    if in_check(board, board.side_to_move()) {
        if is_draw(board, history, board.hash()) {
            return 0;
        }
        let evasions = legal_moves(board);
        if evasions.is_empty() {
            return i16::from(ply) - MATE;
        }
        return evasions
            .as_slice()
            .iter()
            .map(|&m| {
                history.push(board.hash());
                let score = -expected_quiescence(&board.make_move(m), ply + 1, qdepth, history);
                history.pop();
                score
            })
            .max()
            .expect("evasions is non-empty");
    }

    let mut best = evaluate(board);
    if qdepth > 0 {
        let mut captures = legal_moves(board);
        captures.retain(|m| m.flags().is_capture() || m.flags().is_promotion());
        for &m in captures.as_slice() {
            history.push(board.hash());
            let score = -expected_quiescence(&board.make_move(m), ply + 1, qdepth - 1, history);
            history.pop();
            best = best.max(score);
        }
    }
    best
}

/// White's only reasonable move, `Rxa4` (winning a whole rook while also
/// checking), leaves a king whose lone legal reply, `Ka8-b8`, is not a
/// capture: today's capture-only filter leaves quiescence nothing to search
/// there and it stands pat on the check itself instead. Winning the rook
/// makes `Rxa4` the reported best move regardless of whether the reply gets
/// explored, so this isolates the evasion bug's own score contribution
/// rather than needing it to also decide which move `search` reports.
#[test]
fn quiescence_finds_a_forced_non_capture_evasion() {
    let root = Board::try_from_fen("k7/1p6/8/8/r6R/8/8/7K w - - 0 1").expect("valid FEN");
    let rxa4 = find_move(&root, Square::H4, Square::A4);
    let after_rxa4 = root.make_move(rxa4);

    let mut search = Search::new(Vec::new());
    let result = search.search(&root, 1);

    assert_eq!(result.best_move(), Some(rxa4));
    // Seeded with `root`'s own hash: `search_root`'s move loop pushes it
    // before recursing into ply 1, so that's what the real search's own
    // `history` holds by the time it reaches quiescence here too.
    assert_eq!(
        result.score,
        -expected_quiescence(&after_rxa4, 1, MAX_QUIESCENCE_DEPTH, &mut vec![root.hash()])
    );
}

/// Every black root move here is equally hopeless: White's queen captures a
/// pawn on `g7` (`Qxg7`) next regardless of which one Black plays, a plain
/// capture quiescence already explores today, and that capture is
/// checkmate. The bug is what happens at that point, one ply deeper in
/// quiescence's own recursion where `negamax` never looks again: today's
/// code stands pat on it with an ordinary material score instead of the
/// mate it actually is. Deliberately not asserting *which* black move gets
/// reported (every legal one leads to the identical mate, so nothing pins
/// that choice down); `expected_quiescence` is checked against whichever one
/// `search` actually picks.
#[test]
fn quiescence_finds_a_mate_inside_its_own_recursion() {
    let root = Board::try_from_fen("1n5k/6pp/7B/8/8/8/8/Q3K3 b - - 0 1").expect("valid FEN");

    let mut search = Search::new(Vec::new());
    let result = search.search(&root, 1);

    let chosen_move = result.best_move().expect("Black has legal moves here");
    let after_chosen_move = root.make_move(chosen_move);
    assert_eq!(
        result.score,
        -expected_quiescence(
            &after_chosen_move,
            1,
            MAX_QUIESCENCE_DEPTH,
            &mut vec![root.hash()]
        )
    );
    assert!(
        is_mate_score(result.score),
        "a mate found two plies deep inside quiescence must still report as a mate score, got {}",
        result.score
    );
}

// ---- Cutoff-index instrumentation ----

/// Kiwipete's own histogram at a fixed depth, pinned as a floor rather than an
/// exact match: node counts (and so their split across the histogram) shift
/// with pruning/ordering tuning, which is expected to happen, but a
/// well-ordered search should never regress *below* this rate. Each Ordering
/// milestone technique is expected to raise this number, not just avoid
/// lowering it; this documents where it starts.
#[test]
fn negamax_first_move_cutoff_rate_does_not_regress_below_a_known_floor() {
    let board = open_middlegame();
    let result = Search::new(Vec::new())
        .with_max_nodes(OPEN_MIDDLEGAME_DEPTH_6_BUDGET)
        .search(&board, 6);

    let stats = result.negamax_cutoffs;
    assert!(
        stats.fail_high_nodes > 0,
        "this position must actually produce beta cutoffs to search at all, or the \
         rate below is measuring nothing"
    );
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "a diagnostic ratio; node counts are nowhere near f64's 2^52 \
                  exact-integer ceiling, so precision loss here isn't real"
    )]
    let first_move_rate = stats.cutoff_index[0] as f64 / stats.fail_high_nodes as f64;
    assert!(
        first_move_rate >= 0.9,
        "first-move cutoff rate regressed to {first_move_rate:.3}, below the 0.9 floor"
    );
}

// ---- Killer-move instrumentation ----

/// `CutoffStats`'s `Killer` cause count is the pre-SPRT sanity check that the
/// killer table is actually being consulted from inside a real search, not
/// just correct in isolation: it's entirely possible for the table to be
/// wired up, populated, and never once actually looked at by a live move
/// loop, and the existing `first_move_rate` floor above wouldn't catch that
/// on its own since plenty of other ordering already gets a search most of
/// the way to a well-ordered first move.
///
/// Same position and depth as the first-move-rate floor above, for the same
/// reason: it's already known to produce a search deep enough to exercise
/// real ordering, rather than picking a second position and having to
/// re-establish that.
///
/// This asserts only `> 0`, not a specific count or rate: exactly how often
/// a killer fires depends on the replacement policy and where in the tree it
/// happens to land, which is what the unit-level `record_killer`/
/// `move_priority` tests pin down precisely. This test's only job is
/// proving the feature engages at all in a real tree.
#[test]
fn killer_table_is_consulted_during_a_real_search() {
    let board = open_middlegame();
    let result = Search::new(Vec::new())
        .with_max_nodes(OPEN_MIDDLEGAME_DEPTH_6_BUDGET)
        .search(&board, 6);

    assert!(
        result.negamax_cutoffs.by_cause[CutoffCause::Killer.index()] > 0,
        "a depth-6 search of a position this open must cause at least one beta cutoff \
         on a move that was already sitting in a killer slot, or the table isn't being \
         consulted from the real move loop"
    );
}

// ---- Mate-killer instrumentation ----

/// Same shape and reasoning as `killer_table_is_consulted_during_a_real_search`
/// above, for the mate-killer table instead: not a specific count, just proof
/// the feature engages at all. A mate-indicating fail-high is much rarer than
/// an ordinary one, so this needs real depth rather than a contrived position.
/// A table is attached, unlike the killer-table test next door, because
/// searching this deep without one is minutes long even in `--release`.
///
/// Depth 10, which is not the shallowest that works. Cutoff counts on this
/// position run 0, 0, 0, 15, 1754, 16697, 66648 for depths 6 through 12, so
/// depth 9 is where they first appear and fifteen is near enough to zero that
/// the next thing to shrink the tree turns this test into a lie without
/// anyone touching it. Depth 10 buys two orders of magnitude of margin for
/// twice the time, and the depth that merely passes today is the wrong choice
/// for an existence proof.
#[test]
#[ignore = "depth 10 to get a real mate-killer hit, ~11s with a table attached; \
            run with --run-ignored all"]
fn mate_killer_table_is_consulted_during_a_real_search() {
    use turox_engine::search::tt::Tt;

    let board =
        Board::try_from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .expect("valid FEN");
    let mut tt = Tt::new(16);
    let result = Search::new(Vec::new()).with_tt(&mut tt).search(&board, 10);

    assert!(
        result.negamax_cutoffs.by_cause[CutoffCause::MateKiller.index()] > 0,
        "a depth-10 search of a position this open must cause at least one beta cutoff on a \
         move that was already sitting in the mate-killer slot, or the table isn't being \
         consulted from the real move loop"
    );
}

// ---- Cutoff history wiring ----

/// The same "is this actually wired into a real move loop" question
/// `killer_table_is_consulted_during_a_real_search` asks, for `CutoffHistory`:
/// exact scores are `CutoffHistory`'s own unit tests' job (bonus, malus,
/// aging, depth-weighting), and exact ordering consequences are
/// `negamax`'s own `move_priority`/`order_moves` unit tests' job. This only
/// proves `Search::with_cutoff_history` actually reaches the table from a
/// live `search_root`/`negamax` move loop, rather than the table sitting
/// there unread and unwritten.
#[test]
fn search_with_cutoff_history_actually_updates_it() {
    use turox_chess::types::{Color, Piece, Square};
    use turox_engine::search::CutoffHistory;

    let board = open_middlegame();
    let mut history = CutoffHistory::new();
    Search::new(Vec::new())
        .with_cutoff_history(&mut history)
        .with_max_nodes(OPEN_MIDDLEGAME_DEPTH_6_BUDGET)
        .search(&board, 6);

    let any_cell_touched = Color::ALL.into_iter().any(|side| {
        Piece::ALL.into_iter().any(|piece| {
            Square::ALL
                .into_iter()
                .any(|to| history.score(side, piece, to) != 0)
        })
    });
    assert!(
        any_cell_touched,
        "a depth-6 search of a position this open must record at least one quiet-move \
         cutoff or malus somewhere in the table, or it isn't being consulted from the \
         real move loop"
    );
}

/// `philidors_legacy_smothered_mate` above only pins the first move; this
/// pins the *whole* reported line, since that move alone doesn't prove the
/// engine actually found the mate, only that it's willing to sacrifice the
/// queen. `result.pv` should show all three plies of the real combination
/// (`Qe6-g8+ Rxg8 Nf7#`), not just the first move with the rest silently
/// dropped from `info pv`.
#[test]
fn philidors_legacy_reports_the_full_mating_line_not_just_the_first_move() {
    let board = Board::try_from_fen("5r1k/6pp/4Q2N/8/8/8/5PPP/6K1 w - - 4 3").expect("valid FEN");
    let mut search = Search::new(Vec::new());
    let result = search.search(&board, 3);
    let line: Vec<_> = result.pv.into_iter().flatten().collect();

    let queen_sac = find_move(&board, Square::E6, Square::G8);
    let after_sac = board.make_move(queen_sac);
    let forced_recapture = find_move(&after_sac, Square::F8, Square::G8);
    let after_recapture = after_sac.make_move(forced_recapture);
    let knight_mate = find_move(&after_recapture, Square::H6, Square::F7);

    assert_eq!(
        line,
        vec![queen_sac, forced_recapture, knight_mate],
        "the reported pv must be the full three-ply mating combination, not just move 1"
    );
}

// ---- The three ways to stop a search ----
//
// One test per mechanism, each asserting that a search asked to stop actually
// stops. Each is worth a test of its own because a broken mechanism has no
// other symptom to offer: the tests that would notice are the ones that depend
// on stopping in order to finish at all, so they report it by hanging rather
// than by failing, and a hang cannot be told apart from a slow machine.

/// Far below what the first few iterations below cost, so the margin survives
/// the search getting faster. Asserting on the depth reached rather than on a
/// node count keeps that true: node counts move every time pruning improves,
/// and a threshold calibrated against today's tree is one that silently stops
/// testing anything.
const STOPPING_NODE_CAP: u64 = 5_000;

/// `max_nodes` has to actually end the search. Checked through the depth
/// reached, because `Search` reports the last iteration that *completed*: an
/// honoured cap means an iteration got cut off, so the result is shallower than
/// what was asked for.
#[test]
fn a_node_cap_stops_the_search_before_the_asked_for_depth() {
    let result = Search::new(Vec::new())
        .with_max_nodes(STOPPING_NODE_CAP)
        .search(&open_middlegame(), DEPTH_NO_BOUNDED_SEARCH_REACHES);

    assert!(
        result.depth < DEPTH_NO_BOUNDED_SEARCH_REACHES,
        "a search capped at {STOPPING_NODE_CAP} nodes reported depth {} of a requested {DEPTH_NO_BOUNDED_SEARCH_REACHES}, so the cap did not stop it",
        result.depth
    );
    assert!(
        result.nodes < STOPPING_NODE_CAP * 10,
        "a search capped at {STOPPING_NODE_CAP} nodes spent {}, which is too far past the cap to be the 2048-node check schedule",
        result.nodes
    );
}

/// The handle from `stop_flag` has to be the one the search reads. This is the
/// path UCI's `stop` uses from the reader thread while the search runs on its
/// own, so a `stop_flag` that hands back a fresh flag would leave the engine
/// unable to be interrupted at all, which on a real time control means losing
/// on time with a move already in hand.
#[test]
fn a_stop_flag_set_before_the_search_stops_it_before_the_asked_for_depth() {
    let mut search = Search::new(Vec::new());
    search.stop_flag().store(true, Ordering::Relaxed);
    let result = search.search(&open_middlegame(), DEPTH_NO_BOUNDED_SEARCH_REACHES);

    assert!(
        result.depth < DEPTH_NO_BOUNDED_SEARCH_REACHES,
        "a search whose stop flag was already set reported depth {} of a requested {DEPTH_NO_BOUNDED_SEARCH_REACHES}, so the flag from `stop_flag` is not the one it reads",
        result.depth
    );
}

/// `request_stop` is the same contract for a caller on the search's own thread.
/// It is a separate test rather than an alias because it is a separate entry
/// point, and a no-op version of it would leave that caller with no way to stop
/// a search at all.
#[test]
fn request_stop_before_the_search_stops_it_before_the_asked_for_depth() {
    let mut search = Search::new(Vec::new());
    search.request_stop();
    let result = search.search(&open_middlegame(), DEPTH_NO_BOUNDED_SEARCH_REACHES);

    assert!(
        result.depth < DEPTH_NO_BOUNDED_SEARCH_REACHES,
        "a search asked to stop before it began reported depth {} of a requested {DEPTH_NO_BOUNDED_SEARCH_REACHES}, so `request_stop` did not reach `should_abort`",
        result.depth
    );
}
