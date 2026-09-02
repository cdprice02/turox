//! Concrete tests for `board::zobrist`.
//!
//! `tests/zobrist_props.rs` has the property coverage over arbitrary boards;
//! these are facts worth pinning down concretely rather than trusting by
//! inspection (side to move, each castling right, en passant file), plus the
//! full perft-tree walk: `board.hash()` (incremental) matches `compute_hash`
//! (from-scratch) at every node reachable within `depth` plies, exercising
//! the incremental update across every move type perft's own six positions
//! are chosen to cover. `#[ignore]`d and release-only, same reasoning as the
//! deep perft depths in `tests/perft.rs`: this is a full tree walk, not a
//! single check.

use turox_engine::board::zobrist::compute_hash;
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::{CastlingRights, Color};

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
