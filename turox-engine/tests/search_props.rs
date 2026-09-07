//! Property test for `search::Search`, against an unpruned-negamax oracle.
//!
//! No perft equivalent exists here (there's no published ground truth for
//! search, same situation `eval` and `draw` are in), so correctness rests on
//! `naive_negamax`/`naive_quiescence` below: a plain, fully unpruned negamax
//! over the same leaf logic (quiescence, mate/draw scoring) the real `Search`
//! uses. Alpha-beta pruning (correctly implemented) always returns the
//! *identical* score to an unpruned search exploring the same move set at
//! the same depth; it only visits fewer nodes. This doesn't (and can't)
//! validate quiescence's own move selection against anything independent,
//! since both sides call the same one; `tests/search.rs`'s concrete
//! horizon test is for that, along with mate puzzles and the rest of this
//! module's concrete scenario tests. This file is proptest only.
//!
//! Both sides cap capture resolution at `MAX_QUIESCENCE_DEPTH` (the same
//! constant `quiescence` itself uses, not a hand-copied literal): without
//! it, a sufficiently tangled `any_board()` position can make the
//! capture tree's *breadth* blow up long before it naturally bottoms out
//! on material, which is exactly what turned this property test into a
//! multi-minute-per-case runaway before the cap existed. The default-gate
//! version of the property below further restricts itself to sparse
//! boards for the same reason, on top of the cap; the `#[ignore]`d
//! variant restores full `any_board()` density for occasional thorough
//! verification, the same cheap-default/thorough-ignored split
//! `zobrist_props.rs` uses for its own perft-tree walk.

mod common;

use common::any_board_with_legal_move;
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::{evaluate, Score};
use turox_engine::move_gen::attacks::in_check;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::search::draw;
use turox_engine::search::tt::Tt;
use turox_engine::search::{Search, MATE, MAX_QUIESCENCE_DEPTH};

// ---- Independent reference ----

/// Unpruned negamax: always explores every legal move and takes the best,
/// never narrowing an alpha/beta window. Shares `draw`/`in_check`/
/// `legal_moves`/`evaluate` with the real implementation (those are
/// themselves already independently tested elsewhere), but not `Search`'s
/// own pruning, ordering, or abort logic.
fn naive_negamax(board: &Board, depth: u8, ply: u8, history: &mut Vec<u64>) -> Score {
    if draw::is_draw(board, history, board.hash()) {
        return 0;
    }
    let moves = legal_moves(board);
    if moves.is_empty() {
        return if in_check(board, board.side_to_move()) {
            Score::from(ply) - MATE
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
fn naive_quiescence(board: &Board, qdepth: u8) -> Score {
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
    fn alpha_beta_agrees_with_unpruned_negamax(board in sparse_board_with_legal_move(), depth in 1u8..=2) {
        let expected = naive_negamax(&board, depth, 0, &mut Vec::new());
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        prop_assert_eq!(result.depth, depth, "no deadline/node budget set, so every iteration up to depth should complete");
        prop_assert_eq!(result.score, expected);
    }

    #[test]
    fn best_move_is_always_legal(board in any_board_with_legal_move(), depth in 1u8..=2) {
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        let best_move = result.best_move.expect("board has a legal move, so search must return one");
        prop_assert!(legal_moves(&board).as_slice().contains(&best_move));
    }

    #[test]
    fn search_is_deterministic(board in any_board_with_legal_move(), depth in 1u8..=2) {
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
    fn alpha_beta_agrees_with_unpruned_negamax_at_full_density(board in any_board_with_legal_move(), depth in 1u8..=2) {
        let expected = naive_negamax(&board, depth, 0, &mut Vec::new());
        let mut search = Search::new(Vec::new());
        let result = search.search(&board, depth);
        prop_assert_eq!(result.depth, depth);
        prop_assert_eq!(result.score, expected);
    }
}

/// Root randomization must never cost strength: the score reported with it on
/// has to match the score reported with it off, because shuffling only changes
/// *which* of several equally-good moves is chosen, never how good the chosen
/// one is.
///
/// This is the property that makes the feature safe to ship without an SPRT
/// verdict of its own. If it ever fails, randomization is picking worse moves
/// rather than reordering equal ones.
#[test]
fn root_randomization_never_changes_the_score() {
    let board = Board::start_pos();
    let baseline = Search::new(Vec::new()).search(&board, 4);

    for seed in 1..40u64 {
        let randomized = Search::new(Vec::new())
            .with_root_randomization(seed)
            .search(&board, 4);
        assert_eq!(
            randomized.score, baseline.score,
            "seed {seed} changed the root score, so it picked a worse move"
        );
    }
}

/// The behaviour the feature exists for. A deterministic engine played
/// byte-identical games on lichess; over a spread of seeds the chosen move has
/// to actually vary, or nothing has been fixed.
///
/// A `Vec` rather than a `HashSet` because `Move` deliberately implements
/// neither `Hash` nor `Ord`; comparing against the first result answers the
/// question without needing either.
#[test]
fn root_randomization_actually_varies_the_chosen_move() {
    let board = Board::start_pos();
    let chosen: Vec<_> = (1..60u64)
        .map(|seed| {
            Search::new(Vec::new())
                .with_root_randomization(seed)
                .search(&board, 3)
                .best_move
                .expect("start position has legal moves")
        })
        .collect();

    let first = chosen[0];
    assert!(
        chosen.iter().any(|m| *m != first),
        "every seed produced the same move, so randomization is inert"
    );
}

/// Same seed, same result. Randomization is opt-in partly so that a caller who
/// wants reproducibility can still have it, which requires the seed to fully
/// determine the outcome.
#[test]
fn root_randomization_is_reproducible_for_a_given_seed() {
    let board = Board::start_pos();
    let a = Search::new(Vec::new())
        .with_root_randomization(12345)
        .search(&board, 4);
    let b = Search::new(Vec::new())
        .with_root_randomization(12345)
        .search(&board, 4);
    assert_eq!(a, b, "a fixed seed must fully determine the search result");
}

/// xorshift64* maps zero to zero forever, so a zero seed would silently produce
/// an all-zero sequence and a shuffle that never moves anything. `Search`
/// forces it nonzero rather than letting that fail quietly, which is the one
/// case where "it still works" and "it silently did nothing" look identical
/// from the outside.
#[test]
fn a_zero_seed_is_forced_nonzero_rather_than_silently_disabling_the_shuffle() {
    let board = Board::start_pos();
    let baseline = Search::new(Vec::new()).search(&board, 3);

    let zero = Search::new(Vec::new())
        .with_root_randomization(0)
        .search(&board, 3);
    assert_eq!(zero.score, baseline.score, "score must be unaffected");

    let again = Search::new(Vec::new())
        .with_root_randomization(0)
        .search(&board, 3);
    assert_eq!(zero, again, "a zero seed must still be reproducible");

    // The real check: a seed of 0 must behave like the seed it is remapped to,
    // not like "no randomization at all". Comparing against an explicit 1
    // pins that remapping rather than just asserting nothing crashed.
    let one = Search::new(Vec::new())
        .with_root_randomization(1)
        .search(&board, 3);
    assert_eq!(
        zero, one,
        "a zero seed should be remapped to 1, not left as a no-op shuffle"
    );
}

/// A transposition table is a memoisation of a pure function, so a search that
/// uses one must return exactly what the same search returns without one. Any
/// score difference is a table bug by definition, which makes this the one
/// check that needs no independent oracle.
///
/// This is the property that would have caught the mate-score corruption
/// directly. The existing round-trip property could not: it probes at the same
/// ply it stored at, where the adjustment applied on store and the one applied
/// on probe cancel exactly.
///
/// Depths are small on purpose. The no-table side is a search with its
/// memoisation removed, so it pays full price at this engine's branching
/// factor; the deeper sweep that actually exercises accumulated drift is the
/// `#[ignore]`d test below, following the same split the deep perft depths
/// already use.
#[test]
fn a_warm_transposition_table_never_changes_a_score() {
    assert_table_never_changes_scores(&[
        // The position turox got this wrong in: one legal move, forced mate
        // against it.
        ("8/2p3pp/1p3k2/8/6Kn/5q1P/8/8 w - - 8 53", 5),
        // A bare forced mate.
        ("4k3/8/8/8/8/8/6q1/6K1 w - - 0 1", 4),
        // An ordinary midgame position, so the property is not only checked on
        // mate scores, which are the special case rather than the common one.
        (
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            4,
        ),
    ]);
}

/// The deep version of the property above. Mate-score drift grows with the gap
/// between the storing ply and the probing ply, so it is shallow depths that
/// are least likely to catch it; this is the run that would.
#[test]
#[ignore = "searches without a table at depth, minutes; run with --run-ignored all"]
fn a_warm_transposition_table_never_changes_a_score_at_depth() {
    assert_table_never_changes_scores(&[
        ("8/2p3pp/1p3k2/8/6Kn/5q1P/8/8 w - - 8 53", 10),
        ("4k3/8/8/8/8/8/6q1/6K1 w - - 0 1", 8),
        (
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            6,
        ),
    ]);
}

fn assert_table_never_changes_scores(cases: &[(&str, u8)]) {
    for &(fen, max_depth) in cases {
        let board = Board::try_from_fen(fen).expect("test FEN is valid");
        for depth in 1..=max_depth {
            let without = Search::new(Vec::new()).search(&board, depth);
            let mut tt = Tt::new(16);
            let with = Search::new(Vec::new())
                .with_tt(&mut tt)
                .search(&board, depth);

            assert_eq!(
                with.score, without.score,
                "depth {depth} on {fen}: table changed the score from {} to {}",
                without.score, with.score
            );
        }
    }
}

/// The same table reused across successive searches, which is how the UCI
/// session actually drives it: the table outlives any one `go`, so entries
/// stored during a shallow search get probed during a deeper one. That reuse
/// is what let the error accumulate in a real game rather than staying within
/// a single search.
#[test]
fn a_table_reused_across_searches_never_changes_a_score() {
    let board =
        Board::try_from_fen("8/2p3pp/1p3k2/8/6Kn/5q1P/8/8 w - - 8 53").expect("test FEN is valid");
    let mut tt = Tt::new(16);

    for depth in 1u8..=6 {
        let without = Search::new(Vec::new()).search(&board, depth);
        let with = Search::new(Vec::new())
            .with_tt(&mut tt)
            .search(&board, depth);
        assert_eq!(
            with.score, without.score,
            "depth {depth}: a table carried over from earlier depths changed the score"
        );
    }
}

/// No search may report a score outside the range `negamax` can produce. The
/// UCI layer derives a mate's *sign* from this value, so a score past `MATE`
/// does not merely misreport the distance, it can invert which side is mating.
///
/// Every search here uses the table, which is the shape the real game had: the
/// engine reported `mate 118` at depth 64 in a position it was being mated in.
/// Depth 12 is enough to catch it, since the drift was already visible at
/// depth 10 (-30022, printed as `mate 10`); the root has one legal move but
/// the opponent has a queen, so deeper costs real time for no extra coverage.
#[test]
fn a_search_score_never_escapes_the_mate_range() {
    let board =
        Board::try_from_fen("8/2p3pp/1p3k2/8/6Kn/5q1P/8/8 w - - 8 53").expect("test FEN is valid");
    let mut tt = Tt::new(16);

    for depth in 1u8..=12 {
        let score = Search::new(Vec::new())
            .with_tt(&mut tt)
            .search(&board, depth)
            .score;
        assert!(
            score.abs() <= MATE,
            "depth {depth} reported {score}, outside +/- MATE"
        );
        assert!(
            score < 0,
            "depth {depth} reported {score}: this position is a forced mate \
             against the side to move, so the score must stay negative"
        );
    }
}
