//! Tests for `board::zobrist`: does the incrementally maintained `Board::hash()`
//! agree with `compute_hash`'s from-scratch fold.
//!
//! `any_board()` builds positions through `place` and `from_parts`, never through
//! `make_move`, so most of the properties exercise the non-incremental
//! construction paths. `hash_stays_correct_after_a_legal_move` is the one property
//! that calls `make_move`, and is not `#[ignore]`d, so the default gate covers
//! incremental maintenance directly.
//!
//! Concrete tests pin facts worth not trusting by inspection (side to move, each
//! castling right alone, the en passant file), plus a perft-grade tree walk that
//! compares the two hashes at every node reachable from perft's six positions.
//! That walk is `#[ignore]`d and release-only, for the same reason as the deep
//! perft depths: it is a full tree walk, not a single check.

use proptest::prelude::*;
use turox_chess::board::zobrist::compute_hash;
use turox_chess::board::Board;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::strategies::{any_board, any_board_and_legal_move};
use turox_chess::{CastlingRights, Color};

#[test]
fn hash_differs_by_side_to_move_alone() {
    let blank = Board::default();
    let white_to_move = Board::from_parts(blank, Color::White, CastlingRights::NONE, None, 0, 1);
    let black_to_move = Board::from_parts(blank, Color::Black, CastlingRights::NONE, None, 0, 1);
    assert_ne!(white_to_move.hash(), black_to_move.hash());
}

#[test]
fn hash_differs_by_each_castling_right_alone() {
    let blank = Board::default();
    let none = Board::from_parts(blank, Color::White, CastlingRights::NONE, None, 0, 1);

    let rights = [
        CastlingRights::WHITE_KINGSIDE,
        CastlingRights::WHITE_QUEENSIDE,
        CastlingRights::BLACK_KINGSIDE,
        CastlingRights::BLACK_QUEENSIDE,
    ];
    let mut hashes = Vec::with_capacity(rights.len() + 1);
    hashes.push(none.hash());
    for right in rights {
        let board = Board::from_parts(blank, Color::White, right, None, 0, 1);
        hashes.push(board.hash());
    }

    for i in 0..hashes.len() {
        for j in (i + 1)..hashes.len() {
            assert_ne!(
                hashes[i], hashes[j],
                "castling-rights hash collision between entries {i} and {j} (0 = no rights, 1..4 = each right alone)"
            );
        }
    }
}

#[test]
fn hash_differs_by_en_passant_file_alone() {
    let board = Board::try_from_fen("8/8/8/3pP3/8/8/8/8 w - d6 0 1").expect("valid FEN");
    let no_ep = Board::try_from_fen("8/8/8/3pP3/8/8/8/8 w - - 0 1").expect("valid FEN");
    assert_ne!(board.hash(), no_ep.hash());
}

// ---- Perft-tree walk ----

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
const POSITION_5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
const POSITION_6: &str = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";

fn assert_hash_correct_at_every_node(board: &Board, depth: u32) {
    assert_eq!(
        board.hash(),
        compute_hash(board),
        "hash mismatch at depth {depth} for {board:?}"
    );
    if depth == 0 {
        return;
    }
    for &m in legal_moves(board).as_slice() {
        assert_hash_correct_at_every_node(&board.make_move(m), depth - 1);
    }
}

#[test]
#[ignore = "full tree walk; run with --release via --run-ignored all"]
fn hash_is_correct_at_every_node_of_the_perft_tree() {
    for fen in [
        STARTPOS, KIWIPETE, POSITION_3, POSITION_4, POSITION_5, POSITION_6,
    ] {
        let board = Board::try_from_fen(fen).expect("valid FEN");
        assert_hash_correct_at_every_node(&board, 4);
    }
}

proptest! {
    #[test]
    fn hash_matches_compute_hash_for_any_board(board in any_board()) {
        prop_assert_eq!(board.hash(), compute_hash(&board));
    }

    #[test]
    fn hash_survives_fen_round_trip(board in any_board()) {
        let parsed = Board::try_from_fen(&board.to_fen()).expect("to_fen output must parse");
        prop_assert_eq!(board.hash(), parsed.hash());
    }

    // Expected to fail until board::zobrist's documented make_move gap
    // (side to move, castling rights, en passant) is closed: this is the
    // first point in the file that actually calls `make_move`, the same
    // role `tests/legal_props.rs`'s own
    // `any_board_and_legal_move`-based test plays for move generation.
    #[test]
    fn hash_stays_correct_after_a_legal_move((board, m) in any_board_and_legal_move()) {
        let next = board.make_move(m);
        prop_assert_eq!(next.hash(), compute_hash(&next));
    }
}
