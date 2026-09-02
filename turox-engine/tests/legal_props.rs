//! Property tests for `move_gen::legal::legal_moves`.
//!
//! No independent reference generator here: `legal_moves`'s entire contract
//! *is* "pseudolegal moves filtered by post-move king safety," so there's no
//! second technique to check it against, only the definition itself. The
//! proptest below states that definition as two one-directional properties of
//! the actual returned `MoveList` (every legal move is safe; every dropped
//! pseudolegal move wasn't) rather than rebuilding a parallel "expected" list
//! by filtering `pseudo_legal_moves` with the same predicate `legal_moves`
//! itself applies; that filter-and-compare shape is close enough to a second
//! copy of `legal_moves`'s own loop that a shared mistake (e.g. filtering on
//! the wrong side's king) could pass both, since it's built from the exact
//! same primitives in the exact same order rather than checked as a property
//! of the output.
//!
//! Concrete tests (king moves into/out of check, pins, discovered checks,
//! the infamous en-passant-discovered-check position, check/stalemate) live
//! in `tests/legal.rs`, not here: this file is proptest only.

mod common;

use common::{any_board, any_board_and_legal_move};
use proptest::prelude::*;
use std::collections::HashSet;
use turox_engine::board::Board;
use turox_engine::move_gen::attacks::in_check;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::pseudo_legal_moves;
use turox_engine::{Move, MoveFlags, Square};

const fn move_key(m: Move) -> (u8, u8, MoveFlags) {
    (m.from().to_u8(), m.to().to_u8(), m.flags())
}

proptest! {
    #[test]
    fn every_legal_move_stays_safe_and_every_dropped_pseudolegal_move_does_not(board in any_board()) {
        let us = board.side_to_move();

        let mut pseudo = MoveList::new();
        pseudo_legal_moves(&board, &mut pseudo);
        let legal = legal_moves(&board);
        let legal_keys: HashSet<_> = legal.iter().map(|&m| move_key(m)).collect();

        // Soundness: nothing legal_moves returns leaves the mover in check.
        for &m in &legal {
            prop_assert!(
                !in_check(&board.make_move(m), us),
                "legal move {m:?} leaves the mover in check"
            );
        }

        // Completeness: nothing legal_moves dropped was actually safe.
        for &m in &pseudo {
            if !legal_keys.contains(&move_key(m)) {
                prop_assert!(
                    in_check(&board.make_move(m), us),
                    "pseudolegal move {m:?} was dropped but doesn't leave the mover in check"
                );
            }
        }
    }
}

// ---- make_move stays correct on genuinely reachable positions ----
//
// `board/mod.rs`'s own unit tests check `make_move` against hand-picked FEN
// scenarios; this is the first point in the crate where an arbitrary
// *legal* move is actually available, so it's the first point a proptest
// covering the same invariant makes sense.

/// Every (color, piece) bitboard pair is disjoint and their union is exactly
/// `occupied()`, and the mailbox agrees with the bitboards at every square.
/// Same invariant `board/mod.rs`'s own (private) `assert_board_is_internally_consistent`
/// checks; duplicated here rather than imported since integration tests only
/// see the crate's public API.
fn assert_internally_consistent(board: &Board) {
    use turox_engine::{Bitboard, Color, ColoredPiece, Piece};

    let mut union = Bitboard::EMPTY;
    for color in Color::ALL {
        for piece in Piece::ALL {
            let bb = board.pieces(color, piece);
            assert_eq!(
                bb.and(union),
                Bitboard::EMPTY,
                "overlap for {color:?}/{piece:?}"
            );
            union = union.or(bb);
        }
    }
    assert_eq!(union, board.occupied(), "bitboards don't cover occupied()");

    for sq in Square::ALL {
        let via_mailbox = board.piece_at(sq);
        let via_bitboards = Color::ALL.iter().find_map(|&color| {
            Piece::ALL
                .iter()
                .find(|&&piece| board.pieces(color, piece).contains(sq))
                .map(|&piece| ColoredPiece::new(color, piece))
        });
        assert_eq!(
            via_mailbox, via_bitboards,
            "mailbox/bitboard mismatch at {sq:?}"
        );
    }
}

proptest! {
    #[test]
    fn make_move_after_a_legal_move_stays_internally_consistent_and_fen_round_trips(
        (board, m) in any_board_and_legal_move()
    ) {
        let next = board.make_move(m);
        assert_internally_consistent(&next);

        let fen = next.to_fen();
        let parsed = Board::try_from_fen(&fen).expect("to_fen output must parse");
        prop_assert_eq!(next, parsed);
    }
}
