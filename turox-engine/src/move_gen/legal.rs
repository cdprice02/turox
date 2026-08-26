//! Pin- and check-aware legal move generation, plus `perft`, the standard
//! recursive node-count benchmark that doubles as the practical correctness
//! gate on `pseudo_legal`, `attacks`, and `Board::make_move` together.
//!
//! # `legal_moves`: copy-make, not a pin/check detector
//!
//! `legal_moves` filters `pseudo_legal::pseudo_legal_moves` by
//! `!attacks::in_check(&board.make_move(m), us)` — no separate pin detection,
//! no discovered-check bookkeeping. `board.make_move(m)` produces the actual
//! resulting position and `attacks::in_check` actually re-scans it, so pins,
//! discovered checks, and en-passant-discovered checks along the capturing
//! pawn's rank all fall out for free. This is slower than the pin-aware
//! bitboard techniques a faster engine wants (a full `make_move` plus a full
//! `attacked_by` scan per candidate, rather than a cheap pin-mask check), but
//! it's obviously correct by construction — perft is what proves that, and
//! what a later perf pass would benchmark a cleverer version against.
//!
//! # `perft`
//!
//! `depth == 0` stays its own branch rather than falling out of the
//! `depth == 1` bulk-counting shortcut below it: `perft` is called directly
//! with `depth == 0` (`perft_zero_is_one_leaf`, and indirectly any depth-1
//! call's own base case), and without that branch `depth - 1` underflows
//! `u32` before reaching the `depth == 1` check.

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
