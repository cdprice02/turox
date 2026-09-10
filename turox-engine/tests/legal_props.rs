//! Property tests for `move_gen::legal::legal_moves` and its naive reference,
//! `legal_moves_naive`.
//!
//! `legal_moves_naive`'s entire contract *is* "pseudolegal moves filtered by
//! post-move king safety," so there's no independent technique to check it
//! against beyond the definition itself: the first proptest below states
//! that definition as two one-directional properties of its actual returned
//! `MoveList` (every legal move is safe; every dropped pseudolegal move
//! wasn't), rather than rebuilding a parallel "expected" list by filtering
//! `pseudo_legal_moves` with the same predicate `legal_moves_naive` itself
//! applies; that filter-and-compare shape is close enough to a second copy
//! of its own loop that a shared mistake (e.g. filtering on the wrong side's
//! king) could pass both, since it's built from the exact same primitives in
//! the exact same order rather than checked as a property of the output.
//!
//! `legal_moves` (the pin-set fast path, #113) is checked against
//! `legal_moves_naive` directly in the second proptest: now that a second,
//! independently-built technique exists, exact set agreement is the
//! stronger and more direct property, and the one every caller elsewhere in
//! the crate actually depends on.
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
use turox_engine::move_gen::legal::{legal_moves, legal_moves_naive};
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::pseudo_legal_moves;
use turox_engine::{Move, MoveFlags, Square};

const fn move_key(m: Move) -> (u8, u8, MoveFlags) {
    (m.from().to_u8(), m.to().to_u8(), m.flags())
}

fn keys(list: &MoveList) -> HashSet<(u8, u8, MoveFlags)> {
    list.iter().map(|&m| move_key(m)).collect()
}

proptest! {
    #[test]
    fn every_naive_legal_move_stays_safe_and_every_dropped_pseudolegal_move_does_not(board in any_board()) {
        let us = board.side_to_move();

        let mut pseudo = MoveList::new();
        pseudo_legal_moves(&board, &mut pseudo);
        let legal = legal_moves_naive(&board);
        let legal_keys = keys(&legal);

        // Soundness: nothing legal_moves_naive returns leaves the mover in check.
        for &m in &legal {
            prop_assert!(
                !in_check(&board.make_move(m), us),
                "legal move {m:?} leaves the mover in check"
            );
        }

        // Completeness: nothing legal_moves_naive dropped was actually safe.
        for &m in &pseudo {
            if !legal_keys.contains(&move_key(m)) {
                prop_assert!(
                    in_check(&board.make_move(m), us),
                    "pseudolegal move {m:?} was dropped but doesn't leave the mover in check"
                );
            }
        }
    }

    #[test]
    fn pin_set_legal_moves_agrees_with_the_naive_reference(board in any_board()) {
        prop_assert_eq!(
            keys(&legal_moves(&board)),
            keys(&legal_moves_naive(&board)),
            "pin-set legal_moves disagrees with legal_moves_naive on {:?}", board
        );
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
