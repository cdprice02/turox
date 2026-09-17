//! Turns raw PGN game data into a `turox_engine::book::Book`, the offline
//! half of the opening book: a generator run, not part of the engine
//! itself, and not held to `turox-engine`'s zero-runtime-dependency policy
//! (see `Cargo.toml`).
//!
//! `pgn` parses PGN text into games, `san` resolves one SAN move token
//! against a position, `aggregate` walks a set of games into per-position
//! move statistics (respecting a rating floor and a ply cap), and `weight`
//! turns those statistics into the weights `book::BookMove` carries.
//! [`build_book`] is all four wired together, the function `main` calls.

pub mod aggregate;
pub mod pgn;
pub mod san;
pub mod weight;

use aggregate::BuildOptions;
use turox_engine::book::Book;

/// Runs the full pipeline: parses `pgn_text`, aggregates it into
/// per-position move statistics under `options`, and weighs the result
/// into a `Book`.
#[must_use]
pub fn build_book(pgn_text: &str, options: &BuildOptions) -> Book {
    let games = pgn::parse_pgn(pgn_text);
    let raw = aggregate::aggregate(&games, options);
    let kept = aggregate::filter_by_density(raw, options.min_sample_size);
    let entries = kept
        .into_iter()
        .map(|(hash, stats)| (hash, weight::weigh(&stats)))
        .collect();
    Book::new(entries)
}
