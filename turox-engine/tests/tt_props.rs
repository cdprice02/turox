//! Property test for `search::tt::Tt`.

mod common;

/// Any legal move, for tests that need a syntactically valid one to store and
/// do not care which. `Move` has no public constructor by design, so this goes
/// through move generation rather than fabricating bits.
fn a_move() -> turox_engine::types::Move {
    *legal_moves(&Board::start_pos())
        .as_slice()
        .first()
        .expect("the start position has legal moves")
}

use common::any_board_and_legal_move;
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::eval::Score;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::search::tt::Tt;
use turox_engine::search::MATE;

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

/// `hashfull` is what UCI reports so a GUI can tell whether the table is
/// sized sensibly for the time control, so the two ends of its range are
/// worth pinning concretely rather than only through a property.
#[test]
fn hashfull_is_zero_for_a_table_nothing_has_been_stored_in() {
    let tt = Tt::new(1);
    assert_eq!(tt.hashfull(), 0, "a fresh table holds nothing");
}

/// Every entry in the sampled prefix occupied means 1000 permille, not 100:
/// UCI asks for permille, and confusing the two is the kind of off-by-ten
/// that looks plausible in a GUI right up until the table is genuinely full.
#[test]
fn hashfull_reports_permille_not_percent_when_the_sample_is_saturated() {
    let mut tt = Tt::new(1);
    let mv = *legal_moves(&Board::start_pos())
        .as_slice()
        .first()
        .expect("the start position has legal moves");

    // Fill well past the sampled prefix. Keys are the raw index here rather
    // than real Zobrist hashes: `store` masks the key to an index, so a
    // contiguous run of small integers lands on a contiguous run of slots,
    // which is exactly the prefix `hashfull` samples.
    for key in 0..4096u64 {
        tt.store(key, 0, 1, 0, -25_000, 25_000, mv);
    }

    assert_eq!(
        tt.hashfull(),
        1000,
        "a saturated sample is 1000 permille, not 100"
    );
}

/// Occupying half the sampled prefix reads as half. Guards the direction of
/// the ratio: `occupied / sample` and `sample / occupied` both produce
/// plausible-looking numbers, and only one is right.
#[test]
fn hashfull_scales_with_occupancy() {
    let mut tt = Tt::new(1);
    let mv = *legal_moves(&Board::start_pos())
        .as_slice()
        .first()
        .expect("the start position has legal moves");

    for key in 0..500u64 {
        tt.store(key, 0, 1, 0, -25_000, 25_000, mv);
    }

    assert_eq!(
        tt.hashfull(),
        500,
        "500 of the 1000 sampled slots are taken"
    );
}

// Mate-score handling through the transposition table.
//
// The table stores scores in a form independent of how a node was reached, so
// a mate score has to have its distance-from-root removed on store and
// re-added on probe. An ordinary positional score has no such component and
// must survive untouched. Getting this wrong produces no panic and no failing
// test elsewhere: it surfaces as the engine reporting a mate for the wrong
// side, which is how it was found (a lost game where turox announced `#110`
// one move before being checkmated).
//
// The three cases below are deliberately separate rather than one property,
// because they fail independently and one of them passed all along.

/// A quiet positional score has no distance-from-root component, so probing at
/// a different ply than it was stored at must return it unchanged.
#[test]
fn a_quiet_score_is_unchanged_by_the_ply_it_is_probed_at() {
    let mut tt = Tt::new(1);
    let key = 0xDEAD_BEEF_1234_5678;
    tt.store(key, 2, 5, 50, -MATE, MATE, a_move());
    let entry = tt.probe(key).expect("just stored");

    assert_eq!(
        entry.cutoff_score(1, -MATE, MATE, 2),
        Some(50),
        "probing at the ply it was stored at must round-trip"
    );
    assert_eq!(
        entry.cutoff_score(1, -MATE, MATE, 6),
        Some(50),
        "a quiet score must not shift with the probing ply"
    );
}

/// Being mated: `negamax` scores this as `ply - MATE`, so the same mate
/// reached at a deeper ply is further away and must score accordingly.
///
/// This case has always worked, because the unconditional adjustment the table
/// used to apply happens to be exactly the right one for losing scores. It is
/// here to stay green through the fix, not to go from red to green.
#[test]
fn a_losing_mate_re_anchors_to_the_probing_ply() {
    let mut tt = Tt::new(1);
    let key = 0x1111_2222_3333_4444;
    // A node at ply 4 that is mated 4 plies from the root.
    tt.store(key, 4, 3, 4 - MATE, -MATE, MATE, a_move());
    let entry = tt.probe(key).expect("just stored");

    assert_eq!(
        entry.cutoff_score(3, -MATE, MATE, 9),
        Some(9 - MATE),
        "the same mate reached at ply 9 is 9 plies from the root"
    );
}

/// Delivering mate: scored as `MATE - ply`. The mirror of the losing case, and
/// the one the unconditional adjustment gets wrong in both directions at once,
/// so the error compounds to twice the ply difference and can push the score
/// past `MATE` entirely.
#[test]
fn a_winning_mate_re_anchors_to_the_probing_ply() {
    let mut tt = Tt::new(1);
    let key = 0x5555_6666_7777_8888;
    // A node at ply 4 that mates 4 plies from the root.
    tt.store(key, 4, 3, MATE - 4, -MATE, MATE, a_move());
    let entry = tt.probe(key).expect("just stored");

    assert_eq!(
        entry.cutoff_score(3, -MATE, MATE, 9),
        Some(MATE - 9),
        "the same mate reached at ply 9 is 9 plies from the root"
    );
}

/// No stored score may ever come back outside the range `negamax` can produce.
/// A score past `MATE` is what turns a subtle distance error into a reported
/// win for the side being mated, since the UCI layer derives the mate's sign
/// from it.
///
/// Scores are derived from the storing ply rather than picked freely: a node at
/// ply `p` cannot score better than `MATE - p`, because the mate has to be
/// delivered somewhere at or below it. Feeding impossible pairs (say `MATE - 1`
/// at ply 23) would fail this on inputs no search can generate, testing the
/// arithmetic's behaviour on garbage instead of its behaviour on real search
/// output.
#[test]
fn a_probed_score_never_escapes_the_mate_range() {
    let mut tt = Tt::new(1);
    for store_ply in 0u8..24 {
        for probe_ply in 0u8..24 {
            let key = u64::from(store_ply) << 32 | u64::from(probe_ply) | 0xABCD_0000_0000_0000;
            let p = Score::from(store_ply);
            // Mates reachable from a node at `store_ply`, plus ordinary scores.
            for score in [MATE - p, MATE - p - 3, p - MATE, p + 3 - MATE, 0, 250, -250] {
                tt.store(key, store_ply, 3, score, -MATE, MATE, a_move());
                let Some(entry) = tt.probe(key) else { continue };
                if let Some(got) = entry.cutoff_score(3, -MATE, MATE, probe_ply) {
                    assert!(
                        got.abs() <= MATE,
                        "score {score} stored at ply {store_ply} came back as {got} \
                         when probed at ply {probe_ply}, outside +/- MATE"
                    );
                }
            }
        }
    }
}
