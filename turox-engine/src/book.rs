//! An opening book: a precomputed table, keyed on `board::zobrist`'s hash.
//!
//! Maps a position to one or more known-good moves. Built offline by a
//! generator and shipped as a checked-in binary asset; this module is the
//! engine-side format and lookup half only, not the generator itself.
//!
//! Consulted by `uci::session` before a `go` reaches `search::Search`, not
//! from inside `Search` itself: a book hit is a bypass of search, not an
//! input to it.
//!
//! # Interface only, not yet implemented
//!
//! Every method here is a stub returning a fixed, honest "empty" answer
//! rather than `todo!()`/`unimplemented!()`, both denied by this workspace's
//! lint policy. The real logic (the wire format, the weighted-random
//! selection algorithm) belongs to whoever picks up this module; the tests
//! in `tests/book.rs` and `tests/book_props.rs` are written against this
//! exact interface and are expected to fail until it's filled in.

use crate::types::Move;

/// One followable move for a book position, alongside its weight.
///
/// Weight is only meaningful relative to the other candidates at the same
/// position, coming from the generator's source data (frequency and win
/// rate); higher weight means more likely to be chosen, never a hard
/// ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookMove {
    /// The move itself.
    pub mv: Move,
    /// This move's weight among its position's other candidates. Only
    /// meaningful relative to the other weights at the same position, not
    /// on its own.
    pub weight: u32,
}

/// Why [`Book::from_bytes`] rejected a byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookLoadError {
    /// The byte stream is too short to hold even the fingerprint header, or
    /// otherwise isn't shaped like a book file at all.
    Truncated,
    /// The book's stored fingerprint doesn't match this build's
    /// `board::zobrist` key table, so every hash inside it would resolve
    /// against the wrong table's keys.
    FingerprintMismatch,
}

/// A loaded opening book: a lookup from a position's Zobrist hash to its
/// candidate moves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Book {
    /// One entry per book position: its hash and the moves known for it.
    /// Order carries no meaning; lookups go by hash, not position.
    entries: Vec<(u64, Vec<BookMove>)>,
}

impl Book {
    /// Builds a book directly from `entries`, without going through
    /// [`Book::to_bytes`]/[`Book::from_bytes`]. The path a generator, or a
    /// test, uses to construct one in memory.
    #[must_use]
    pub const fn new(entries: Vec<(u64, Vec<BookMove>)>) -> Self {
        Self { entries }
    }

    /// The candidate moves recorded for `hash`, or an empty slice if `hash`
    /// isn't in the book (an out-of-book position).
    #[must_use]
    pub const fn moves(&self, _hash: u64) -> &[BookMove] {
        &[]
    }

    /// A weighted-random choice among `hash`'s candidate moves, seeded for
    /// reproducibility. `None` for an out-of-book position, matching
    /// [`Book::moves`] returning empty.
    #[must_use]
    pub const fn choose(&self, _hash: u64, _seed: u64) -> Option<Move> {
        None
    }

    /// Serializes this book to bytes. The first 8 bytes are a
    /// little-endian fingerprint of this build's `board::zobrist` key
    /// table, which [`Book::from_bytes`] checks; everything after that is
    /// implementation-defined.
    #[must_use]
    pub const fn to_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Deserializes a book previously written by [`Book::to_bytes`].
    ///
    /// # Errors
    ///
    /// [`BookLoadError::Truncated`] if `bytes` is too short to hold the
    /// fingerprint header. [`BookLoadError::FingerprintMismatch`] if
    /// `bytes` was written by a build with a different `board::zobrist` key
    /// table.
    pub const fn from_bytes(_bytes: &[u8]) -> Result<Self, BookLoadError> {
        Err(BookLoadError::Truncated)
    }
}
