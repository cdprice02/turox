//! Pseudolegal move generation: every move a piece's basic movement rule
//! allows, for `board.side_to_move()`, without checking whether it leaves the
//! mover's own king in check — that filter is `legal`'s job. Each of the five
//! generators below is its own `pub` function (not folded into
//! `pseudo_legal_moves`) so each gets its own reference-oracle proptest, and a
//! bug in one fails in isolation rather than inside a diff against the whole
//! move list.
//!
//! `king_moves` deliberately does not pre-filter against `attacks::attacked_by`:
//! that would be a plausible-looking optimization that's wrong on its own,
//! since it doesn't know about pins, discovered checks, or castling-through-
//! check — `legal`'s copy-make already has to handle all three, so there's no
//! partial win from duplicating part of it here.
//!
//! # Castling
//!
//! The {Color}x{kingside,queenside} four-way mapping `CLAUDE.md` flags as a
//! repeat offender. `castling_moves` derives every square that matters
//! (`tables::between(king_sq, rook_sq)`) from where the king and rook actually
//! stand, so Black isn't a separate case — it falls out of `king_sq`/`rook_sq`
//! already being Black's squares. b1/b8 must be **empty but need not be
//! unattacked** — the king never crosses it, only the rook does — so the
//! occupancy check uses the full `between` set while the safety check excludes
//! `File::B`. Landing in check is `legal`'s job; castling *through* check isn't,
//! since `legal`'s copy-make filter only inspects the resulting position.
//!
//! **The rook lookup needs a rank filter, not just a file filter.** "The piece
//! of `color`/`Rook` on file A/H" is wrong the moment a pawn has promoted to a
//! rook on that file — e.g. Black promoting on a1 while Black's real queenside
//! rook is still on a8. Picking the wrong one doesn't panic: `between` on two
//! unaligned squares returns `Bitboard::EMPTY`, which trivially passes the
//! occupancy check and collapses the safety check to "is `king_sq` itself
//! attacked", skipping the real transit squares. Perft caught this at depth 4
//! on the standard "Position 4" test position, which is built to reach exactly
//! this promotion-creates-an-ambiguous-same-file-rook scenario within a few
//! plies — no hand-written FEN scenario thought to construct it.
//!
//! # Pawns
//!
//! Pushing twice *through* `empty` (not a single shift-by-16) is what makes a
//! blocker on the intermediate square stop a double push. The push and capture
//! branches each independently check for landing on the promotion rank —
//! they don't share logic, which is the failure mode to watch for if a pawn
//! rule ever needs a third variant. En passant sources are
//! `tables::pawn_attacks(them, ep_sq) & board.pieces(us, Pawn)`, the same
//! reverse-the-color reasoning as `attacks::attackers_of`, and the crate's
//! second and last use of that trick.

use crate::board::Board;
use crate::move_gen::attacks::{attacked_by, king_square, piece_attacks};
use crate::move_gen::move_list::MoveList;
use crate::move_gen::tables::between;
use crate::{Bitboard, CastlingRights, Color, Direction, File, Move, MoveFlags, Piece};

/// Generates every pseudolegal move for `board.side_to_move()` into `list`.
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

/// Pushes, double pushes, captures (including en passant), and all four
/// promotion variants (quiet and capturing) for every pawn of
/// `board.side_to_move()`.
pub fn pawn_moves(board: &Board, list: &mut MoveList) {
    let color = board.side_to_move();
    pawn_pushes(board, list, color);

    let en_passant = board
        .en_passant()
        .map_or(Bitboard::EMPTY, |sq| sq.bitboard());
    let enemy = board[color.flip()];
    let occupied = board.occupied();
    for sq in board.pieces(color, Piece::Pawn) {
        let pawn_attacks = piece_attacks(Piece::Pawn, color, sq, occupied);
        for target in pawn_attacks {
            if en_passant.contains(target) {
                list.push(Move::new(sq, target, MoveFlags::EnPassant));
            } else if enemy.contains(target) {
                if target.rank() == color.far_rank() {
                    list.push(Move::new(sq, target, MoveFlags::PromoteCaptureBishop));
                    list.push(Move::new(sq, target, MoveFlags::PromoteCaptureKnight));
                    list.push(Move::new(sq, target, MoveFlags::PromoteCaptureRook));
                    list.push(Move::new(sq, target, MoveFlags::PromoteCaptureQueen));
                } else {
                    list.push(Move::new(sq, target, MoveFlags::Capture));
                }
            }
        }
    }
}

fn pawn_pushes(board: &Board, list: &mut MoveList, color: Color) {
    let empty = board.empty();
    let dir = match color {
        Color::White => Direction::North,
        Color::Black => Direction::South,
    };
    for sq in board.pieces(color, Piece::Pawn) {
        let push = sq.bitboard().shift(dir).and(empty);
        if !push.is_empty() {
            let push_sq = push.lsb().expect("push is non-empty");
            if push_sq.rank() == color.far_rank() {
                list.push(Move::new(sq, push_sq, MoveFlags::PromoteBishop));
                list.push(Move::new(sq, push_sq, MoveFlags::PromoteKnight));
                list.push(Move::new(sq, push_sq, MoveFlags::PromoteRook));
                list.push(Move::new(sq, push_sq, MoveFlags::PromoteQueen));
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

/// Every quiet move and capture for `board.side_to_move()`'s king — deliberately
/// including moves onto attacked squares; see the module doc for why that
/// filter belongs in `legal`, not here.
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

/// Kingside and queenside castling for `board.side_to_move()`, where the
/// relevant `CastlingRights` bit is set, the squares between king and rook are
/// empty, and the king's start/transit/landing squares are all unattacked. See
/// the module doc for why this needs `attacks::attacked_by` rather than being
/// deferred to `legal`'s filter.
pub fn castling_moves(board: &Board, list: &mut MoveList) {
    let color = board.side_to_move();
    let Some(king_sq) = king_square(board, color) else {
        return;
    };
    let occupied = board.occupied();
    let attacked = attacked_by(board, color.flip(), occupied);

    let valid_castle = |rook_sq| {
        let between = between(king_sq, rook_sq);
        if !between.and(occupied).is_empty() {
            return false;
        }
        let king_path = between.and_not(File::B.bitboard());
        king_path.or(king_sq.bitboard()).and(attacked).is_empty()
    };
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
        if valid_castle(rook_sq) {
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
        if valid_castle(rook_sq) {
            let castle_sq = castle_sq(Direction::West);
            list.push(Move::new(king_sq, castle_sq, MoveFlags::QueenCastle));
        }
    }
}
