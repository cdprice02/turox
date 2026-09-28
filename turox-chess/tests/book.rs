//! Concrete tests for `book`: the opening book's file format and lookup.
//!
//! `tests/book_props.rs` has the property coverage over arbitrary hashes,
//! move sets, and seeds; this file pins the specific concrete scenarios a
//! property alone wouldn't reliably hit: round-tripping through bytes,
//! rejecting a too-short byte stream, rejecting a fingerprint mismatch, and
//! the weighting actually mattering rather than just being random.

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
