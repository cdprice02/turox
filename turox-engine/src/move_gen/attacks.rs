//! Square-attack queries: "is this square attacked, and by what?" Built on top
//! of `tables` (leaper attacks) and `magic` (slider attacks), both **forward**:
//! given a piece standing on `sq` with some `occupied` set, what does it hit.
//! Everything here composes in that same forward direction, with one
//! deliberate exception (`attackers_of`) below.

use crate::board::Board;
use crate::move_gen::magic::{bishop_attacks, queen_attacks, rook_attacks};
use crate::move_gen::tables::{king_attacks, knight_attacks, pawn_attacks};
use crate::types::{Bitboard, Color, Piece, Square};

/// Every square a piece of `piece`/`color` standing on `sq` attacks, given
/// `occupied`. The single dispatch point from `Piece` to the right
/// `tables`/`magic` function; every other function in this module and in
/// `move_gen::pseudo_legal` should go through this rather than matching on
/// `Piece` itself.
pub const fn piece_attacks(piece: Piece, color: Color, sq: Square, occupied: Bitboard) -> Bitboard {
    match piece {
        Piece::Pawn => pawn_attacks(color, sq),
        Piece::Knight => knight_attacks(sq),
        Piece::Bishop => bishop_attacks(sq, occupied),
        Piece::Rook => rook_attacks(sq, occupied),
        Piece::Queen => queen_attacks(sq, occupied),
        Piece::King => king_attacks(sq),
    }
}

/// Union of every square attacked by any piece of `by`, against `occupied`.
/// Takes `occupied` explicitly rather than always using `board.occupied()`:
/// for king safety, the correct occupancy has the king already lifted off
/// its old square, or a square directly behind it through an enemy slider
/// reads as falsely safe.
///
/// Not `const`: `for piece in Piece::ALL` and `for sq in pieces` both go
/// through `IntoIterator`, which isn't const-callable. Rewriting either loop
/// as an index/`lsb` walk would buy a `const` no caller currently needs, at
/// a real readability cost, so this stays a plain `fn`. Same reasoning
/// applies to `attackers_of` below.
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
pub const fn king_square(board: &Board, color: Color) -> Option<Square> {
    board.pieces(color, Piece::King).lsb()
}

/// Whether `color`'s king is currently attacked. `false` if `color` has no
/// king, rather than panicking.
///
/// Goes through `attackers_of` rather than `is_attacked`/`attacked_by`: this
/// is the hottest line in `legal_moves` (once per pseudolegal move, via
/// `make_move` + `in_check`), and `attackers_of` answers "is this one square
/// attacked" in five lookups where `attacked_by` builds the enemy's entire
/// attack set (all six piece types over every enemy piece) just to test one
/// bit of it.
pub fn in_check(board: &Board, color: Color) -> bool {
    if let Some(sq) = king_square(board, color) {
        !attackers_of(board, sq, color.flip()).is_empty()
    } else {
        false
    }
}

/// Which pieces of `by` attack `sq`, the superpiece trick: stand each piece
/// type on `sq` in turn, radiate its attack pattern, and intersect with the
/// real pieces of that type/color. The reverse of every other function in
/// this module, and the one place a color bug can hide.
///
/// Knight, king, and slider attack relations are *symmetric*: "a attacks b"
/// iff "b attacks a", so radiating from `sq` works unmodified for those
/// four. Pawns are **not**: a white pawn on d3 attacks c4/e4, but a pawn
/// standing on c4 attacking as *white* would radiate onto b5/d5, not d3. To
/// find white pawns attacking `sq`, radiate a *black* pawn from `sq`
/// instead: `pawn_attacks(by.flip(), sq)`. This is a {Color}x{direction}
/// shape, the same kind that has repeatedly produced scrambled bugs in this
/// crate, and the one other place it matters is en passant source lookup in
/// `pseudo_legal`. It produces the right answer on any
/// vertically symmetric test position even with the flip missing or
/// backward, so verify against an asymmetric one (a pawn a few ranks off the
/// board's horizontal midline is enough).
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
