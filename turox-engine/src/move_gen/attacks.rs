//! Square-attack queries: "is this square attacked, and by what?" Built on top
//! of `tables` (leaper attacks) and `magic` (slider attacks), both of which are
//! **forward**: given a piece standing on `sq` with some `occupied` set, what
//! does it hit. Everything in this module composes in that same forward
//! direction, with one deliberate exception (`attackers_of`) called out below.
//!
//! # Public surface
//!
//! - `fn piece_attacks(piece: Piece, color: Color, sq: Square, occupied: Bitboard) -> Bitboard` —
//!   the single place the `Piece` -> attack-function dispatch lives. `color`
//!   only matters for `Piece::Pawn`; every other piece attacks the same way
//!   regardless of color.
//! - `fn attacked_by(board: &Board, by: Color, occupied: Bitboard) -> Bitboard` —
//!   union of every square attacked by any piece of `by`, evaluated against
//!   `occupied` rather than `board.occupied()`. Taking `occupied` explicitly
//!   (rather than always using the board's own) matters for king safety: to ask
//!   "is this square safe for my king to step onto", the correct occupancy has
//!   the king already lifted off its old square, or a square directly behind the
//!   king through an enemy slider reads as falsely safe. Nothing in this PR
//!   exercises that case (`legal.rs`'s copy-make filter sidesteps it entirely by
//!   actually moving the king), but the parameter costs nothing and keeps the
//!   door open for a later staged/check-evasion generator that wants to mask
//!   king moves against a precomputed attack set instead of paying a full
//!   `make_move` per candidate.
//! - `fn is_attacked(board: &Board, sq: Square, by: Color) -> bool`
//! - `fn king_square(board: &Board, color: Color) -> Option<Square>` — `None` on
//!   a board with no king for that color (e.g. `Board::default()`, or many test
//!   FENs), not a panic.
//! - `fn in_check(board: &Board, color: Color) -> bool`
//! - `fn attackers_of(board: &Board, sq: Square, by: Color) -> Bitboard` — which
//!   pieces of `by` attack `sq`. **The one reverse-direction function**, and the
//!   one place a color bug can hide.
//!
//! # `attackers_of`: the superpiece trick, and its one trap
//!
//! The standard technique: stand each piece type on `sq` in turn, radiate its
//! attack pattern, and intersect with the real pieces of that type/color. Five
//! lookups total (knight, king, pawn, rook-or-queen, bishop-or-queen) — this is
//! also the shape a future SEE implementation wants, which is why it earns its
//! own function rather than being inlined into `is_attacked`.
//!
//! Knight, king, and slider attack relations are *symmetric* — "a attacks b"
//! iff "b attacks a" — so radiating from `sq` and intersecting with real pieces
//! works unmodified for those four lookups. Pawns are **not** symmetric: a
//! white pawn on d3 attacks c4/e4, but a pawn standing on c4 attacking as
//! *white* would radiate onto b5/d5, not d3. To find white pawns attacking
//! `sq`, radiate a *black* pawn from `sq` instead — `pawn_attacks(by.flip(), sq)`.
//! This is exactly the {Color}x{direction} shape flagged in `CLAUDE.md`: it
//! produces the right answer on any vertically symmetric test position even
//! with the flip missing or backward, so verify against an asymmetric one (a
//! single pawn a few ranks off the board's horizontal midline is enough).
//!
//! The proptest for this module checks `attackers_of` against the *forward*
//! definition directly (`{ s in board[by] : piece_attacks(piece_at(s), by, s,
//! occupied).contains(sq) }`), not against a second reverse implementation —
//! the forward form can't get the pawn flip wrong because it never inverts
//! anything, which is exactly what makes it a trustworthy check on the reverse
//! one.

use crate::board::Board;
use crate::move_gen::magic::{bishop_attacks, queen_attacks, rook_attacks};
use crate::move_gen::tables::{king_attacks, knight_attacks, pawn_attacks};
use crate::types::{Bitboard, Color, Piece, Square};

/// Every square a piece of `piece`/`color` standing on `sq` attacks, given
/// `occupied`. The single dispatch point from `Piece` to the right
/// `tables`/`magic` function; every other function in this module and in
/// `move_gen::pseudo_legal` should go through this rather than matching on
/// `Piece` itself.
pub fn piece_attacks(piece: Piece, color: Color, sq: Square, occupied: Bitboard) -> Bitboard {
    match piece {
        Piece::Pawn => pawn_attacks(color, sq),
        Piece::Knight => knight_attacks(sq),
        Piece::Bishop => bishop_attacks(sq, occupied),
        Piece::Rook => rook_attacks(sq, occupied),
        Piece::Queen => queen_attacks(sq, occupied),
        Piece::King => king_attacks(sq),
    }
}

/// Union of every square attacked by any piece of `by`, against `occupied`
/// (not necessarily `board.occupied()` — see the module doc).
pub fn attacked_by(board: &Board, by: Color, occupied: Bitboard) -> Bitboard {
    let mut attacked_by = Bitboard::EMPTY;
    for piece in Piece::ALL {
        let pieces = board.pieces(by, piece);
        for sq in pieces {
            attacked_by = attacked_by.or(piece_attacks(piece, by, sq, occupied));
        }
    }
    attacked_by
}

/// Whether any piece of `by` attacks `sq`, given the board's own occupancy.
pub fn is_attacked(board: &Board, sq: Square, by: Color) -> bool {
    !attacked_by(board, by, board.occupied())
        .and(sq.bitboard())
        .is_empty()
}

/// `color`'s king's square, or `None` if it has no king.
pub fn king_square(board: &Board, color: Color) -> Option<Square> {
    board.pieces(color, Piece::King).lsb()
}

/// Whether `color`'s king is currently attacked. `false` if `color` has no
/// king, rather than panicking.
pub fn in_check(board: &Board, color: Color) -> bool {
    if let Some(sq) = king_square(board, color) {
        is_attacked(board, sq, color.flip())
    } else {
        false
    }
}

/// Which pieces of `by` attack `sq` — the superpiece trick. See the module doc
/// for the pawn-direction trap this function is the one place in the crate
/// where it matters twice (the other is en passant source lookup in
/// `pseudo_legal`).
pub fn attackers_of(board: &Board, sq: Square, by: Color) -> Bitboard {
    let mut attackers_of = Bitboard::EMPTY;
    for piece in Piece::ALL {
        let attack_radiation = if piece == Piece::Pawn {
            pawn_attacks(by.flip(), sq)
        } else {
            piece_attacks(piece, by, sq, board.occupied())
        };
        attackers_of = attackers_of.or(attack_radiation.and(board.pieces(by, piece)));
    }
    attackers_of
}
