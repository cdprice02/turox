//! Property test for `search::tt::Tt`.

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
    /// exactly the score that went in.
    #[test]
    fn store_then_probe_at_the_same_node_returns_the_exact_score(
        (board, mv) in any_board_and_legal_move(),
        ply in 0u8..128,
        depth in 0u8..64,
        score in -20_000i16..20_000,
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

    /// A regression property for the exact shape of bug `probe`/`store` had during
    /// implementation: `store` saved `key & mask` instead of the full `key`, and `probe`
    /// compared against `key & mask` too, so two different keys landing on the same index
    /// were indistinguishable. `key ^ (1 << 63)` flips only the top bit: for every
    /// `hash_mb` this test generates, the table's mask never reaches anywhere near bit 63,
    /// so this is guaranteed to land on the same index as `key` while still being a
    /// genuinely different key never passed to `store`.
    #[test]
    fn probe_never_returns_a_hit_for_a_different_key_colliding_at_the_same_index(
        (board, mv) in any_board_and_legal_move(),
        ply in 0u8..128,
        depth in 0u8..64,
        score in -20_000i16..20_000,
        hash_mb in 1usize..64,
    ) {
        let key = board.hash();
        let colliding_key = key ^ (1u64 << 63);

        let mut tt = Tt::new(hash_mb);
        tt.store(key, ply, depth, score, -25_000, 25_000, mv);

        let hit = tt.probe(colliding_key);
        prop_assert!(
            hit.is_none(),
            "probe must not return a hit for a key that was never stored, even one \
             colliding at the same index: {hit:?}"
        );
    }
}
