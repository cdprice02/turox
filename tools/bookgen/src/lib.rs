//! Turns raw PGN game data into a `turox_chess::book::Book`, the offline
//! half of the opening book: a generator run, not part of the engine
//! itself, and not held to `turox-engine`'s zero-runtime-dependency policy
//! (see `Cargo.toml`).
//!
//! `pgn` parses PGN text into games (or, via `turox_notation::pgn::PgnReader`, streams
//! them one at a time from a reader), `san` resolves one SAN move token
//! against a position, `aggregate` walks a set of games into per-position
//! move statistics (respecting a rating floor and a ply cap), and `weight`
//! turns those statistics into the weights `book::BookMove` carries.
//! [`build_book_from_games`] is the last three wired together; [`build_book`]
//! adds `turox_notation::pgn::parse_pgn` in front of it for a small in-memory source.

pub mod aggregate;
pub mod weight;

use aggregate::BuildOptions;
use turox_chess::book::Book;
use turox_notation::pgn::PgnGame;

/// Runs the aggregate-then-weigh half of the pipeline against `games`.
///
/// Already-parsed or streamed in one at a time (see `turox_notation::pgn::PgnReader`):
/// whichever the caller has, this doesn't care, since it only ever
/// consumes the iterator it's given.
#[must_use]
pub fn build_book_from_games(
    games: impl IntoIterator<Item = PgnGame>,
    options: &BuildOptions,
) -> Book {
    let raw = aggregate::aggregate(games, options);
    let kept = aggregate::filter_by_density(raw, options.min_sample_size);
    let entries = kept
        .into_iter()
        .map(|(hash, stats)| (hash, weight::weigh(&stats)))
        .collect();
    Book::new(entries)
}

/// [`build_book_from_games`], parsing `pgn_text` first.
///
/// Holds all of `pgn_text` and every game it contains in memory at once (see
/// `turox_notation::pgn::parse_pgn`'s own doc), which is fine for a small synthetic
/// fixture or a test but not a real source file; a real generator run
/// should build a `turox_notation::pgn::PgnReader` per input and call
/// [`build_book_from_games`] directly instead.
#[must_use]
pub fn build_book(pgn_text: &str, options: &BuildOptions) -> Book {
    build_book_from_games(turox_notation::pgn::parse_pgn(pgn_text), options)
}
