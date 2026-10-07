//! Tests for `book`: the opening book's file format and lookup.
//!
//! Property tests cover what has to hold for every hash, candidate set and
//! seed: the chosen move is always among the position's candidates, and with a
//! seeded RNG repeated calls are not always identical. Concrete tests pin what
//! a property would not reliably hit: round-tripping through bytes, rejecting a
//! short or mismatched header, the weighting actually mattering, and the
//! shipped book's opening names.

use proptest::prelude::*;
use turox_chess::board::Board;
use turox_chess::book::{default_book, Book, BookLoadError, BookMove};
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::{Move, MoveFlags, Square};

/// A `BookMove`'s `(from, to, flags, weight)`, used to compare book contents
/// independent of return order: `Book::moves` makes no promise about the
/// order candidates come back in, only about which candidates are present.
const fn move_key(bm: &BookMove) -> (u8, u8, MoveFlags, u32) {
    (
        bm.mv.from().to_u8(),
        bm.mv.to().to_u8(),
        bm.mv.flags(),
        bm.weight,
    )
}

fn sorted_keys(moves: &[BookMove]) -> Vec<(u8, u8, MoveFlags, u32)> {
    let mut keys: Vec<_> = moves.iter().map(move_key).collect();
    keys.sort();
    keys
}

const HASH_A: u64 = 0x1234_5678_9abc_def0;
const HASH_B: u64 = 0x0fed_cba9_8765_4321;

const fn e2e4() -> Move {
    Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush)
}

const fn d2d4() -> Move {
    Move::new(Square::D2, Square::D4, MoveFlags::DoublePawnPush)
}

const fn g1f3() -> Move {
    Move::new(Square::G1, Square::F3, MoveFlags::Quiet)
}

#[test]
fn a_position_the_book_never_recorded_returns_no_moves() {
    let book = Book::new(vec![(HASH_A, vec![BookMove::new(e2e4(), 10)])]);

    assert!(
        book.moves(HASH_B).is_empty(),
        "HASH_B was never inserted, so it's out-of-book"
    );
}

#[test]
fn an_empty_book_has_no_moves_for_any_position() {
    let book = Book::new(vec![]);

    assert!(
        book.moves(HASH_A).is_empty(),
        "a book with no entries at all must never report a hit"
    );
}

#[test]
fn a_known_position_returns_exactly_its_recorded_moves() {
    let recorded = vec![
        BookMove::new(e2e4(), 10),
        BookMove::new(d2d4(), 7),
        BookMove::new(g1f3(), 1),
    ];
    let book = Book::new(vec![(HASH_A, recorded.clone()), (HASH_B, vec![])]);

    assert_eq!(
        sorted_keys(book.moves(HASH_A)),
        sorted_keys(&recorded),
        "HASH_A's moves must come back exactly as recorded, order aside"
    );
}

#[test]
fn choose_returns_none_for_an_out_of_book_position() {
    let book = Book::new(vec![(HASH_A, vec![BookMove::new(e2e4(), 10)])]);

    assert_eq!(
        book.choose(HASH_B, 42),
        None,
        "HASH_B has no entry, so there is nothing to choose"
    );
}

#[test]
fn choose_never_returns_a_zero_weight_move_when_a_positive_weight_alternative_exists() {
    let book = Book::new(vec![(
        HASH_A,
        vec![BookMove::new(e2e4(), 5), BookMove::new(d2d4(), 0)],
    )]);

    for seed in 1..500u64 {
        assert_eq!(
            book.choose(HASH_A, seed),
            Some(e2e4()),
            "a weight-0 move must never be chosen over a weight-5 alternative, seed {seed}"
        );
    }
}

#[test]
fn to_bytes_from_bytes_round_trips_every_entry() {
    let book = Book::new(vec![
        (
            HASH_A,
            vec![BookMove::new(e2e4(), 900), BookMove::new(d2d4(), 100)],
        ),
        (HASH_B, vec![BookMove::new(g1f3(), u32::MAX)]),
    ]);

    let bytes = book.to_bytes();
    let round_tripped = Book::from_bytes(&bytes).expect("a book's own bytes must load back");

    assert_eq!(
        sorted_keys(round_tripped.moves(HASH_A)),
        sorted_keys(book.moves(HASH_A))
    );
    assert_eq!(
        sorted_keys(round_tripped.moves(HASH_B)),
        sorted_keys(book.moves(HASH_B))
    );
}

#[test]
fn to_bytes_from_bytes_round_trips_a_positions_name() {
    let book = Book::with_names(
        vec![(HASH_A, vec![BookMove::new(e2e4(), 10)])],
        vec![(HASH_A, "Open Game".to_string())],
    );

    let bytes = book.to_bytes();
    let round_tripped = Book::from_bytes(&bytes).expect("a book's own bytes must load back");

    assert_eq!(round_tripped.name(HASH_A), Some("Open Game"));
}

/// A name belongs to a position, so a position the name table does not
/// mention has none, even when it sits next to one that does and even when
/// both are in the same book.
#[test]
fn a_position_with_no_name_does_not_borrow_a_neighbours() {
    let book = Book::with_names(
        vec![
            (HASH_A, vec![BookMove::new(e2e4(), 10)]),
            (HASH_B, vec![BookMove::new(d2d4(), 5)]),
        ],
        vec![(HASH_A, "Open Game".to_string())],
    );

    let round_tripped =
        Book::from_bytes(&book.to_bytes()).expect("a book's own bytes must load back");

    assert_eq!(round_tripped.name(HASH_A), Some("Open Game"));
    assert_eq!(round_tripped.name(HASH_B), None);
}

/// `name` binary-searches, so it is wrong rather than slow if the table it
/// searches is out of order. Both ways in sort, and this is what says so:
/// the names go in descending and every one is still found.
#[test]
fn names_are_found_however_they_were_ordered_going_in() {
    let mut hashes: Vec<u64> = (0..64u64)
        .map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .collect();
    hashes.sort_unstable();
    hashes.reverse();

    let names: Vec<(u64, String)> = hashes
        .iter()
        .map(|h| (*h, format!("opening {h:016x}")))
        .collect();
    let book = Book::with_names(Vec::new(), names.clone());
    let round_tripped =
        Book::from_bytes(&book.to_bytes()).expect("a book's own bytes must load back");

    for (hash, name) in &names {
        assert_eq!(book.name(*hash), Some(name.as_str()), "in memory");
        assert_eq!(
            round_tripped.name(*hash),
            Some(name.as_str()),
            "round tripped"
        );
    }
}

#[test]
fn from_bytes_rejects_an_empty_byte_stream() {
    assert_eq!(
        Book::from_bytes(&[]),
        Err(BookLoadError::Truncated),
        "no bytes at all can't even hold the fingerprint header"
    );
}

#[test]
fn from_bytes_rejects_a_byte_stream_shorter_than_the_version_and_fingerprint_header() {
    // A real book's own bytes, truncated to 4: a valid version byte plus
    // the first 3 of the fingerprint's 8, so this exercises running out of
    // bytes specifically, not `from_bytes_rejects_an_unsupported_format_version`'s
    // path (which an arbitrary too-short byte string could trip instead,
    // depending on what its first byte happened to be).
    let book = Book::new(vec![(HASH_A, vec![BookMove::new(e2e4(), 1)])]);
    let bytes = book.to_bytes();

    assert_eq!(
        Book::from_bytes(&bytes[..4]),
        Err(BookLoadError::Truncated),
        "4 bytes is shorter than the 1-byte version plus 8-byte fingerprint header"
    );
}

#[test]
fn from_bytes_rejects_an_unsupported_format_version() {
    let book = Book::new(vec![(HASH_A, vec![BookMove::new(e2e4(), 1)])]);
    let mut bytes = book.to_bytes();

    // The version is checked before anything else, including the
    // fingerprint right behind it: flip only byte 0, so a build with the
    // correct fingerprint still rejects a stream claiming a different
    // layout rather than trying to read the rest of it as if it understood
    // that layout.
    bytes[0] = !bytes[0];

    assert_eq!(
        Book::from_bytes(&bytes),
        Err(BookLoadError::UnsupportedVersion),
        "an unrecognized format version must be rejected before the fingerprint is even read"
    );
}

#[test]
fn from_bytes_rejects_a_fingerprint_that_does_not_match_this_build() {
    let book = Book::new(vec![(HASH_A, vec![BookMove::new(e2e4(), 1)])]);
    let mut bytes = book.to_bytes();

    // Flip every bit of the 8-byte fingerprint (bytes 1..9, right after the
    // 1-byte version): a byte can never equal its own bitwise complement,
    // so this is guaranteed to change the stored fingerprint without
    // needing to know its actual value, while leaving the version byte
    // (index 0) untouched so this exercises the fingerprint check
    // specifically, not `from_bytes_rejects_an_unsupported_format_version`'s.
    for byte in &mut bytes[1..9] {
        *byte = !*byte;
    }

    assert_eq!(
        Book::from_bytes(&bytes),
        Err(BookLoadError::FingerprintMismatch),
        "a corrupted fingerprint must be rejected, not silently misread"
    );
}

/// Plays `line` from the start position, for the shipped-book tests below.
fn after(line: &[&str]) -> Board {
    let mut board = Board::start_pos();
    for uci in line {
        let legal = legal_moves(&board);
        let matching: Vec<_> = legal
            .as_slice()
            .iter()
            .filter(|m| m.to_uci() == *uci)
            .collect();
        assert_eq!(matching.len(), 1, "{uci} is exactly one legal move here");
        board = board.make_move(*matching[0]);
    }
    board
}

/// The shipped book's own names, which the format tests above cannot reach:
/// they build books in memory, so nothing else notices if a regeneration
/// produces a book that loads but names the wrong positions.
#[test]
fn the_shipped_book_names_the_openings_its_lines_reach() {
    let book = default_book().expect("the embedded book loads");
    for (line, expected) in [
        (&["e2e4", "c7c5"][..], "Sicilian Defense"),
        (&["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"][..], "Ruy Lopez"),
        (&["g1f3"][..], "Zukertort Opening"),
    ] {
        assert_eq!(
            book.name(after(line).hash()),
            Some(expected),
            "line {line:?}"
        );
    }
}

/// The start position precedes every opening, so it has no name to inherit.
/// It is also reachable from itself (1.Nf3 Nf6 2.Ng1 Ng8), which is what
/// makes this worth pinning: a walk that names by any path rather than the
/// shortest one gives the start position the name of an opening it comes
/// before.
#[test]
fn the_shipped_book_does_not_name_the_start_position() {
    let book = default_book().expect("the embedded book loads");
    assert_eq!(book.name(Board::start_pos().hash()), None);

    assert_eq!(
        after(&["g1f3", "g8f6", "f3g1", "f6g8"]).hash(),
        Board::start_pos().hash(),
        "the knights return to the start position, which is the case this guards"
    );
}

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
            .map(|(mv, weight)| BookMove::new(mv, weight))
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
    /// Matches `a_fixed_seed_fully_determines_the_search_result`'s
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

const fn b1c3() -> Move {
    Move::new(Square::B1, Square::C3, MoveFlags::Quiet)
}

/// Concrete candidate sets, deliberately at moderate weight ratios (no
/// steeper than 3:2) rather than proptest-drawn ones. A proptest weight in
/// `1..1000` can land arbitrarily close to 999:1, and at that ratio the
/// chance that none of 50 fixed seeds lands in the minority slice is real,
/// not negligible: that is what surfaced as a flaky CI failure once
/// already. At these ratios, missing a candidate across 50 independent
/// seeds is astronomically unlikely rather than a coin flip.
fn moderate_weight_candidate_sets() -> Vec<Vec<BookMove>> {
    vec![
        vec![BookMove::new(e2e4(), 2), BookMove::new(d2d4(), 1)],
        vec![BookMove::new(e2e4(), 3), BookMove::new(d2d4(), 2)],
        vec![
            BookMove::new(e2e4(), 1),
            BookMove::new(d2d4(), 1),
            BookMove::new(b1c3(), 1),
        ],
    ]
}

/// Not a `proptest!` property: this needs many *seeds* for a few fixed
/// candidate sets, which is a loop over seeds rather than a property
/// proptest itself shrinks over. Mirrors
/// `root_randomization_actually_varies_the_chosen_move`'s same shape.
#[test]
fn chosen_move_varies_across_seeds_when_multiple_candidates_exist() {
    for candidates in moderate_weight_candidate_sets() {
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
