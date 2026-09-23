//! Resolving one SAN (Standard Algebraic Notation) move token against a
//! specific position.
//!
//! SAN is context-dependent (`Nf3` only means something once you know
//! which knight, if either, can legally reach `f3`), so resolution always
//! needs the actual position, not just the token text.

use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::{File, Move, MoveFlags, Piece, Rank, Square};

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
///
/// Works entirely in ASCII bytes, on the strength of the `is_ascii` check
/// up front: every real SAN token is pure ASCII, which is what licenses
/// slicing at fixed byte offsets below without ever risking a
/// char-boundary panic (the same trick `Move::from_uci` uses for the same
/// reason).
fn parse_san(token: &str) -> Option<SanMove> {
    if !token.is_ascii() {
        return None;
    }

    let token = token.strip_suffix(['+', '#']).unwrap_or(token);

    if token == "O-O" || token == "0-0" {
        return Some(SanMove::Castle(CastleSide::King));
    }
    if token == "O-O-O" || token == "0-0-0" {
        return Some(SanMove::Castle(CastleSide::Queen));
    }

    let bytes = token.as_bytes();
    let (token, promotion) = if bytes.len() >= 2 && bytes[bytes.len() - 2] == b'=' {
        let piece = match bytes[bytes.len() - 1] {
            b'N' => Piece::Knight,
            b'B' => Piece::Bishop,
            b'R' => Piece::Rook,
            b'Q' => Piece::Queen,
            _ => return None,
        };
        // `split_at_checked` rather than a slice: it returns None instead
        // of panicking when the index is not a character boundary, so the
        // parser rejects malformed input rather than aborting on it.
        (token.split_at_checked(token.len() - 2)?.0, Some(piece))
    } else {
        (token, None)
    };

    if token.len() < 2 {
        return None;
    }
    let (front, dest_str) = token.split_at(token.len() - 2);
    let to = Square::try_from_algebraic(dest_str)?;

    // Splitting on a character boundary rather than indexing: a SAN token
    // reaching here is not guaranteed to be ASCII, and a byte index into the
    // middle of a multi-byte character would panic rather than fail to parse.
    let (piece, front) = match front.split_at_checked(1) {
        Some(("N", rest)) => (Piece::Knight, rest),
        Some(("B", rest)) => (Piece::Bishop, rest),
        Some(("R", rest)) => (Piece::Rook, rest),
        Some(("Q", rest)) => (Piece::Queen, rest),
        Some(("K", rest)) => (Piece::King, rest),
        _ => (Piece::Pawn, front),
    };

    let front = front.strip_suffix('x').unwrap_or(front);
    if front.len() > 2 {
        return None;
    }

    let mut disambiguate_file = None;
    let mut disambiguate_rank = None;
    for &b in front.as_bytes() {
        if (b'a'..=b'h').contains(&b) {
            if disambiguate_file.is_some() {
                return None;
            }
            disambiguate_file = Some(File::from_u8(b - b'a')?);
        } else if (b'1'..=b'8').contains(&b) {
            if disambiguate_rank.is_some() {
                return None;
            }
            disambiguate_rank = Some(Rank::from_u8(b - b'1')?);
        } else {
            return None;
        }
    }

    Some(SanMove::Normal {
        piece,
        disambiguate_file,
        disambiguate_rank,
        to,
        promotion,
    })
}

/// Filters `board`'s legal moves down to the one `parsed` names: matching
/// piece (or the castling flag), destination square, promotion, and, if
/// present, the file/rank disambiguator. `None` if that's zero moves or
/// more than one.
fn resolve(board: &Board, parsed: SanMove) -> Option<Move> {
    let candidates = legal_moves(board);
    let mut matches = candidates
        .as_slice()
        .iter()
        .copied()
        .filter(|&mv| matches_parsed(board, mv, parsed));

    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

/// Whether `mv`, a legal move in whatever position it came from, is the
/// one `parsed` describes.
fn matches_parsed(board: &Board, mv: Move, parsed: SanMove) -> bool {
    match parsed {
        SanMove::Castle(CastleSide::King) => mv.flags() == MoveFlags::KingCastle,
        SanMove::Castle(CastleSide::Queen) => mv.flags() == MoveFlags::QueenCastle,
        SanMove::Normal {
            piece,
            disambiguate_file,
            disambiguate_rank,
            to,
            promotion,
        } => {
            let Some(moved) = board.piece_at(mv.from()) else {
                return false;
            };
            moved.piece() == piece
                && mv.to() == to
                && mv.flags().promotion_piece() == promotion
                && disambiguate_file.is_none_or(|f| mv.from().file() == f)
                && disambiguate_rank.is_none_or(|r| mv.from().rank() == r)
        }
    }
}
