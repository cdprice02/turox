//! Pin- and check-aware legal move generation.
//!
//! Plus `perft`, the standard recursive node-count benchmark that doubles as the
//! practical correctness gate on `pseudo_legal`, `attacks`, and `Board::make_move`
//! together.

use crate::board::Board;
use crate::move_gen::attacks::in_check;
use crate::move_gen::move_list::MoveList;
use crate::move_gen::pseudo_legal::pseudo_legal_moves;

/// Every legal move for `board.side_to_move()`: `pseudo_legal_moves` filtered in place,
/// via `MoveList::retain`, to the moves that don't leave the mover's own king in check.
///
/// No separate pin detection, no discovered-check bookkeeping. `board.make_move(m)`
/// produces the actual resulting position and `in_check` actually re-scans it, so pins,
/// discovered checks, and en-passant-discovered checks along the capturing pawn's rank
/// all fall out for free. This is still slower than the pin-aware bitboard techniques a
/// faster engine wants (a full `make_move` per candidate, rather than a cheap precomputed
/// pin-mask test), but it's obviously correct by construction. Perft is what proves that,
/// and what `benches/move_gen.rs`/`benches/perft.rs` benchmark a cleverer version
/// against, if one ever replaces this.
#[must_use]
pub fn legal_moves(board: &Board) -> MoveList {
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
///
/// # Panics
///
/// Never in practice: a legal move count exceeding `u64::MAX` isn't a real chess
/// position.
#[must_use]
pub fn perft(board: &Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = legal_moves(board);
    if depth == 1 {
        return u64::try_from(moves.len())
            .expect("move count fits u64 (MoveList::CAPACITY < u64::MAX)");
    }
    moves
        .as_slice()
        .iter()
        .map(|&m| perft(&board.make_move(m), depth - 1))
        .sum()
}
