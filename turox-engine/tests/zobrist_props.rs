//! Property tests for `board::zobrist`: does the incrementally-maintained
//! `Board::hash()` agree with `zobrist::compute_hash`'s from-scratch fold.
//!
//! Unlike `eval`, this stage has a genuine perft-grade ground truth: the
//! `#[ignore]`d perft-tree walk at the bottom recurses the real legal-move
//! tree and checks every node, the same discipline `tests/perft.rs` uses
//! for move generation itself.
//!
//! `any_board()` builds positions through `place`/`from_parts` only, never
//! through `make_move`, so the plain property tests below (matching the
//! naive reference, surviving a FEN round-trip) exercise the non-incremental
//! construction paths, not `make_move`'s own hash-maintenance.
//! `hash_stays_correct_after_a_legal_move` is the one test in this file
//! that actually calls `make_move`, deliberately not `#[ignore]`d so the
//! default `cargo nextest run --workspace` gate covers it directly rather
//! than relying only on the expensive release-only perft-tree walk below.

mod common;

use common::any_board;
use proptest::prelude::*;
use turox_engine::board::zobrist::compute_hash;
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::{CastlingRights, Color, Move};

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
    // `any_board_with_legal_move`-based test plays for move generation.
    #[test]
    fn hash_stays_correct_after_a_legal_move((board, m) in any_board_with_legal_move()) {
        let next = board.make_move(m);
        prop_assert_eq!(next.hash(), compute_hash(&next));
    }
}

// ---- Concrete asymmetric tests ----
//
// Side to move and castling rights are exactly the `{Color}x{state}`-shaped
// facts this project's own history says are worth pinning down concretely
// rather than trusting by inspection.

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
//
// `board.hash()` (incremental) matches `compute_hash` (from-scratch) at
// every node reachable within `depth` plies, exercising the incremental
// update across every move type perft's own six positions are chosen to
// cover: castling, en passant, promotion, and heavy middlegame branching.
// `#[ignore]`d and release-only, same reasoning as the deep perft depths in
// `tests/perft.rs`: this is a full tree walk, not a single check.

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
