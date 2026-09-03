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
