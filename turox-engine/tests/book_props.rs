//! Property tests for `book`, over arbitrary hashes, candidate move sets,
//! and seeds. `tests/book.rs` has the concrete scenarios (round-tripping,
//! fingerprint rejection, weight actually mattering); this file covers what
//! has to hold for *every* position and seed, not just a few hand-picked
//! ones: the chosen move is always among the position's candidates, and,
//! with a seeded RNG, repeated calls aren't always identical. Mirrors
//! `search_props.rs`'s root-randomization properties, the existing
//! precedent for testing seeded, weighted choice in this crate.

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use turox_engine::book::{Book, BookMove};
use turox_engine::{Move, MoveFlags, Square};

/// Every `Square`, as an index for building distinct moves without pulling
/// in `tests/common`'s move-generation-oriented `any_square` (this file has
/// no need for `Board` at all: a book entry is just a hash and a set of
/// moves, board-independent by construction).
#[expect(
    clippy::expect_used,
    reason = "clippy.toml's allow-expect-in-tests reaches #[test] fns, not a plain helper like this one"
)]
fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_u8(i).expect("i in 0..64"))
}

fn any_move() -> impl Strategy<Value = Move> {
    (any_square(), any_square()).prop_map(|(from, to)| Move::new(from, to, MoveFlags::Quiet))
}

/// 1 to 5 candidate moves for one book position, each distinct (by `from`/
/// `to`, since every move here is `MoveFlags::Quiet`) and with a positive
/// weight: a book position with a zero-weight-only candidate set, or with
/// two "different" moves that are actually the same move twice, isn't a
/// meaningful scenario for either property below.
fn any_candidate_set() -> impl Strategy<Value = Vec<BookMove>> {
    proptest::collection::vec((any_move(), 1u32..1000), 1..5).prop_map(|pairs| {
        let mut seen = std::collections::HashSet::new();
        pairs
            .into_iter()
            .filter(|(mv, _)| seen.insert((mv.from(), mv.to())))
            .map(|(mv, weight)| BookMove { mv, weight })
            .collect()
    })
}

proptest! {
    /// Whatever `choose` returns is always one of the position's own
    /// candidates, never a move the book never recorded for that hash.
    #[test]
    fn chosen_move_is_always_among_the_position_candidates(
        hash: u64,
        candidates in any_candidate_set(),
        seed: u64,
    ) {
        let book = Book::new(vec![(hash, candidates.clone())]);
        let chosen = book.choose(hash, seed);

        prop_assert!(
            chosen.is_some_and(|m| candidates.iter().any(|bm| bm.mv == m)),
            "choose({hash:#x}, {seed}) returned {chosen:?}, not one of {candidates:?}"
        );
    }

    /// Same hash, same candidates, same seed: the same choice every time.
    /// Matches `root_randomization_is_reproducible_for_a_given_seed`'s
    /// reasoning: a caller who wants a reproducible run (measurement, a
    /// fixed test) needs this, and it costs nothing real play needs, since
    /// real play reseeds per game.
    #[test]
    fn chosen_move_is_reproducible_for_a_given_seed(
        hash: u64,
        candidates in any_candidate_set(),
        seed: u64,
    ) {
        let book = Book::new(vec![(hash, candidates)]);

        prop_assert_eq!(book.choose(hash, seed), book.choose(hash, seed));
    }

    /// A position outside the book stays outside it regardless of seed:
    /// `choose` on a hash with no entry is `None` for every seed, not just
    /// the ones a smaller example happened to try.
    #[test]
    fn an_out_of_book_hash_never_produces_a_move(unknown_hash: u64, seed: u64) {
        let book = Book::new(vec![]);

        prop_assert_eq!(book.choose(unknown_hash, seed), None);
    }
}

/// An arbitrary fixed hash for the tests below that don't care which
/// position they're probing, only that `choose` behaves consistently for
/// one. Named (rather than a repeated inline literal) so it's grouped
/// cleanly for `clippy::unusual_byte_groupings`.
const PROBE_HASH: u64 = 0x00C0_FFEE;

/// Not a `proptest!` property: this needs many *seeds* for one fixed,
/// generated candidate set, which is a loop over seeds around one proptest
/// draw rather than a property proptest itself shrinks over. Mirrors
/// `root_randomization_actually_varies_the_chosen_move`'s same shape.
#[test]
fn chosen_move_varies_across_seeds_when_multiple_candidates_exist() {
    let mut runner = proptest::test_runner::TestRunner::default();
    let strategy =
        any_candidate_set().prop_filter("need at least 2 distinct moves to vary", |c| c.len() >= 2);

    for _ in 0..20 {
        let candidates = strategy
            .new_tree(&mut runner)
            .expect("strategy generation must not fail")
            .current();
        let book = Book::new(vec![(PROBE_HASH, candidates.clone())]);

        let chosen: std::collections::HashSet<_> = (0u64..50)
            .map(|seed| {
                book.choose(PROBE_HASH, seed.max(1))
                    .map(|m| (m.from(), m.to()))
            })
            .collect();

        assert!(
            chosen.len() > 1,
            "every seed produced the same move among {candidates:?}, so weighting is inert"
        );
    }
}
