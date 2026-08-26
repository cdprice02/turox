//! Pseudolegal move generation: every move a piece's basic movement rule
//! allows, for `board.side_to_move()`, without checking whether it leaves the
//! mover's own king in check. That filter is `legal`'s job — see this module's
//! doc note on why the split exists.
//!
//! # Public surface
//!
//! - `fn pseudo_legal_moves(board: &Board, list: &mut MoveList)` — calls all
//!   five functions below, in any order (their outputs don't overlap).
//! - `fn pawn_moves(board: &Board, list: &mut MoveList)`
//! - `fn knight_moves(board: &Board, list: &mut MoveList)`
//! - `fn king_moves(board: &Board, list: &mut MoveList)`
//! - `fn slider_moves(board: &Board, list: &mut MoveList)` — bishop, rook, queen
//! - `fn castling_moves(board: &Board, list: &mut MoveList)`
//!
//! Each is `pub`, not just an internal helper of `pseudo_legal_moves`, so each
//! gets its own reference-oracle proptest and a bug in, say, pawn generation
//! fails in isolation rather than inside a 40-move diff against the whole list.
//!
//! # Why king moves aren't filtered here
//!
//! `king_moves` generates every king step onto a non-friendly square, including
//! ones an enemy piece attacks — that's `legal`'s job, once it exists, via
//! copy-make (`board.make_move(m)` then check the resulting king isn't
//! attacked). Pre-filtering with `attacks::attacked_by` here would be a
//! plausible-looking optimization that's actually wrong on its own: it doesn't
//! know about pins, discovered checks, or (see below) castling-through-check,
//! so it would need `legal`'s machinery anyway. Keep the concerns separate.
//!
//! # Knights, king, sliders
//!
//! `knight_moves`/`king_moves`/`slider_moves` are all one shared helper,
//! `nonpawn_moves(board, list, piece)`, parameterized on `Piece` rather than
//! three near-duplicate functions. Per source square, through
//! `attacks::piece_attacks` (the one place the `Piece` -> attack-fn dispatch
//! lives, rather than reaching into `tables`/`magic` directly):
//!
//! ```text
//! targets  = piece_attacks(piece, us, from, occupied).and_not(board[us])
//! captures = targets.and(board[them])       -> MoveFlags::Capture
//! quiet    = targets.and_not(captures)      -> MoveFlags::Quiet
//! ```
//!
//! `occupied` is hoisted once per `nonpawn_moves` call rather than
//! recomputed per source square — cheap either way (`board.occupied()` is one
//! `OR` of two stored fields), but consistent with the same hoisting done in
//! `castling_moves` below. `quiet` comes from subtracting `captures` out of
//! `targets` rather than intersecting with `board.empty()` — equivalent given
//! every square is empty/ours/theirs, and avoids a third lookup.
//!
//! # Castling
//!
//! The {Color}x{kingside,queenside} four-way mapping `CLAUDE.md` flags as a
//! repeat offender. `castling_moves` handles it with two small closures shared
//! between the kingside and queenside branches, rather than four independent
//! code paths: `valid_castle(rook_sq)` derives every square that matters
//! (`tables::between(king_sq, rook_sq)`) directly from where the king and rook
//! actually are, so it needs no per-color or per-side branching at all —
//! Black castling isn't a separate case, it falls out of `king_sq`/`rook_sq`
//! already being Black's squares. `castle_sq(dir)` computes the landing square
//! by shifting the king two steps in `dir`; both branches call it
//! (`castle_sq(Direction::East)` / `castle_sq(Direction::West)`) instead of
//! hand-rolling the shift, since a hand-rolled copy in just one branch is
//! exactly the shape that let the `KingCastle`/`QueenCastle` flag get crossed
//! once already during development.
//!
//! `valid_castle` checks two things: `between(king_sq, rook_sq)` must be fully
//! empty (the rook's path — for queenside this includes b1/b8), and
//! `between(...).and_not(File::B.bitboard())` unioned with `king_sq` itself must be fully
//! unattacked (the king's path, which never touches the b-file). b1/b8 must be
//! **empty but need not be unattacked** — the king never crosses it, only the
//! rook does — which is exactly why the occupancy check uses the full
//! `between` set but the safety check excludes `File::B`. `occupied` and
//! `attacked = attacks::attacked_by(board, them, occupied)` are both computed
//! once before either branch runs, not once per closure call. Using the
//! board's actual occupancy (not a king-removed one) for `attacked` is correct
//! here: the only way our own king could shadow a transit square from an enemy
//! slider is via the same rank the king sits on, and a slider on that rank is
//! already attacking the king's own square, which fails the check regardless.
//!
//! This is why `pseudo_legal` depends on `attacks` (merged in the prior PR):
//! `legal`'s copy-make filter only inspects the *resulting* position, so on its
//! own it can't catch castling *through* check — only landing in it. Rook
//! presence on the corner is implied by the castling right and is already
//! assumed by `Board::make_move`.
//!
//! **The rook lookup needs a rank filter, not just a file filter.** Finding
//! the castling rook as "the piece of `color`/`Rook` on file A/H" is wrong the
//! moment a pawn has promoted to a rook that happens to land on the same file
//! — e.g. a black pawn promoting on a1 while black's real queenside rook is
//! still on a8; both are "a rook on the A-file", but only one is on
//! `color.back_rank()`. Picking the wrong one silently breaks `valid_castle`
//! rather than panicking: `between(king_sq, wrong_rook_sq)` for two squares
//! that aren't even aligned returns `Bitboard::EMPTY`, which trivially passes
//! the occupancy check and collapses the safety check down to "is `king_sq`
//! itself attacked", skipping the real transit squares entirely. Perft caught
//! this at depth 4 on the standard "Position 4" test position specifically
//! because that position is built to reach exactly this promotion-creates-an-
//! ambiguous-same-file-rook scenario within a few plies — the six-position
//! perft suite exists precisely to catch cases like this one, that no
//! hand-written FEN scenario thought to construct.
//!
//! # Pawns
//!
//! Split across `pawn_moves` (captures, en passant, and promotion-by-capture)
//! and `pawn_pushes` (quiet pushes, double pushes, and promotion-by-push),
//! both looping per pawn rather than shifting the whole `pawns` bitboard at
//! once — since `from` is already in hand at each iteration, there's no need
//! to reverse-derive a source square from a target the way a fully batched
//! approach would.
//!
//! - `pawn_pushes`: `sq.bitboard().shift(dir).and(empty)` for the single push;
//!   shifting that result by `dir` *again* (through `empty` a second time,
//!   then masked to `color.double_pawn_push_rank()`) for the double push.
//!   Pushing twice *through* `empty` is what makes a blocker on the
//!   intermediate square stop the double push; a single shift-by-16 would skip
//!   right over it. The double-push branch is nested inside the
//!   non-promotion `else`, since a double push can never land on the back
//!   rank by construction.
//! - Promotion: both the push and the capture branch check
//!   `target.rank() == color.far_rank()` before deciding between one
//!   `Quiet`/`Capture` move and the four `Promote{Knight,Bishop,Rook,Queen}`
//!   (or `PromoteCapture*`) moves. These two checks are independent code
//!   paths — one was implemented before the other during development, which
//!   is exactly the failure mode to watch for if a pawn rule ever needs a
//!   third variant: the push and capture branches never share logic, so a
//!   fix to one doesn't propagate to the other.
//! - En passant: sources for the ep target square are
//!   `tables::pawn_attacks(them, ep_sq) & board.pieces(us, Pawn)` — the same
//!   reverse-the-color reasoning `attacks::attackers_of` uses (stand the
//!   *opposing* color's pawn on the target square to find which of *our*
//!   pawns could have captured onto it), and the second and last place in the
//!   crate that trick is needed. Flag is `MoveFlags::EnPassant`, **not**
//!   `Capture` — `MoveFlags::is_capture()` already covers both.

use crate::board::Board;
use crate::move_gen::attacks::{attacked_by, king_square, piece_attacks};
use crate::move_gen::move_list::MoveList;
use crate::move_gen::tables::between;
use crate::{Bitboard, CastlingRights, Color, Direction, File, Move, MoveFlags, Piece};

/// Generates every pseudolegal move for `board.side_to_move()` into `list`.
/// Calls the five functions below; their outputs never overlap (each covers a
/// disjoint set of piece types / move shapes), so order between them doesn't
/// matter.
#[allow(unused_variables)]
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
