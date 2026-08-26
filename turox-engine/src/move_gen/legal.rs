//! Pin- and check-aware legal move generation: the piece that ties
//! `pseudo_legal`, `attacks`, and `Board::make_move` together into an actual
//! list of legal moves for a position, plus `perft` — the standard recursive
//! node-count benchmark that is also the practical correctness gate on
//! everything under it.
//!
//! # Public surface
//!
//! - `fn legal_moves(board: &Board) -> MoveList`
//! - `fn perft(board: &Board, depth: u32) -> u64`
//!
//! # `legal_moves`: copy-make, not a pin/check detector
//!
//! The naive-but-correct approach, and the one to use here: generate
//! `pseudo_legal::pseudo_legal_moves` into a scratch `MoveList`, and for each
//! candidate move `m`, keep it iff `!attacks::in_check(&board.make_move(m),
//! us)`. No separate pin detection, no discovered-check bookkeeping — copy-make
//! answers both questions for free, because `board.make_move(m)` actually
//! produces the resulting position and `attacks::in_check` actually re-scans
//! it. This is *slower* than the pin-aware bitboard techniques a faster engine
//! would eventually want (each candidate move costs a full `make_move` plus a
//! full `attacked_by` scan, rather than a cheap pin-mask check), but it is the
//! version that is obviously correct by construction, which is what this PR
//! is for — perft is what proves it, and what a later perf pass would
//! benchmark against before reaching for something cleverer.
//!
//! **The trap**: `board.make_move(m)` flips `side_to_move` in the result.
//! Capture `us = board.side_to_move()` **before** calling `make_move`, and
//! filter on `in_check(&next, us)` — `in_check(&next, next.side_to_move())`
//! checks the *opponent's* king and is exactly backward. Because most
//! pseudolegal moves in a normal position don't leave the mover in check, a
//! backward filter doesn't fail quietly — every `legal_moves` call would keep
//! roughly the wrong moves and perft would be off almost immediately, at
//! depth 1 or 2, on nearly every position. Loud, but still worth stating
//! plainly rather than discovering by staring at a wrong node count.
//!
//! This same construction handles the classic edge cases other approaches
//! have to special-case, for free: a king moving away from a slider's ray is
//! correctly still in check in the copy (the king's old square is genuinely
//! empty in `next`, so the ray genuinely continues through it); an en passant
//! capture that discovers a check along the capturing pawn's rank is correctly
//! caught (`make_move`'s `EnPassant` branch actually removes the captured
//! pawn from its real square, not the destination square, before `in_check`
//! ever runs).
//!
//! # `perft`
//!
//! The recursive definition, with the standard "bulk counting" shortcut:
//! `perft(board, 0) = 1`; `perft(board, 1) = legal_moves(board).len()`
//! (skipping a full `make_move` + recurse-to-count-`1`s just to re-derive a
//! count `legal_moves` already produced); `perft(board, depth) =
//! sum(perft(board.make_move(m), depth - 1) for m in legal_moves(board))`
//! otherwise. `depth == 0` has to stay its own explicit case rather than
//! falling out of the `depth == 1` one: `perft` is called directly with
//! `depth == 0` (`tests/perft.rs`'s `perft_zero_is_one_leaf`, and indirectly
//! any depth-1 call's own base case), and without that branch `depth - 1`
//! underflows `u32` before ever reaching the `depth == 1` check.
//!
//! Six standard positions with known node counts (`tests/perft.rs`) exercise
//! `legal_moves`, `pseudo_legal`, `attacks`, `MoveList`, `make_move`, `tables`,
//! and `magic` together — a wrong count at low depth on any of them localizes
//! to a specific rule (a wrong count that only shows up at higher depth means
//! the bug is in a move that itself doesn't get exercised until deeper, e.g.
//! a promotion or an en passant availability that only arises a few plies in).

use crate::board::Board;
use crate::move_gen::attacks::in_check;
use crate::move_gen::move_list::MoveList;
use crate::move_gen::pseudo_legal::pseudo_legal_moves;

/// Every legal move for `board.side_to_move()`: `pseudo_legal_moves` filtered
/// to the moves that don't leave the mover's own king in check. See the
/// module doc for why copy-make (rather than pin/discovered-check detection)
/// is the right first version of this.
pub fn legal_moves(board: &Board) -> MoveList {
    let mut pl_moves = MoveList::default();
    pseudo_legal_moves(board, &mut pl_moves);

    // `side_to_move` before `board.make_move` flips it
    let color = board.side_to_move();

    let mut l_moves = MoveList::default();
    for &m in &pl_moves {
        if !in_check(&board.make_move(m), color) {
            l_moves.push(m);
        }
    }
    l_moves
}

/// The number of leaf positions reachable from `board` after exactly `depth`
/// plies of legal play. `perft(board, 0) == 1`. See the module doc for why
/// that stays a separate branch from the `depth == 1` bulk-counting shortcut.
pub fn perft(board: &Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = legal_moves(board);
    if depth == 1 {
        return moves.len() as u64;
    }
    moves
        .as_slice()
        .iter()
        .map(|&m| perft(&board.make_move(m), depth - 1))
        .sum()
}
