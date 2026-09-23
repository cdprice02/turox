//! Pin- and check-aware legal move generation.
//!
//! Plus `perft`, the standard recursive node-count benchmark that doubles as the
//! practical correctness gate on `pseudo_legal`, `attacks`, and `Board::make_move`
//! together.

use crate::board::Board;
use crate::move_gen::attacks::{
    attacked_by, check_response_squares, checkers, in_check, king_square, pinned,
};
use crate::move_gen::magic::{bishop_attacks, rook_attacks};
use crate::move_gen::move_list::MoveList;
use crate::move_gen::pseudo_legal::pseudo_legal_moves;
use crate::move_gen::tables::line;
use crate::{Color, Piece, Square};

/// Every legal move for `board.side_to_move()`, computing a pin set and a check-response
/// mask once per node rather than proving each candidate legal by playing it and
/// rescanning the result.
///
/// Two passes over `pseudo_legal_moves`' output, in this order because the
/// first is conditional and shrinks what the second pass has to look at:
///
/// 1. While in check, every non-king move must land in `check_response_squares`, with
///    its own carve-out for capturing the checking pawn en passant. King moves pass
///    through untouched here: `check_response_squares` doesn't apply to them at all
///    (see its own doc), so their legality is left entirely to the second pass, check
///    or no check.
/// 2. Always: a pinned piece may move only within `tables::line(king_sq, its own
///    square)`; an en passant capture must also pass `en_passant_is_legal`; and a king
///    move must land on a square `attacked_without_king` doesn't cover.
///
/// `legal_moves_naive` below is the naive reference this has to agree with;
/// `tests/legal_props.rs`'s proptest checks exactly that.
#[must_use]
pub fn legal_moves(board: &Board) -> MoveList {
    let mut moves = MoveList::default();

    let color = board.side_to_move();
    let Some(king_sq) = king_square(board, color) else {
        return moves;
    };

    pseudo_legal_moves(board, &mut moves);

    // One `checkers` scan rather than two: `in_check` and `checkers` both call
    // `attackers_of` on `king_sq`, so checking `in_check` first and then computing
    // `checkers` again inside the `if` scanned the enemy's attacks on the king twice
    // on every single call, not just the ones actually in check.
    let checkers = checkers(board, king_sq, color.flip());
    if !checkers.is_empty() {
        let check_response_squares = check_response_squares(king_sq, checkers);
        moves.retain(|m| {
            let from = m.from();
            let to = m.to();

            // King moves are left untouched here: `check_response_squares` is a
            // non-king mask (the king escaping to a third square that's neither the
            // checker's square nor between it and the king is often the *only*
            // legal response), and the `king_is_safe` check below already covers
            // king-move legality unconditionally, check or no check.
            if from == king_sq {
                return true;
            }

            // Capturing the checking pawn en passant resolves the check even though
            // `to` is the empty square behind that pawn, not the checker's own
            // square, so `check_response_squares` alone says this destination is
            // illegal. `checkers.count() == 1` matters: with two checkers, capturing
            // one en passant still leaves the other unaddressed.
            if m.flags().is_en_passant() {
                let captured_sq = Square::new(to.file(), from.rank());
                if checkers.count() == 1 && checkers.contains(captured_sq) {
                    return true;
                }
            }

            check_response_squares.contains(to)
        });
    }

    // Computed once here rather than inside the closure below, same reasoning as
    // `checkers` above: every candidate move needs the same two board-wide facts, not a
    // fresh computation per move.
    let pinned = pinned(board, color, king_sq);
    let attacked_without_king = attacked_by(board, color.flip(), board.occupied().without(king_sq));
    moves.retain(|m| {
        let from = m.from();
        let to = m.to();

        if pinned.contains(from) && !line(from, king_sq).contains(to) {
            return false;
        }

        // Checked unconditionally, not only for moves the first pass didn't already
        // special-case: that pass's carve-out is about resolving an existing check by
        // capturing the checking pawn; this is about whether the same capture exposes a
        // *different* check by removing a piece that was blocking a slider. The two are
        // independent, and a move has to clear both.
        if m.flags().is_en_passant() && !en_passant_is_legal(board, color, from, to, king_sq) {
            return false;
        }

        if from == king_sq && attacked_without_king.contains(to) {
            return false;
        }

        true
    });
    moves
}

/// Whether an en passant capture from `from` to `to` is legal for `color`, beyond what
/// `pinned`/`check_response_squares` already say about `from`/`to` themselves.
///
/// The one move shape where removing a piece other than the one standing on `to`
/// (the captured pawn sits on the same rank as `from`, one file over, not on `to`
/// itself) can expose the king: with *both* the capturing pawn's own square and the
/// captured pawn's square removed from occupancy, does an enemy rook, bishop, or queen
/// now attack `color`'s king. Neither `pinned` nor a normal `make_move` pin check would
/// catch this: the captured pawn was never pinned (it isn't `color`'s piece), and the
/// capturing pawn moving off `from` isn't what exposes the king, the *other* pawn
/// disappearing off the board entirely is.
///
/// Deliberately checks sliders directly (`rook_attacks`/`bishop_attacks` radiating from
/// `king_sq`) rather than calling `attacked_by`: `attacked_by` loops over the real
/// board's own piece lists, so removing a square from its `occupied` argument only
/// affects *slider* attacks, which are the only ones that consult `occupied` at all.
/// The captured pawn's own attack on the king (exactly what's in play when the checker
/// being captured *is* that pawn) doesn't care about occupancy and would still count,
/// producing a false "still attacked" here for precisely the en-passant-escapes-check
/// case `check_response_squares`'s own doc already flags. Since the captured piece in
/// en passant is always a pawn, never a slider, restricting to sliders makes that false
/// positive structurally impossible rather than special-casing it away.
///
/// Deliberately not folded into `pinned` or `check_response_squares`: both of those
/// are pure king/slider geometry, and this needs pawn-capture-specific knowledge (which
/// square the captured pawn is actually on) that would leak a narrow special case into
/// otherwise general-purpose helpers.
#[must_use]
pub const fn en_passant_is_legal(
    board: &Board,
    color: Color,
    from: Square,
    to: Square,
    king_sq: Square,
) -> bool {
    let captured_sq = Square::new(to.file(), from.rank());
    let occupied = board.occupied().without(from).without(captured_sq).with(to);

    let enemy_color = color.flip();
    let enemy_queens = board.pieces(enemy_color, Piece::Queen);
    let enemy_rooks = board.pieces(enemy_color, Piece::Rook).or(enemy_queens);
    let enemy_bishops = board.pieces(enemy_color, Piece::Bishop).or(enemy_queens);

    let attacked_by_rook = rook_attacks(king_sq, occupied).and(enemy_rooks);
    let attacked_by_bishop = bishop_attacks(king_sq, occupied).and(enemy_bishops);

    attacked_by_rook.is_empty() && attacked_by_bishop.is_empty()
}

/// Every legal move for `board.side_to_move()`: `pseudo_legal_moves` filtered in place,
/// via `MoveList::retain`, to the moves that don't leave the mover's own king in check.
///
/// No separate pin detection, no discovered-check bookkeeping. `board.make_move(m)`
/// produces the actual resulting position and `in_check` actually re-scans it, so pins,
/// discovered checks, and en-passant-discovered checks along the capturing pawn's rank
/// all fall out for free. Kept as the naive reference for `legal_moves` above rather
/// than deleted: obviously correct by construction, at the cost of a full `make_move`
/// per candidate instead of a cheap precomputed pin-mask test.
#[must_use]
pub fn legal_moves_naive(board: &Board) -> MoveList {
    let mut moves = MoveList::default();
    pseudo_legal_moves(board, &mut moves);

    // `side_to_move` before `board.make_move` flips it
    let color = board.side_to_move();

    moves.retain(|m| !in_check(&board.make_move(m), color));
    moves
}

/// The number of leaf positions reachable from `board` after exactly `depth` plies of
/// legal play.
///
/// `depth == 0` stays its own branch rather than falling out of the `depth == 1`
/// bulk-counting shortcut below it: `perft` is called directly with `depth == 0`
/// (`perft_zero_is_one_leaf`, and indirectly any depth-1 call's own base case), and
/// without that branch `depth - 1` underflows `u32` before reaching the `depth == 1`
/// check.
#[must_use]
pub fn perft(board: &Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = legal_moves(board);
    if depth == 1 {
        return u64::try_from(moves.len()).unwrap_or(u64::MAX);
    }
    moves
        .as_slice()
        .iter()
        .map(|&m| perft(&board.make_move(m), depth - 1))
        .sum()
}
