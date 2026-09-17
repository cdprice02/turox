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
//! # The embedded default book
//!
//! `opening.bin` is `tools/bookgen`'s output from the Lichess Elite
//! Database (CC0-licensed; see ADR 0006 for why this crate builds its own
//! format rather than reading Polyglot), covering December 2024 through
//! November 2025 (twelve monthly snapshots, ~3.4 million games), filtered
//! to both players at 2000+ and a position reached by at least 10
//! qualifying games, up to 20 plies (10 moves per side) deep. It's
//! embedded at compile time via [`default_book`] rather than read from an
//! external file at startup, matching `turox-engine`'s existing
//! zero-runtime-dependency policy: the book ships inside the binary
//! itself.
//!
//! # Wire format
//!
//! `Book::to_bytes`'s output: an 8-byte little-endian fingerprint of the
//! build's `board::zobrist` key table (`from_bytes` checks this first, ahead
//! of parsing anything else), a 4-byte little-endian entry count, then for
//! each entry an 8-byte hash, a 4-byte move count, and that many
//! `(2-byte move bits, 4-byte weight)` pairs, all little-endian.

use crate::board::zobrist;
use crate::rng::xorshift64star;
use crate::types::Move;

/// The generated book file itself: see the module doc's "The embedded
/// default book" section for exactly what source data and settings
/// produced it.
const OPENING_BOOK_BYTES: &[u8] = include_bytes!("opening.bin");

/// Decodes the book embedded in this binary.
///
/// A build's own compile-time data, not external input, so a real caller
/// can reasonably treat a mismatch here as a build inconsistency rather
/// than something to recover from at runtime; this still returns a
/// `Result` rather than panicking, since deciding how to react to that
/// (fall back to no book, abort the build, ...) is a policy choice for
/// the caller, not this function.
///
/// # Errors
///
/// [`BookLoadError::FingerprintMismatch`] if this build's `board::zobrist`
/// key table doesn't match the one `opening.bin` was generated against.
/// [`BookLoadError::Truncated`] should never happen for the embedded
/// bytes specifically (they're checked in as a known-good `Book`), but
/// `from_bytes` doesn't get to assume that about its input just because
/// this caller happens to know better.
pub fn default_book() -> Result<Book, BookLoadError> {
    Book::from_bytes(OPENING_BOOK_BYTES)
}

/// Reads `N` bytes at `*pos` and advances `*pos` by `N`, or `None` if fewer
/// than `N` bytes remain. The one bounds check every other `read_*` helper
/// here builds on, so truncation is caught in exactly one place.
fn read_bytes<const N: usize>(bytes: &[u8], pos: &mut usize) -> Option<[u8; N]> {
    let chunk = bytes.get(*pos..*pos + N)?.try_into().ok()?;
    *pos += N;
    Some(chunk)
}

/// Reads a little-endian `u64` at `*pos`, advancing past it.
fn read_u64(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    read_bytes(bytes, pos).map(u64::from_le_bytes)
}

/// Reads a little-endian `u32` at `*pos`, advancing past it.
fn read_u32(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    read_bytes(bytes, pos).map(u32::from_le_bytes)
}

/// Reads a little-endian `u16` at `*pos`, advancing past it.
fn read_u16(bytes: &[u8], pos: &mut usize) -> Option<u16> {
    read_bytes(bytes, pos).map(u16::from_le_bytes)
}

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
    pub fn moves(&self, hash: u64) -> &[BookMove] {
        self.entries
            .iter()
            .find(|entry| entry.0 == hash)
            .map_or(&[], |bm| bm.1.as_slice())
    }

    /// A weighted-random choice among `hash`'s candidate moves, seeded for
    /// reproducibility. `None` for an out-of-book position, matching
    /// [`Book::moves`] returning empty.
    #[must_use]
    pub fn choose(&self, hash: u64, seed: u64) -> Option<Move> {
        let moves = self.moves(hash);
        if moves.is_empty() {
            return None;
        }

        let weights = moves.iter().map(|bm| u64::from(bm.weight));
        let weight_total: u64 = weights.clone().sum();

        let chosen = xorshift64star(seed) % weight_total;

        let mut sum = 0;
        for (i, b) in weights.enumerate() {
            sum += b;
            if sum > chosen {
                return Some(moves[i].mv);
            }
        }

        None
    }

    /// Serializes this book to bytes. See the module doc for the exact wire
    /// format; the first 8 bytes are always this build's `board::zobrist`
    /// fingerprint, which [`Book::from_bytes`] checks before reading
    /// anything else.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&zobrist::FINGERPRINT.to_le_bytes());

        let entry_count = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&entry_count.to_le_bytes());

        for (hash, moves) in &self.entries {
            buf.extend_from_slice(&hash.to_le_bytes());

            let move_count = u32::try_from(moves.len()).unwrap_or(u32::MAX);
            buf.extend_from_slice(&move_count.to_le_bytes());

            for bm in moves {
                buf.extend_from_slice(&bm.mv.bits().to_le_bytes());
                buf.extend_from_slice(&bm.weight.to_le_bytes());
            }
        }

        buf
    }

    /// Deserializes a book previously written by [`Book::to_bytes`].
    ///
    /// # Errors
    ///
    /// [`BookLoadError::Truncated`] if `bytes` is too short to hold the
    /// fingerprint header. [`BookLoadError::FingerprintMismatch`] if
    /// `bytes` was written by a build with a different `board::zobrist` key
    /// table.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BookLoadError> {
        let mut pos = 0;

        let fingerprint = read_u64(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
        if fingerprint != zobrist::FINGERPRINT {
            return Err(BookLoadError::FingerprintMismatch);
        }

        let entry_count = read_u32(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
        let mut entries = Vec::new();
        for _ in 0..entry_count {
            let hash = read_u64(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;

            let move_count = read_u32(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
            let mut moves = Vec::new();
            for _ in 0..move_count {
                let bits = read_u16(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
                let weight = read_u32(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
                moves.push(BookMove {
                    mv: Move::from_bits(bits),
                    weight,
                });
            }

            entries.push((hash, moves));
        }

        Ok(Self { entries })
    }
}
