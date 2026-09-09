//! Pseudolegal move generation: every move a piece's basic movement rule allows, for
//! `board.side_to_move()`, without checking whether it leaves the mover's own king in
//! check.
//!
//! That filter is `legal`'s job. Each of the five generators below is its own `pub`
//! function (not folded into `pseudo_legal_moves`) so each gets its own reference-oracle
//! proptest, and a bug in one fails in isolation rather than inside a diff against the
//! whole move list.

use crate::board::Board;
use crate::move_gen::attacks::{attacked_by, king_square, piece_attacks};
use crate::move_gen::move_list::MoveList;
use crate::move_gen::tables::between;
use crate::{Bitboard, CastlingRights, Color, Direction, File, Move, MoveFlags, Piece, Square};

/// Generates every pseudolegal move for `board.side_to_move()` into `list`.
///
/// Calls the five functions below; their outputs never overlap (each covers a
/// disjoint set of piece types / move shapes), so order between them doesn't
/// matter.
pub fn pseudo_legal_moves(board: &Board, list: &mut MoveList) {
    pawn_moves(board, list);
    knight_moves(board, list);
    king_moves(board, list);
    slider_moves(board, list);
    castling_moves(board, list);
}

/// Whether `m` is exactly the move `pseudo_legal_moves(board)` would have produced for
/// it, without building the list.
///
/// Full re-derivation, not a shortcut: a stale or hash-collided TT move has
/// to be rejected here, not downstream. `tests/pseudo_legal_props.rs` states the
/// contract directly as membership in `pseudo_legal_moves`'s own output, over both
/// moves drawn from that output (must accept) and arbitrary `(from, to, flags)` triples
/// (must reject unless they happen to coincide with a real one).
#[must_use]
pub fn is_pseudo_legal(board: &Board, m: Move) -> bool {
    let sq_from = m.from();
    let (color_from, piece_from) = match board.piece_at(sq_from) {
        None => return false,
        Some(cp) => (cp.color(), cp.piece()),
    };

    if color_from != board.side_to_move() {
        return false;
    }

    let sq_to = m.to();
    let cp_to = board.piece_at(sq_to);
    // Facts about the `to` square itself, same family as `cp_to`: whether anything
    // at all sits there, and whether it's specifically an enemy. Two separate
    // predicates, not one overloaded flag: `Quiet`/`DoublePawnPush`/quiet-promotion
    // need "nothing at all," `Capture`/capture-promotion need "an enemy specifically."
    let sq_to_is_occupied = cp_to.is_some();
    let sq_to_is_occupied_by_enemy = cp_to.is_some_and(|cp| cp.color() != color_from);

    // Facts about what kind of move this is, independent of either square alone.
    let is_en_passant =
        piece_from == Piece::Pawn && board.en_passant().is_some_and(|ep| ep == sq_to);
    let is_promotion = piece_from == Piece::Pawn && sq_to.rank() == color_from.far_rank();

    match m.flags() {
        MoveFlags::Quiet => {
            if sq_to_is_occupied || is_promotion {
                return false;
            }
            match piece_from {
                Piece::Pawn => sq_from.bitboard().shift(color_from.forward()) == sq_to.bitboard(),
                Piece::Knight | Piece::Bishop | Piece::Rook | Piece::Queen | Piece::King => {
                    piece_attacks(piece_from, color_from, sq_from, board.occupied()).contains(sq_to)
                }
            }
        }
        MoveFlags::DoublePawnPush => {
            if piece_from != Piece::Pawn || sq_to_is_occupied || is_promotion {
                return false;
            }
            if sq_to.rank() != color_from.double_pawn_push_rank() {
                return false;
            }
            // Mirrors `pawn_pushes`: the double push is a single push, re-pushed, and
            // both squares (not just the landing one) have to be empty, or a blocker
            // on the intermediate square would wrongly validate.
            let push = sq_from.bitboard().shift(color_from.forward());
            push.and(board.occupied()).is_empty()
                && push.shift(color_from.forward()) == sq_to.bitboard()
        }
        MoveFlags::KingCastle => {
            if piece_from != Piece::King || sq_to_is_occupied || is_promotion {
                return false;
            }
            castle_is_pseudo_legal(
                board,
                color_from,
                sq_to,
                CastlingRights::kingside(color_from),
            )
        }
        MoveFlags::QueenCastle => {
            if piece_from != Piece::King || sq_to_is_occupied || is_promotion {
                return false;
            }
            castle_is_pseudo_legal(
                board,
                color_from,
                sq_to,
                CastlingRights::queenside(color_from),
            )
        }
        MoveFlags::Capture => {
            if is_en_passant || is_promotion {
                return false;
            }
            if !sq_to_is_occupied_by_enemy {
                return false;
            }
            piece_attacks(piece_from, color_from, sq_from, board.occupied()).contains(sq_to)
        }
        MoveFlags::EnPassant => {
            // `is_promotion` can never also be true here (the en passant target
            // square is never the far rank), so there's nothing else to guard.
            if !is_en_passant {
                return false;
            }
            piece_attacks(piece_from, color_from, sq_from, board.occupied()).contains(sq_to)
        }
        MoveFlags::PromoteKnight
        | MoveFlags::PromoteBishop
        | MoveFlags::PromoteRook
        | MoveFlags::PromoteQueen => {
            if !is_promotion || sq_to_is_occupied {
                return false;
            }
            sq_from.bitboard().shift(color_from.forward()) == sq_to.bitboard()
        }
        MoveFlags::PromoteCaptureKnight
        | MoveFlags::PromoteCaptureBishop
        | MoveFlags::PromoteCaptureRook
        | MoveFlags::PromoteCaptureQueen => {
            if !is_promotion || !sq_to_is_occupied_by_enemy {
                return false;
            }
            piece_attacks(piece_from, color_from, sq_from, board.occupied()).contains(sq_to)
        }
    }
}

/// Whether `color` may castle per `rights` and land on `sq_to` (`is_pseudo_legal`'s
/// `KingCastle`/`QueenCastle` arms, one call each with `rights` fixed to that side).
///
/// **Requires `rights` to already be a single side** (`CastlingRights::kingside(color)`
/// or `queenside(color)`, never a union of the two): everything else here, including
/// which file the rook starts on and which square the king lands on, is derived from
/// that one value plus `color` rather than taken as separate parameters. `rights`
/// itself decides kingside vs. queenside via equality against the known-singular
/// `kingside(color)` constant; `CastlingRights` has no general "which side is this"
/// query, because for any non-singular value the question wouldn't have one answer.
///
/// The rook's home square comes from `CastlingRights::rook_squares`, the same constant
/// table `Board::make_move` itself trusts outright with no board lookup: if `rights`
/// is held, the rook has necessarily never moved (the only thing that clears a
/// corner's right), so there's nothing to disambiguate the way `castling_moves`'s own
/// dynamic `board.pieces(...)` lookup has to (that one exists for the promoted-rook
/// case perft caught, which can't arise for a square nothing has ever searched to
/// begin with). Same reasoning gives the king's own square directly, without needing
/// `sq_from` from the caller: `is_pseudo_legal`'s top level already confirmed a King of
/// `color` sits at `sq_from`, and there's only one king per color, so if `rights` holds
/// at all, that king has never moved off `color.back_rank()`'s `File::E`.
fn castle_is_pseudo_legal(
    board: &Board,
    color: Color,
    sq_to: Square,
    rights: CastlingRights,
) -> bool {
    if !board.castling_rights().contains(rights) {
        return false;
    }
    let kingside = rights == CastlingRights::kingside(color);
    let king_sq = Square::new(File::E, color.back_rank());
    let expected_to = Square::new(if kingside { File::G } else { File::C }, color.back_rank());
    if sq_to != expected_to {
        return false;
    }
    let (rook_sq, _rook_to) = CastlingRights::rook_squares(color, kingside);
    let occupied = board.occupied();
    let attacked = attacked_by(board, color.flip(), occupied);
    castle_path_is_clear(occupied, king_sq, rook_sq, attacked)
}

/// Whether a king on `king_sq` may castle with the rook on `rook_sq`, given `occupied`
/// and `attacked` (by the opponent) for the position: the squares between them are
/// empty, and the king's start/transit (excluding `File::B`, which only the rook
/// crosses)/landing squares are all unattacked.
///
/// Pure bitboard math, no `Board` access of its own: both `castling_moves` (which
/// already has `occupied`/`attacked` computed once, shared across a kingside *and*
/// queenside check in the same call) and `castle_is_pseudo_legal` (which computes them
/// itself, since at most one castle side is ever in play per call) already have
/// exactly what this needs, and recomputing either bitboard here a second time would
/// cost `castling_moves` a real second `attacked_by` scan over every enemy piece.
const fn castle_path_is_clear(
    occupied: Bitboard,
    king_sq: Square,
    rook_sq: Square,
    attacked: Bitboard,
) -> bool {
    let between_sqs = between(king_sq, rook_sq);
    if !between_sqs.and(occupied).is_empty() {
        return false;
    }
    let king_path = between_sqs.and_not(File::B.bitboard());
    king_path.or(king_sq.bitboard()).and(attacked).is_empty()
}

/// Pushes all four promotion variants (`from` -> `to`), or all four
/// capturing-promotion variants if `capturing`, in a fixed Bishop/Knight/
/// Rook/Queen order. The one place that order is spelled out: `pawn_moves`'s
/// capture loop and `pawn_pushes`'s quiet-promotion branch both call this
/// rather than each listing the same four `push` calls.
fn push_promotions(list: &mut MoveList, from: Square, to: Square, capturing: bool) {
    let flags: [MoveFlags; 4] = if capturing {
        [
            MoveFlags::PromoteCaptureBishop,
            MoveFlags::PromoteCaptureKnight,
            MoveFlags::PromoteCaptureRook,
            MoveFlags::PromoteCaptureQueen,
        ]
    } else {
        [
            MoveFlags::PromoteBishop,
            MoveFlags::PromoteKnight,
            MoveFlags::PromoteRook,
            MoveFlags::PromoteQueen,
        ]
    };
    for flag in flags {
        list.push(Move::new(from, to, flag));
    }
}

/// Pushes (via `pawn_pushes`), captures, en passant, and all four capturing-promotion
/// variants for every pawn of `board.side_to_move()`.
///
/// Each pawn's own attack squares (`piece_attacks`) are checked against the en passant
/// target and the enemy occupancy directly, rather than reversing the lookup the way
/// `attacks::attackers_of` does: there's only one pawn's worth of targets per iteration,
/// so there's no set to intersect against.
pub fn pawn_moves(board: &Board, list: &mut MoveList) {
    let color = board.side_to_move();
    pawn_pushes(board, list, color);

    let en_passant = board.en_passant().map_or(Bitboard::EMPTY, Square::bitboard);
    let enemy = board[color.flip()];
    let occupied = board.occupied();
    for sq in board.pieces(color, Piece::Pawn) {
        let pawn_attacks = piece_attacks(Piece::Pawn, color, sq, occupied);
        for target in pawn_attacks {
            if en_passant.contains(target) {
                list.push(Move::new(sq, target, MoveFlags::EnPassant));
            } else if enemy.contains(target) {
                if target.rank() == color.far_rank() {
                    push_promotions(list, sq, target, true);
                } else {
                    list.push(Move::new(sq, target, MoveFlags::Capture));
                }
            }
        }
    }
}

/// Single and double pushes, and quiet-promotion variants, for every pawn of
/// `color`. Pushing twice *through* `empty` (not a single shift-by-16) is
/// what makes a blocker on the intermediate square stop the double push.
fn pawn_pushes(board: &Board, list: &mut MoveList, color: Color) {
    let empty = board.empty();
    let dir = color.forward();
    for sq in board.pieces(color, Piece::Pawn) {
        let push = sq.bitboard().shift(dir).and(empty);
        if !push.is_empty() {
            let push_sq = push.lsb().expect("push is non-empty");
            if push_sq.rank() == color.far_rank() {
                push_promotions(list, sq, push_sq, false);
            } else {
                list.push(Move::new(sq, push_sq, MoveFlags::Quiet));
                let double_push = push
                    .shift(dir)
                    .and(empty)
                    .and(color.double_pawn_push_rank().bitboard());
                if !double_push.is_empty() {
                    let double_push_sq = double_push.lsb().expect("double_push is non-empty");
                    list.push(Move::new(sq, double_push_sq, MoveFlags::DoublePawnPush));
                }
            }
        }
    }
}

/// Every quiet move and capture for every knight of `board.side_to_move()`.
pub fn knight_moves(board: &Board, list: &mut MoveList) {
    nonpawn_moves(board, list, Piece::Knight);
}

/// Every quiet move and capture for `board.side_to_move()`'s king, deliberately including
/// moves onto attacked squares.
///
/// Pre-filtering against `attacks::attacked_by` here would be a plausible-looking
/// optimization that's wrong on its own: it doesn't know about pins, discovered checks,
/// or castling-through-check, and `legal`'s copy-make already has to handle all three, so
/// there's no partial win from duplicating part of it here.
pub fn king_moves(board: &Board, list: &mut MoveList) {
    nonpawn_moves(board, list, Piece::King);
}

/// Every quiet move and capture for every bishop, rook, and queen of
/// `board.side_to_move()`.
pub fn slider_moves(board: &Board, list: &mut MoveList) {
    nonpawn_moves(board, list, Piece::Bishop);
    nonpawn_moves(board, list, Piece::Rook);
    nonpawn_moves(board, list, Piece::Queen);
}

fn nonpawn_moves(board: &Board, list: &mut MoveList, piece: Piece) {
    let color = board.side_to_move();
    let pieces = board.pieces(color, piece);
    let occupied = board.occupied();
    for sq in pieces {
        let targets = piece_attacks(piece, color, sq, occupied).and_not(board[color]);
        let captures = targets.and(board[color.flip()]);
        for c in captures {
            list.push(Move::new(sq, c, MoveFlags::Capture));
        }
        let quiet = targets.and_not(captures);
        for q in quiet {
            list.push(Move::new(sq, q, MoveFlags::Quiet));
        }
    }
}

/// Kingside and queenside castling for `board.side_to_move()`.
///
/// Legal only where the relevant `CastlingRights` bit is set, the squares between king
/// and rook are empty, and the king's start/transit/landing squares are all unattacked.
///
/// `castle_path_is_clear` derives every square that matters
/// (`tables::between(king_sq, rook_sq)`) from where the king and rook actually
/// stand, so Black isn't a separate case: it falls out of `king_sq`/`rook_sq` already
/// being Black's squares. The rook lookup below filters by `color.back_rank()` as well as
/// file, not file alone: "the piece of `color`/`Rook` on file A/H" is wrong the moment a
/// pawn has promoted to a rook on that file (e.g. Black promoting on a1 while Black's
/// real queenside rook is still on a8). Picking the wrong one doesn't panic: `between` on
/// two unaligned squares returns `Bitboard::EMPTY`, which trivially passes the occupancy
/// check and collapses the safety check to "is `king_sq` itself attacked", skipping the
/// real transit squares. Perft caught this at depth 4 on the standard "Position 4" test
/// position, built to reach exactly this promotion-creates-an-ambiguous-same-file-rook
/// scenario within a few plies; no hand-written FEN scenario thought to construct it.
///
/// This needs `attacks::attacked_by` here rather than being deferred to `legal`'s filter:
/// `legal`'s copy-make only inspects the *resulting* position, so it can catch landing in
/// check but not castling *through* it. b1/b8 must be **empty but need not be
/// unattacked**: the king never crosses it, only the rook does, so `castle_path_is_clear`'s
/// occupancy check uses the full `between` set while its safety check excludes `File::B`.
///
/// # Panics
///
/// If `board`'s `CastlingRights` claims a side has castling rights but the expected rook
/// isn't actually on its corner square, or if `board` has no king of `color` at all: both
/// are invariants `Board` is supposed to maintain, not conditions this function is meant
/// to recover from.
pub fn castling_moves(board: &Board, list: &mut MoveList) {
    let color = board.side_to_move();
    let Some(king_sq) = king_square(board, color) else {
        return;
    };
    let occupied = board.occupied();
    let attacked = attacked_by(board, color.flip(), occupied);

    let castle_sq = |dir: Direction| {
        king_sq
            .bitboard()
            .shift(dir)
            .shift(dir)
            .lsb()
            .expect("started with a king")
    };

    let rights = board.castling_rights().without_color(color.flip());
    let kingside = rights.contains(CastlingRights::kingside(color));
    if kingside {
        let rook_sq = board
            .pieces(color, Piece::Rook)
            .and(File::H.bitboard())
            .and(color.back_rank().bitboard())
            .lsb()
            .expect("CastlingRights says we have a rook there");
        if castle_path_is_clear(occupied, king_sq, rook_sq, attacked) {
            let castle_sq = castle_sq(Direction::East);
            list.push(Move::new(king_sq, castle_sq, MoveFlags::KingCastle));
        }
    }
    let queenside = rights.contains(CastlingRights::queenside(color));
    if queenside {
        let rook_sq = board
            .pieces(color, Piece::Rook)
            .and(File::A.bitboard())
            .and(color.back_rank().bitboard())
            .lsb()
            .expect("CastlingRights says we have a rook there");
        if castle_path_is_clear(occupied, king_sq, rook_sq, attacked) {
            let castle_sq = castle_sq(Direction::West);
            list.push(Move::new(king_sq, castle_sq, MoveFlags::QueenCastle));
        }
    }
}
