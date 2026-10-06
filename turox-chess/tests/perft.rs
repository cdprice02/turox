//! `perft` against the six standard test positions (see the README's
//! "References" section), the end-to-end correctness gate for move
//! generation: it exercises `legal_moves`, `pseudo_legal`, `attacks`, `MoveList`,
//! `Board::make_move`, `tables`, and `magic` together. A wrong count at low
//! depth on any position localizes to a specific rule; matching all six
//! (including the deep, `#[ignore]`d depths) is the point at which move
//! generation is actually done.
//!
//! `perft` counts the last ply's moves rather than making them, so it runs at
//! tens of millions of nodes a second even in the `dev` profile CI uses
//! (`opt-level = 1`; see the workspace `Cargo.toml`). Depths up to about 16M
//! nodes take under half a second there and run on every push. The depths of
//! 90M nodes and more are `#[ignore]`d: a few seconds each in `--release`, run
//! with `cargo nextest run --workspace --run-ignored all --release`.

#![expect(
    clippy::expect_used,
    reason = "`clippy.toml`'s allow-expect-in-tests reaches `#[test]` functions and `#[cfg(test)]` modules, but not plain helpers in an integration test or bench, where a failed fixture should abort the run"
)]

use turox_chess::board::Board;
use turox_chess::move_gen::legal::{legal_moves, perft};

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
const POSITION_5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
const POSITION_6: &str = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";

fn board(fen: &str) -> Board {
    Board::try_from_fen(fen).expect("valid FEN")
}

// ---- startpos ----

#[test]
fn startpos_perft_1() {
    assert_eq!(perft(&board(STARTPOS), 1), 20);
}

#[test]
fn startpos_perft_2() {
    assert_eq!(perft(&board(STARTPOS), 2), 400);
}

#[test]
fn startpos_perft_3() {
    assert_eq!(perft(&board(STARTPOS), 3), 8_902);
}

#[test]
fn startpos_perft_4() {
    assert_eq!(perft(&board(STARTPOS), 4), 197_281);
}

#[test]
fn startpos_perft_5() {
    assert_eq!(perft(&board(STARTPOS), 5), 4_865_609);
}

#[test]
#[ignore = "~119M nodes, about 2.5s in --release; run with --run-ignored all"]
fn startpos_perft_6() {
    assert_eq!(perft(&board(STARTPOS), 6), 119_060_324);
}

// ---- Kiwipete ----

#[test]
fn kiwipete_perft_1() {
    assert_eq!(perft(&board(KIWIPETE), 1), 48);
}

#[test]
fn kiwipete_perft_2() {
    assert_eq!(perft(&board(KIWIPETE), 2), 2_039);
}

#[test]
fn kiwipete_perft_3() {
    assert_eq!(perft(&board(KIWIPETE), 3), 97_862);
}

#[test]
fn kiwipete_perft_4() {
    assert_eq!(perft(&board(KIWIPETE), 4), 4_085_603);
}

#[test]
#[ignore = "~194M nodes, about 2.5s in --release; run with --run-ignored all"]
fn kiwipete_perft_5() {
    assert_eq!(perft(&board(KIWIPETE), 5), 193_690_690);
}

// ---- Position 3 ----

#[test]
fn position_3_perft_1() {
    assert_eq!(perft(&board(POSITION_3), 1), 14);
}

#[test]
fn position_3_perft_2() {
    assert_eq!(perft(&board(POSITION_3), 2), 191);
}

#[test]
fn position_3_perft_3() {
    assert_eq!(perft(&board(POSITION_3), 3), 2_812);
}

#[test]
fn position_3_perft_4() {
    assert_eq!(perft(&board(POSITION_3), 4), 43_238);
}

#[test]
fn position_3_perft_5() {
    assert_eq!(perft(&board(POSITION_3), 5), 674_624);
}

#[test]
fn position_3_perft_6() {
    assert_eq!(perft(&board(POSITION_3), 6), 11_030_083);
}

#[test]
#[ignore = "~179M nodes, about 3.5s in --release; run with --run-ignored all"]
fn position_3_perft_7() {
    assert_eq!(perft(&board(POSITION_3), 7), 178_633_661);
}

// ---- Position 4 ----

#[test]
fn position_4_perft_1() {
    assert_eq!(perft(&board(POSITION_4), 1), 6);
}

#[test]
fn position_4_perft_2() {
    assert_eq!(perft(&board(POSITION_4), 2), 264);
}

#[test]
fn position_4_perft_3() {
    assert_eq!(perft(&board(POSITION_4), 3), 9_467);
}

#[test]
fn position_4_perft_4() {
    assert_eq!(perft(&board(POSITION_4), 4), 422_333);
}

#[test]
fn position_4_perft_5() {
    assert_eq!(perft(&board(POSITION_4), 5), 15_833_292);
}

// ---- Position 5 ----

#[test]
fn position_5_perft_1() {
    assert_eq!(perft(&board(POSITION_5), 1), 44);
}

#[test]
fn position_5_perft_2() {
    assert_eq!(perft(&board(POSITION_5), 2), 1_486);
}

#[test]
fn position_5_perft_3() {
    assert_eq!(perft(&board(POSITION_5), 3), 62_379);
}

#[test]
fn position_5_perft_4() {
    assert_eq!(perft(&board(POSITION_5), 4), 2_103_487);
}

#[test]
#[ignore = "~90M nodes, about 1.5s in --release; run with --run-ignored all"]
fn position_5_perft_5() {
    assert_eq!(perft(&board(POSITION_5), 5), 89_941_194);
}

// ---- Position 6 ----

#[test]
fn position_6_perft_1() {
    assert_eq!(perft(&board(POSITION_6), 1), 46);
}

#[test]
fn position_6_perft_2() {
    assert_eq!(perft(&board(POSITION_6), 2), 2_079);
}

#[test]
fn position_6_perft_3() {
    assert_eq!(perft(&board(POSITION_6), 3), 89_890);
}

#[test]
fn position_6_perft_4() {
    assert_eq!(perft(&board(POSITION_6), 4), 3_894_594);
}

#[test]
#[ignore = "~164M nodes, about 2.5s in --release; run with --run-ignored all"]
fn position_6_perft_5() {
    assert_eq!(perft(&board(POSITION_6), 5), 164_075_551);
}

// ---- perft(0) ----

#[test]
fn perft_zero_is_one_leaf() {
    assert_eq!(perft(&board(STARTPOS), 0), 1);
}

// ---- perft_divide ----
//
// Per-root-move node counts. Not called by any test above; it earns its
// place the moment one of the counts above is ever wrong and needs
// localizing to a specific root move, which is exactly what a raw total
// can't tell you.
#[expect(
    dead_code,
    reason = "kept compiling and ready for the moment a perft count is wrong and needs localizing to a root move"
)]
fn perft_divide(board: &Board, depth: u32) -> Vec<(String, u64)> {
    legal_moves(board)
        .iter()
        .map(|&m| {
            let nodes = perft(&board.make_move(m), depth.saturating_sub(1));
            (format!("{m:?}"), nodes)
        })
        .collect()
}
