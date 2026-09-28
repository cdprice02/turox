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
//! many `(2-byte move bits, 4-byte weight)` pairs. After the entries, a
//! 4-byte count of distinct names and that many names, each a 2-byte length
//! followed by that many UTF-8 bytes, then a 4-byte count of named positions
//! and that many `(8-byte hash, 2-byte name index)` pairs.
//!
//! Names are interned because the same one covers a whole line: a book of
//! four hundred thousand named positions draws on a few thousand distinct
//! names, so storing each position's own copy would cost several times what
//! the moves do.
//!
//! Names sit in their own table, keyed by position, rather than on each move.
//! A name describes a position reached, so keying it that way is what lets a
//! transposition into a named line find its name rather than only the one
//! move that happened to record it. It is also much smaller: a per-move name
//! costs its length field on every move whether or not one exists.

use crate::board::zobrist;
use crate::types::Move;
use std::collections::BTreeMap;
use turox_rng::xorshift64star;

/// This crate's own book format version, bumped whenever [`Book::to_bytes`]'s
/// byte layout changes shape (a field added, removed, resized, or
/// reordered). Distinct from `zobrist::FINGERPRINT`: that identifies which
/// *key table* a stored hash was computed against, not which *layout* the
/// bytes around it are in, and the two can change independently of each
/// other.
const FORMAT_VERSION: u8 = 3;

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

/// Reads a length-prefixed name at `*pos`, advancing past it: a 2-byte
/// length, then that many UTF-8 bytes.
///
/// Running out of bytes and declared-but-invalid-UTF-8 bytes are both
/// [`BookLoadError::Truncated`], the same "malformed either way" treatment
/// that error's own doc already gives a name whose length runs past the
/// stream.
fn read_name(bytes: &[u8], pos: &mut usize) -> Result<String, BookLoadError> {
    let len = read_u16(bytes, pos).ok_or(BookLoadError::Truncated)?;
    let chunk = bytes
        .get(*pos..*pos + usize::from(len))
        .ok_or(BookLoadError::Truncated)?;
    *pos += usize::from(len);
    String::from_utf8(chunk.to_vec()).map_err(|_| BookLoadError::Truncated)
}

/// Writes `name` in [`read_name`]'s format: a 2-byte length, then that many
/// UTF-8 bytes. Truncated rather than refused past `u16::MAX`, since no
/// opening name comes near it and a whole book is not worth failing to write
/// over one.
fn write_name(buf: &mut Vec<u8>, name: &str) {
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
}

impl BookMove {
    /// A move and the weight it carries among its position's candidates.
    #[must_use]
    pub const fn new(mv: Move, weight: u32) -> Self {
        Self { mv, weight }
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
    /// The distinct opening names, indexed by `named_positions`.
    names: Vec<String>,
    /// Which name each named position carries, sorted by hash so a lookup is
    /// a binary search.
    named_positions: Vec<(u64, u16)>,
}

impl Book {
    /// Builds a book directly from `entries`, without going through
    /// [`Book::to_bytes`]/[`Book::from_bytes`]. The path a generator, or a
    /// test, uses to construct one in memory.
    #[must_use]
    pub const fn new(entries: Vec<(u64, Vec<BookMove>)>) -> Self {
        Self {
            entries,
            names: Vec::new(),
            named_positions: Vec::new(),
        }
    }

    /// [`Book::new`], with an opening name per position.
    ///
    /// Sorted here rather than trusted to arrive sorted, since
    /// [`Book::name`]'s binary search answers wrongly rather than slowly
    /// against a table that is not.
    #[must_use]
    pub fn with_names(
        entries: Vec<(u64, Vec<BookMove>)>,
        per_position: Vec<(u64, String)>,
    ) -> Self {
        let mut names: Vec<String> = Vec::new();
        let mut index_of: BTreeMap<String, u16> = BTreeMap::new();
        let mut named_positions = Vec::with_capacity(per_position.len());

        for (hash, name) in per_position {
            let next = u16::try_from(names.len()).unwrap_or(u16::MAX);
            let index = *index_of.entry(name.clone()).or_insert_with(|| {
                names.push(name);
                next
            });
            named_positions.push((hash, index));
        }

        named_positions.sort_unstable_by_key(|(hash, _)| *hash);
        Self {
            entries,
            names,
            named_positions,
        }
    }

    /// The opening name recorded for `hash`, or `None` for a position no
    /// named line reaches.
    #[must_use]
    pub fn name(&self, hash: u64) -> Option<&str> {
        let at = self
            .named_positions
            .binary_search_by_key(&hash, |(hash, _)| *hash)
            .ok()?;
        let (_, index) = self.named_positions[at];
        self.names.get(usize::from(index)).map(String::as_str)
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

    /// Every position in the book with its moves.
    ///
    /// For a generator walking the whole book, which [`Book::moves`] cannot
    /// serve: that scans linearly, so calling it per position would be
    /// quadratic in the book's size.
    pub fn positions(&self) -> impl Iterator<Item = (u64, &[BookMove])> {
        self.entries
            .iter()
            .map(|(hash, mvs)| (*hash, mvs.as_slice()))
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
            }
        }

        let name_count = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&name_count.to_le_bytes());
        for name in &self.names {
            write_name(&mut buf, name);
        }

        let position_count = u32::try_from(self.named_positions.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&position_count.to_le_bytes());
        for (hash, index) in &self.named_positions {
            buf.extend_from_slice(&hash.to_le_bytes());
            buf.extend_from_slice(&index.to_le_bytes());
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
                moves.push(BookMove {
                    mv: Move::from_bits(bits),
                    weight,
                });
            }

            entries.push((hash, moves));
        }

        // Neither table is pre-allocated from its count: both come from the
        // file, and a corrupt one would reserve on a promise the bytes need
        // not keep.
        let name_count = read_u32(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
        let mut names = Vec::new();
        for _ in 0..name_count {
            names.push(read_name(bytes, &mut pos)?);
        }

        let position_count = read_u32(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
        let mut named_positions = Vec::new();
        for _ in 0..position_count {
            let hash = read_u64(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
            let index = read_u16(bytes, &mut pos).ok_or(BookLoadError::Truncated)?;
            named_positions.push((hash, index));
        }

        // Sorted on the way in as well as in `with_names`: bytes from disk
        // have not been through that constructor, and `name`'s binary search
        // answers wrongly rather than slowly against an unsorted table.
        named_positions.sort_unstable_by_key(|(hash, _)| *hash);

        Ok(Self {
            entries,
            names,
            named_positions,
        })
    }
}
