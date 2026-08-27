//! `perft` throughput: the number that actually matters and the one
//! comparable across engines, since it's node count divided by wall time
//! rather than the per-call microbenchmarks `benches/move_gen.rs` reports.
//!
//! Depths chosen to land in the low millions of nodes (`startpos` depth 5,
//! ~4.9M; `kiwipete` depth 4, ~4.1M; same depths as `tests/perft.rs`'s
//! `#[ignore]`d cases), so a single Criterion sample is seconds, not the
//! tens of minutes a depth-6+ perft would take.
//!
//! Measured on this machine (release), before/after the `in_check` and
//! `legal_moves` changes in this same PR (both positions here load via
//! `try_from_fen`, so `Board::start_pos`'s own const-ification, also in this
//! PR, isn't exercised by this particular benchmark): `startpos_depth_5`
//! 10.9M -> 21.5M nodes/sec, `kiwipete_depth_4` 11.0M -> 22.9M nodes/sec, both
//! roughly doubled, confirmed by Criterion's own before/after comparison
//! against a saved baseline, not just the raw numbers.

// Not part of the crate's public API, so `missing_docs` doesn't apply here:
// criterion's own `criterion_group!`/`criterion_main!` macros generate an
// undocumented `fn main`.
#![allow(missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use turox_engine::board::Board;
use turox_engine::move_gen::legal::perft;

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

fn bench_perft(c: &mut Criterion, name: &str, fen: &str, depth: u32, nodes: u64) {
    let board = Board::try_from_fen(fen).expect("valid FEN");
    let mut group = c.benchmark_group("perft");
    group.throughput(Throughput::Elements(nodes));
    group.sample_size(10);
    group.bench_function(name, |b| {
        b.iter(|| perft(&board, depth));
    });
    group.finish();
}

fn startpos_depth_5(c: &mut Criterion) {
    bench_perft(c, "startpos_depth_5", STARTPOS, 5, 4_865_609);
}

fn kiwipete_depth_4(c: &mut Criterion) {
    bench_perft(c, "kiwipete_depth_4", KIWIPETE, 4, 4_085_603);
}

criterion_group!(benches, startpos_depth_5, kiwipete_depth_4);
criterion_main!(benches);
