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
//! `Book::to_bytes`'s output, all little-endian: a 1-byte format version
//! (`from_bytes` checks this first, ahead of everything else, since an
//! unrecognized version makes every byte after it meaningless), an 8-byte
//! fingerprint of the build's `board::zobrist` key table, a 4-byte entry
//! count, then for each entry an 8-byte hash, a 4-byte move count, and that
//! many `(2-byte move bits, 4-byte weight, name)` triples, where `name` is a
//! 2-byte length followed by that many UTF-8 bytes (length `0` means no
//! name).

use crate::board::zobrist;
use crate::rng::xorshift64star;
use crate::types::Move;

/// This crate's own book format version, bumped whenever [`Book::to_bytes`]'s
/// byte layout changes shape (a field added, removed, resized, or
/// reordered). Distinct from `zobrist::FINGERPRINT`: that identifies which
/// *key table* a stored hash was computed against, not which *layout* the
/// bytes around it are in, and the two can change independently of each
/// other.
const FORMAT_VERSION: u8 = 2;

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
/// [`BookLoadError::UnsupportedVersion`] if `opening.bin` predates this
/// build's format version. [`BookLoadError::Truncated`] should never happen
/// for the embedded bytes specifically (they're checked in as a known-good
/// `Book`), but `from_bytes` doesn't get to assume that about its input just
/// because this caller happens to know better.
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

/// Reads a `u8` at `*pos`, advancing past it.
fn read_u8(bytes: &[u8], pos: &mut usize) -> Option<u8> {
    read_bytes(bytes, pos).map(u8::from_le_bytes)
}

/// Reads a length-prefixed, possibly-empty name at `*pos`, advancing past
/// it: a 2-byte length, then that many UTF-8 bytes. Length `0` reads as
/// `Ok(None)`, matching [`write_name`]'s own convention, so an absent name
/// costs exactly the 2-byte length field and nothing more.
///
/// Returns `Result`, not `Option<Option<String>>` (a `clippy::option_option`
/// lint denial, and confusing regardless): running out of bytes and
/// declared-but-invalid-UTF-8 bytes are both [`BookLoadError::Truncated`],
/// the same "malformed either way" treatment [`BookLoadError::Truncated`]'s
/// own doc already gives a name whose length runs past the stream.
fn read_name(bytes: &[u8], pos: &mut usize) -> Result<Option<String>, BookLoadError> {
    let len = read_u16(bytes, pos).ok_or(BookLoadError::Truncated)?;
    if len == 0 {
        return Ok(None);
    }
    let chunk = bytes
        .get(*pos..*pos + usize::from(len))
        .ok_or(BookLoadError::Truncated)?;
    *pos += usize::from(len);
    String::from_utf8(chunk.to_vec())
        .map(Some)
        .map_err(|_| BookLoadError::Truncated)
}

/// Writes `name` in [`read_name`]'s format: a 2-byte length, then that many
/// UTF-8 bytes, or just a `0` length for `None`.
fn write_name(buf: &mut Vec<u8>, name: Option<&str>) {
    let Some(name) = name else {
        buf.extend_from_slice(&0u16.to_le_bytes());
        return;
    };
    let len = u16::try_from(name.len()).unwrap_or(u16::MAX);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&name.as_bytes()[..usize::from(len)]);
}

/// One followable move for a book position, alongside its weight.
///
/// Weight is only meaningful relative to the other candidates at the same
/// position, coming from the generator's source data (frequency and win
/// rate); higher weight means more likely to be chosen, never a hard
/// ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookMove {
    /// The move itself.
    pub mv: Move,
    /// This move's weight among its position's other candidates. Only
    /// meaningful relative to the other weights at the same position, not
    /// on its own.
    pub weight: u32,
    /// The opening or variation name this move belongs to, if the book's
    /// source data carried one (e.g. "Ruy Lopez: Berlin Defense" or an ECO
    /// code). Per move, not per position: two replies from the same
    /// position can genuinely be different named openings, so a name one
    /// level up would be a false merge, not a simplification.
    pub name: Option<String>,
}

impl BookMove {
    /// A `BookMove` with no name, the common case: most callers (every
    /// test in this crate, and any move a real generator run didn't manage
    /// to attribute to a named opening) have no use for one.
    #[must_use]
    pub const fn new(mv: Move, weight: u32) -> Self {
        Self {
            mv,
            weight,
            name: None,
        }
    }

    /// A `BookMove` carrying `name`, for a generator that resolved one.
    #[must_use]
    pub const fn with_name(mv: Move, weight: u32, name: String) -> Self {
        Self {
            mv,
            weight,
            name: Some(name),
        }
    }
}

/// Why [`Book::from_bytes`] rejected a byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookLoadError {
    /// The byte stream is too short to hold even the version and
    /// fingerprint header, or otherwise isn't shaped like a book file at
    /// all (including a name whose declared length runs past the end of
    /// the stream, or whose bytes aren't valid UTF-8: both are exactly as
    /// unreadable as running out of bytes outright).
    Truncated,
    /// The book's stored format version doesn't match this build's
    /// `FORMAT_VERSION`, so the byte layout after it can't be assumed to
    /// mean what this build thinks it means.
    UnsupportedVersion,
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
        self.choose_move(hash, seed).map(|bm| bm.mv)
    }

    /// [`Book::choose`], but returns the whole chosen entry (its weight and
    /// name too) rather than just the move: what a caller reporting *why*
    /// a book hit happened, not merely which move it picked, needs.
    #[must_use]
    pub fn choose_move(&self, hash: u64, seed: u64) -> Option<&BookMove> {
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
                return Some(&moves[i]);
            }
        }

        None
    }

    /// Serializes this book to bytes. See the module doc for the exact wire
    /// format; the first byte is always `FORMAT_VERSION`, which
    /// [`Book::from_bytes`] checks before reading anything else.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(FORMAT_VERSION);
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
                write_name(&mut buf, bm.name.as_deref());
            }
        }

        buf
    }

    /// Deserializes a book previously written by [`Book::to_bytes`].
    ///
    /// # Errors
    ///
    /// [`BookLoadError::Truncated`] if `bytes` is too short to hold the
    /// version and fingerprint header (or is truncated anywhere later on).
    /// [`BookLoadError::UnsupportedVersion`] if `bytes` was written by a
    /// different format version. [`BookLoadError::FingerprintMismatch`] if
    /// `bytes` was written by a build with a different `board::zobrist` key
    /// table.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BookLoadError> {
        let mut pos = 0;

        let version = read_u8(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
        if version != FORMAT_VERSION {
            return Err(BookLoadError::UnsupportedVersion);
        }

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
                let name = read_name(bytes, &mut pos)?;
                moves.push(BookMove {
                    mv: Move::from_bits(bits),
                    weight,
                    name,
                });
            }

            entries.push((hash, moves));
        }

        Ok(Self { entries })
    }
}
