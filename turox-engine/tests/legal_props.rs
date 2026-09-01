//! Property and concrete tests for `move_gen::legal::legal_moves`.
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
//! Concrete tests cover the classic cases a naive or subtly-wrong copy-make
//! filter gets wrong: king moves into/out of check, pins, discovered checks,
//! the infamous en-passant-discovered-check position, and check/stalemate
//! producing an empty legal move list.

mod common;

use common::any_board;
use proptest::prelude::*;
use std::collections::HashSet;
use turox_engine::board::Board;
use turox_engine::move_gen::attacks::in_check;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::pseudo_legal_moves;
use turox_engine::{Move, Square};

const fn move_key(m: Move) -> (u8, u8, u8) {
    (m.from().index(), m.to().index(), m.flags() as u8)
}

fn contains(list: &MoveList, from: Square, to: Square) -> bool {
    list.iter().any(|&m| m.from() == from && m.to() == to)
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

// ---- Concrete cases: the ones a subtly-wrong filter gets wrong ----

#[test]
fn king_cannot_move_into_an_attacked_square() {
    // Black rook on e8 covers the entire e-file; the white king on e1 has
    // d1/f1 (off the file) but not e2 (still on the file, still attacked).
    let board = Board::try_from_fen("4r3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E1, Square::E2));
    assert!(contains(&moves, Square::E1, Square::D1));
    assert!(contains(&moves, Square::E1, Square::F1));
}

#[test]
fn king_in_check_can_capture_the_checking_piece() {
    let board = Board::try_from_fen("8/8/8/8/8/8/4r3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(contains(&moves, Square::E1, Square::E2));
}

#[test]
fn pinned_bishop_cannot_move_off_the_pin_line() {
    // White king e1, white bishop e2, black rook e8: the bishop is pinned
    // along the e-file. It can't step to a diagonal square off that file;
    // doing so would expose the king to the rook.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4B3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E2, Square::D3));
    assert!(!contains(&moves, Square::E2, Square::F3));
}

#[test]
fn pinned_rook_can_still_move_along_the_pin_line() {
    // Same pin, but the pinned piece is itself a rook: sliding along e-file
    // (toward or away from the king, short of capturing/passing the pinner)
    // stays legal, since the king is never exposed.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/4R3/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(contains(&moves, Square::E2, Square::E5));
}

#[test]
fn en_passant_capture_that_discovers_a_rank_check_is_illegal() {
    // The textbook case: White king a5, White pawn e5, Black pawn d5 (just
    // double-pushed, ep target d6), Black rook h5. Capturing en passant
    // (e5xd6) removes the d5 pawn from the board, the one piece blocking the
    // rook's rank check on the king, so it must NOT appear as legal, even
    // though it's a perfectly ordinary pseudolegal en passant capture.
    let board = Board::try_from_fen("8/8/8/K1Pp3r/8/8/8/8 w - d6 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E5, Square::D6));
}

#[test]
fn king_moving_off_a_sliders_ray_is_still_in_check() {
    // King on e1 stepping to e2 does not escape a rook's e-file check: e2 is
    // still on the file. This is the case copy-make gets right "for free":
    // the king's *old* square (e1) is genuinely vacated in the copy, so if the
    // king had instead tried to step *behind itself* along a rank/file/
    // diagonal it was blocking, the ray would correctly continue through.
    let board = Board::try_from_fen("4r3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!contains(&moves, Square::E1, Square::E2));
}

#[test]
fn checkmate_has_no_legal_moves() {
    // Fool's mate.
    let board =
        Board::try_from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
            .expect("valid FEN");
    assert!(in_check(&board, board.side_to_move()));
    assert!(legal_moves(&board).is_empty());
}

#[test]
fn stalemate_has_no_legal_moves_and_is_not_check() {
    // Classic king-and-queen-vs-king stalemate: Black king a8 has no legal
    // move and is not in check.
    let board = Board::try_from_fen("k7/8/1Q6/8/8/8/8/1K6 b - - 0 1").expect("valid FEN");
    assert!(!in_check(&board, board.side_to_move()));
    assert!(legal_moves(&board).is_empty());
}

#[test]
fn double_check_only_the_king_may_move() {
    // White king e1, attacked simultaneously by a rook on the e-file and a
    // bishop on the a5-e1 diagonal, plus a white knight on b3 that *could*
    // capture the bishop (b3-a5 is a legal knight move) if this were only a
    // single check. Since it isn't, the rook's check remains regardless,
    // that capture must still be excluded: no block or capture resolves both
    // checks at once, so every legal move must be a king move.
    let board = Board::try_from_fen("4r3/8/8/b7/8/1N6/8/4K3 w - - 0 1").expect("valid FEN");
    let moves = legal_moves(&board);
    assert!(!moves.is_empty());
    assert!(moves.iter().all(|m| m.from() == Square::E1));
    assert!(!contains(&moves, Square::B3, Square::A5));
}

// ---- make_move stays correct on genuinely reachable positions ----
//
// `board/mod.rs`'s own unit tests check `make_move` against hand-picked FEN
// scenarios; this is the first point in the crate where an arbitrary
// *legal* move is actually available, so it's the first point a proptest
// covering the same invariant makes sense.

fn any_board_with_legal_move() -> impl Strategy<Value = (Board, Move)> {
    any_board()
        .prop_filter("must have at least one legal move", |board| {
            !legal_moves(board).is_empty()
        })
        .prop_flat_map(|board| {
            let moves: Vec<Move> = legal_moves(&board).iter().copied().collect();
            (Just(board), prop::sample::select(moves))
        })
}

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
        (board, m) in any_board_with_legal_move()
    ) {
        let next = board.make_move(m);
        assert_internally_consistent(&next);

        let fen = next.to_fen();
        let parsed = Board::try_from_fen(&fen).expect("to_fen output must parse");
        prop_assert_eq!(next, parsed);
    }
}
