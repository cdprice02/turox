//! Micro-benchmark for `eval::evaluate`: the score negamax will call at
//! every search leaf, so this is the baseline the incremental material/PST
//! accumulator (tracked separately) gets measured against once it exists.
//!
//! Same anti-const-folding shape as `benches/move_gen.rs`: drives over a
//! precomputed corpus and `black_box`es both the input and the returned
//! score.

#![allow(missing_docs, reason = "bench binaries aren't a public API surface")]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use turox_engine::board::Board;
use turox_engine::eval::evaluate;
use turox_engine::move_gen::legal::legal_moves;

// Same six standard perft positions as benches/move_gen.rs and
// tests/perft.rs; duplicated rather than shared, since a bench compiles as
// its own binary and only sees `pub` API.
const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
const POSITION_5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
const POSITION_6: &str = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";

/// Same corpus shape as `benches/move_gen.rs`'s `sample_boards`: the six
/// base positions plus, for each, its first four legal children.
fn sample_boards() -> Vec<Board> {
    let bases = [
        STARTPOS, KIWIPETE, POSITION_3, POSITION_4, POSITION_5, POSITION_6,
    ];
    let mut boards = Vec::new();
    for fen in bases {
        let board = Board::try_from_fen(fen).expect("valid FEN");
        let children = legal_moves(&board);
        boards.extend(children.iter().take(4).map(|&m| board.make_move(m)));
        boards.push(board);
    }
    boards
}

fn eval_bench(c: &mut Criterion) {
    let boards = sample_boards();
    let mut group = c.benchmark_group("eval");
    group.throughput(Throughput::Elements(
        u64::try_from(boards.len()).expect("fits u64"),
    ));
    group.bench_function("evaluate", |b| {
        b.iter(|| {
            for board in &boards {
                black_box(evaluate(black_box(board)));
            }
        });
    });
    group.finish();
}

criterion_group!(benches, eval_bench);
criterion_main!(benches);
