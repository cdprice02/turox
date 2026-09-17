//! Resolving one SAN (Standard Algebraic Notation) move token against a
//! specific position.
//!
//! SAN is context-dependent (`Nf3` only means something once you know
//! which knight, if either, can legally reach `f3`), so resolution always
//! needs the actual position, not just the token text.

use turox_engine::board::Board;
use turox_engine::{File, Move, Piece, Rank, Square};

/// A SAN token's parsed shape, before it's checked against any position.
///
/// Deliberately carries no source square: SAN text itself only ever gives
/// a *disambiguator* (an optional file and/or rank), never a guaranteed
/// full square (`"Nbd7"` says a knight from the b-file, not which rank;
/// `"exd5"` says a pawn from the e-file, not which one). Recovering the
/// real source square needs the position's own legal moves, which is
/// [`resolve`]'s job, not [`parse_san`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SanMove {
    /// `O-O` or `O-O-O`. No piece, no disambiguator, no destination
    /// square, no promotion: none of `Normal`'s fields apply to a
    /// castling move, which is why this is a separate variant rather than
    /// an `Option<CastleSide>` field alongside them.
    Castle(CastleSide),
    /// Every other move shape.
    Normal {
        /// The moving piece; `Piece::Pawn` when no piece letter is given.
        piece: Piece,
        /// The disambiguating source file, if the token gave one.
        disambiguate_file: Option<File>,
        /// The disambiguating source rank, if the token gave one.
        disambiguate_rank: Option<Rank>,
        /// The destination square.
        to: Square,
        /// The promotion piece, if the token gave one (always one of
        /// knight, bishop, rook, or queen).
        promotion: Option<Piece>,
    },
}

/// Which side [`SanMove::Castle`] castles toward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CastleSide {
    /// `O-O`.
    King,
    /// `O-O-O`.
    Queen,
}

/// Resolves `token` (e.g. `"Nf3"`, `"exd5"`, `"O-O"`, `"e8=Q+"`) against the
/// legal moves available in `board`, returning the one `Move` it names.
///
/// `None` for a token that doesn't parse as SAN at all, or that resolves to
/// zero or more than one legal move (ambiguous notation is a real, if rare,
/// possibility in hand-annotated or buggy PGN, not just malformed input;
/// either way there's no single answer to return).
#[must_use]
pub fn resolve_san(board: &Board, token: &str) -> Option<Move> {
    let parsed = parse_san(token)?;
    resolve(board, parsed)
}

/// Parses `token`'s text alone, with no board context at all: see
/// [`SanMove`]'s own doc for why that's as far as text-only parsing can
/// ever go. `None` for a token that isn't well-formed SAN.
fn parse_san(_token: &str) -> Option<SanMove> {
    None
}

/// Filters `board`'s legal moves down to the one `parsed` names: matching
/// piece (or the castling flag), destination square, promotion, and, if
/// present, the file/rank disambiguator. `None` if that's zero moves or
/// more than one.
fn resolve(_board: &Board, _parsed: SanMove) -> Option<Move> {
    None
}
