//! Concrete tests for `book`: the opening book's file format and lookup.
//!
//! `tests/book_props.rs` has the property coverage over arbitrary hashes,
//! move sets, and seeds; this file pins the specific scenarios #196's own
//! testing section calls for: round-tripping through bytes, rejecting a
//! too-short byte stream, rejecting a fingerprint mismatch, and the
//! weighting actually mattering rather than just being random.

use turox_engine::book::{Book, BookLoadError, BookMove};
use turox_engine::{Move, MoveFlags, Square};

/// A `BookMove`'s `(from, to, flags, weight)`, used to compare book contents
/// independent of return order: `Book::moves` makes no promise about the
/// order candidates come back in, only about which candidates are present.
const fn move_key(bm: BookMove) -> (u8, u8, MoveFlags, u32) {
    (
        bm.mv.from().to_u8(),
        bm.mv.to().to_u8(),
        bm.mv.flags(),
        bm.weight,
    )
}

fn sorted_keys(moves: &[BookMove]) -> Vec<(u8, u8, MoveFlags, u32)> {
    let mut keys: Vec<_> = moves.iter().copied().map(move_key).collect();
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
    let book = Book::new(vec![(
        HASH_A,
        vec![BookMove {
            mv: e2e4(),
            weight: 10,
        }],
    )]);

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
        BookMove {
            mv: e2e4(),
            weight: 10,
        },
        BookMove {
            mv: d2d4(),
            weight: 7,
        },
        BookMove {
            mv: g1f3(),
            weight: 1,
        },
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
    let book = Book::new(vec![(
        HASH_A,
        vec![BookMove {
            mv: e2e4(),
            weight: 10,
        }],
    )]);

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
        vec![
            BookMove {
                mv: e2e4(),
                weight: 5,
            },
            BookMove {
                mv: d2d4(),
                weight: 0,
            },
        ],
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
            vec![
                BookMove {
                    mv: e2e4(),
                    weight: 900,
                },
                BookMove {
                    mv: d2d4(),
                    weight: 100,
                },
            ],
        ),
        (
            HASH_B,
            vec![BookMove {
                mv: g1f3(),
                weight: u32::MAX,
            }],
        ),
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
fn to_bytes_from_bytes_round_trips_an_empty_book() {
    let book = Book::new(vec![]);
    let bytes = book.to_bytes();
    let round_tripped = Book::from_bytes(&bytes).expect("an empty book must still load back");

    assert_eq!(
        round_tripped.moves(HASH_A),
        [],
        "an empty book must round-trip to an empty book"
    );
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
fn from_bytes_rejects_a_byte_stream_shorter_than_the_fingerprint_header() {
    assert_eq!(
        Book::from_bytes(&[0u8; 4]),
        Err(BookLoadError::Truncated),
        "4 bytes is shorter than the 8-byte fingerprint header alone"
    );
}

#[test]
fn from_bytes_rejects_a_fingerprint_that_does_not_match_this_build() {
    let book = Book::new(vec![(
        HASH_A,
        vec![BookMove {
            mv: e2e4(),
            weight: 1,
        }],
    )]);
    let mut bytes = book.to_bytes();

    // Flip every bit of the first 8 bytes (the documented fingerprint
    // header on `Book::to_bytes`): a byte can never equal its own bitwise
    // complement, so this is guaranteed to change the stored fingerprint
    // without needing to know its actual value.
    for byte in bytes.iter_mut().take(8) {
        *byte = !*byte;
    }

    assert_eq!(
        Book::from_bytes(&bytes),
        Err(BookLoadError::FingerprintMismatch),
        "a corrupted fingerprint must be rejected, not silently misread"
    );
}
