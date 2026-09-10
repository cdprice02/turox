//! Square-attack queries: "is this square attacked, and by what?"
//!
//! Built on top of `tables` (leaper attacks) and `magic` (slider attacks), both
//! **forward**: given a piece standing on `sq` with some `occupied` set, what does it
//! hit. Everything here composes in that same forward direction, with one deliberate
//! exception (`attackers_of`) below.

use crate::board::Board;
use crate::move_gen::magic::{bishop_attacks, queen_attacks, rook_attacks};
use crate::move_gen::tables::{between, king_attacks, knight_attacks, pawn_attacks};
use crate::types::{Bitboard, Color, Piece, Square};

/// Every square a piece of `piece`/`color` standing on `sq` attacks, given `occupied`.
///
/// The single dispatch point from `Piece` to the right `tables`/`magic` function; every
/// other function in this module and in `move_gen::pseudo_legal` should go through this
/// rather than matching on `Piece` itself.
#[must_use]
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

/// The "second layer" of `sq`'s rook-ray attacks: whatever sits directly behind the
/// nearest blocker in `blockers`, as if that blocker weren't there.
///
/// The technique is [Checks and Pinned Pieces
/// (Bitboards)](https://www.chessprogramming.org/Checks_and_Pinned_Pieces_(Bitboards)).
/// For pin detection, `sq` is the king's own square, `occupied` is the whole board, and
/// `blockers` is the mover's own pieces: the first pass finds what the king would see
/// as a rook, `blockers` picks out which of *those* hits are the mover's own pieces
/// (not the opponent's, which already block for real and need no second look), and the
/// second pass reveals whatever sits directly behind each such piece. Intersecting the
/// result with the opponent's rooks/queens gives the actual pinners; `tables::between`
/// between the king and each pinner then gives the pinned piece itself.
///
/// **Gotcha**: `blockers` is not `occupied`. Passing the full occupancy as `blockers`
/// removes *everything* the first pass hit (including the opponent's own blocking
/// piece), which reveals a square nothing is actually pinned against and produces a
/// pinned set with false positives. `blockers` must be exactly the subset worth
/// looking behind.
#[must_use]
pub const fn xray_rook_attacks(occupied: Bitboard, blockers: Bitboard, sq: Square) -> Bitboard {
    let attacks = rook_attacks(sq, occupied);
    let blockers = blockers.and(attacks);
    attacks.xor(rook_attacks(sq, occupied.xor(blockers)))
}

/// The bishop-ray equivalent of [`xray_rook_attacks`]; same contract, same gotcha about
/// `blockers`, diagonal rays instead of rank/file.
#[must_use]
pub const fn xray_bishop_attacks(occupied: Bitboard, blockers: Bitboard, sq: Square) -> Bitboard {
    let attacks = bishop_attacks(sq, occupied);
    let blockers = blockers.and(attacks);
    attacks.xor(bishop_attacks(sq, occupied.xor(blockers)))
}

/// `color`'s own pieces that are pinned to `king_sq` by an enemy slider: each may
/// legally move only within `tables::line(king_sq, its_own_square)`, everywhere else
/// left illegal by exposing the king.
///
/// `king_sq` is a parameter rather than looked up here: `legal_moves` needs it for
/// several other things in the same node (king-move legality, `checkers`, the check
/// mask below), so it's computed once there and threaded through rather than
/// re-deriving it in every helper that happens to need it too.
///
/// Built from [`xray_rook_attacks`]/[`xray_bishop_attacks`] radiating from `king_sq`
/// itself (the "reverse" trick `attackers_of` already uses elsewhere in this module):
/// intersect each xray result with the matching enemy slider type
/// (rook-rays against enemy rooks *and* queens, bishop-rays against enemy bishops
/// *and* queens), then `tables::between(king_sq, pinner_sq)` intersected with `color`'s
/// own pieces for each pinner found gives the actual pinned square. No pinner identity
/// is kept in the result, only the pinned squares themselves: nothing downstream needs
/// to know *which* enemy piece is doing the pinning, only that the square is
/// pin-restricted and to what line.
#[must_use]
pub fn pinned(board: &Board, color: Color, king_sq: Square) -> Bitboard {
    let mut pinned = Bitboard::EMPTY;

    let blockers = board[color];

    let enemy_color = color.flip();
    let enemy_queens = board.pieces(enemy_color, Piece::Queen);
    let enemy_rooks = board.pieces(enemy_color, Piece::Rook).or(enemy_queens);
    let enemy_bishops = board.pieces(enemy_color, Piece::Bishop).or(enemy_queens);

    let rook_xrays = xray_rook_attacks(board.occupied(), blockers, king_sq);
    let rook_pinners = rook_xrays.and(enemy_rooks);
    for rook_sq in rook_pinners {
        pinned = pinned.or(between(king_sq, rook_sq).and(blockers));
    }

    let bishop_xrays = xray_bishop_attacks(board.occupied(), blockers, king_sq);
    let bishop_pinners = bishop_xrays.and(enemy_bishops);
    for bishop_sq in bishop_pinners {
        pinned = pinned.or(between(king_sq, bishop_sq).and(blockers));
    }

    pinned
}

/// Every enemy piece currently attacking `color`'s king on `king_sq`.
///
/// `attackers_of` under the name the chess programming literature already uses for
/// this exact query (CPW/Stockfish both call it `checkers`), since `legal_moves` needs
/// to branch on its popcount (zero, one, or two-plus) rather than just its emptiness
/// the way `in_check` does.
#[must_use]
pub fn checkers(board: &Board, king_sq: Square, by: Color) -> Bitboard {
    attackers_of(board, king_sq, by)
}

/// The squares a non-king move must land on to be legal, given `color`'s `checkers` at
/// `king_sq`.
///
/// Every square if `checkers` is empty (no check, no restriction), no square at all if
/// two or more checkers (double check, nothing but a king move resolves either one),
/// or the single checker's own square plus everything strictly between it and the
/// king if exactly one (block or capture).
///
/// Takes `checkers` rather than recomputing it: `legal_moves` already has it (see
/// [`checkers`]), and every non-king pseudo-legal move needs this same mask, so it's
/// computed once per node, not once per candidate move.
///
/// **En passant gotcha**: if the lone checker is the pawn that just double-pushed, an
/// en passant capture of it is legal, but its destination square is the empty square
/// *behind* that pawn, not the checker's own square, so this mask alone says that
/// destination is illegal, since it's neither the checker's square nor between it and
/// the king. `legal_moves` needs its own extra clause for this move shape specifically;
/// it can't be folded into this mask without this function also knowing about en
/// passant, which is a pawn-capture detail this square-geometry function shouldn't
/// need.
#[must_use]
pub const fn check_response_squares(king_sq: Square, checkers: Bitboard) -> Bitboard {
    if checkers.count() >= 2 {
        return Bitboard::EMPTY;
    }

    let checker = checkers.lsb();
    match checker {
        Some(checker) => checkers.or(between(checker, king_sq)),
        None => Bitboard::ALL,
    }
}

/// Union of every square attacked by any piece of `by`, against `occupied`.
///
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
#[must_use]
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
#[must_use]
pub fn is_attacked(board: &Board, sq: Square, by: Color) -> bool {
    !attacked_by(board, by, board.occupied())
        .and(sq.bitboard())
        .is_empty()
}

/// `color`'s king's square, or `None` if it has no king.
#[must_use]
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
#[must_use]
pub fn in_check(board: &Board, color: Color) -> bool {
    king_square(board, color).is_some_and(|sq| !attackers_of(board, sq, color.flip()).is_empty())
}

/// Which pieces of `by` attack `sq`, the superpiece trick.
///
/// Stand each piece type on `sq` in turn, radiate its attack pattern, and intersect with
/// the real pieces of that type/color. The reverse of every other function in this
/// module, and the one place a color bug can hide.
///
/// Knight, king, and slider attack relations are *symmetric*: "a attacks b"
/// iff "b attacks a", so radiating from `sq` works unmodified for those
/// four. Pawns are **not**: a white pawn on d3 attacks c4/e4, but a pawn
/// standing on c4 attacking as *white* would radiate onto b5/d5, not d3. To
/// find white pawns attacking `sq`, radiate a *black* pawn from `sq`
/// instead: `pawn_attacks(by.flip(), sq)`. The one other place this matters
/// is en passant source lookup in `pseudo_legal`. It produces the right answer on any
/// vertically symmetric test position even with the flip missing or
/// backward, so verify against an asymmetric one (a pawn a few ranks off the
/// board's horizontal midline is enough).
#[must_use]
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
