//! Property test for `search::tt::Tt`.
//!
//! `Tt::new`/`probe`/`store`/`Entry::cutoff_score` are still `todo!()` stubs (see
//! `search::tt`'s own doc); this test is written against the settled design ahead of
//! that, per this repo's usual test-before-implementation split. It's expected to panic
//! until those bodies are filled in, not a sign anything here is wrong; delete any
//! `.proptest-regressions` file that shows up from a run against the stub before real
//! work starts, the same as any other stub-phase run.

mod common;

use common::any_board_and_legal_move;
use proptest::prelude::*;
use turox_engine::search::tt::Tt;

proptest! {
    /// Storing a result with `score` strictly inside `(alpha, beta)` always yields
    /// `Bound::Exact` (see `Bound`'s own doc), and `Exact` always qualifies for a cutoff
    /// regardless of depth/bound eligibility beyond `min_depth <= stored depth`. So a
    /// `store` immediately followed by a `probe`/`cutoff_score` at the *same* `ply` and
    /// `depth` (the simplest case: no path-dependence to reconstruct) must recover
    /// exactly the score that went in, once it's round-tripped through the `i16` narrowing
    /// and back.
    #[test]
    fn store_then_probe_at_the_same_node_returns_the_exact_score(
        (board, mv) in any_board_and_legal_move(),
        ply in 0u16..128,
        depth in 0u32..64,
        score in -20_000i32..20_000,
        hash_mb in 1usize..64,
    ) {
        let key = board.hash();
        // Comfortably outside `score`'s own range, so `score` always lands strictly
        // between them: `store` always derives `Bound::Exact` here, never a cutoff bound.
        let alpha = -25_000;
        let beta = 25_000;

        let mut tt = Tt::new(hash_mb);
        tt.store(key, ply, depth, score, alpha, beta, mv);

        let entry = tt.probe(key).expect("just stored under this exact key");
        let recovered = entry
            .cutoff_score(depth, alpha, beta, ply)
            .expect("Bound::Exact at depth >= min_depth always qualifies");

        prop_assert_eq!(recovered, score);
    }
}
