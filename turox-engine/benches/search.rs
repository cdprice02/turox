//! `Search::search` throughput: nodes/sec at a fixed depth on the same two
//! branchy standard positions `benches/perft.rs` uses, so pruning quality
//! and move-ordering changes have a number to be measured against, the same
//! role `benches/perft.rs` plays for move generation itself.
//!
//! Depth chosen low enough (4 plies) to keep a single Criterion sample in
//! the low seconds even before a transposition table exists to cut the
//! tree down further; deeper depths are a later, deliberate rebaseline
//! once that lands, not something to guess ahead of time here.
//!
//! Unlike `perft`'s node counts, `Search`'s node count isn't a fixed ground
//! truth (it depends on pruning and move-ordering quality, which are
//! expected to change), so throughput is measured against whatever node
//! count this run's own search actually reports, not a hardcoded constant.

#![allow(missing_docs, reason = "bench binaries aren't a public API surface")]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use turox_engine::board::Board;
use turox_engine::search::Search;

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

fn bench_search(c: &mut Criterion, name: &str, fen: &str, depth: u32) {
    let board = Board::try_from_fen(fen).expect("valid FEN");
    let nodes = Search::new(Vec::new()).search(&board, depth).nodes;

    let mut group = c.benchmark_group("search");
    group.throughput(Throughput::Elements(nodes));
    group.sample_size(10);
    group.bench_function(name, |b| {
        b.iter(|| Search::new(Vec::new()).search(&board, depth));
    });
    group.finish();
}

fn startpos_depth_4(c: &mut Criterion) {
    bench_search(c, "startpos_depth_4", STARTPOS, 4);
}

fn kiwipete_depth_4(c: &mut Criterion) {
    bench_search(c, "kiwipete_depth_4", KIWIPETE, 4);
}

criterion_group!(benches, startpos_depth_4, kiwipete_depth_4);
criterion_main!(benches);
