//! `Search::search` throughput: nodes/sec at a fixed depth on the same two
//! branchy standard positions `benches/perft.rs` uses, so pruning quality
//! and move-ordering changes have a number to be measured against, the same
//! role `benches/perft.rs` plays for move generation itself.
//!
//! Depth 6, not the original 4: depth 4 turned out too shallow for a
//! transposition table's own effect to clear the noise floor (too few
//! transpositions in a 4-ply tree from either position for probe/store
//! overhead to pay for itself), confirmed by comparing against depth 4 on
//! `main` directly rather than assumed. 6 still keeps a single Criterion
//! sample in the low seconds.
//!
//! Each iteration gets its own `Tt`, cleared before the timed closure runs
//! (`tt.clear()` is outside `b.iter`'s own timing, same as `board` itself),
//! so what's measured is the benefit iterative deepening's own shallower
//! passes (depth 1..depth) get from populating the table within *this one*
//! `search` call, not cross-call reuse across iterations, which a real game
//! benefits from too but this bench isn't shaped to isolate.
//!
//! Unlike `perft`'s node counts, `Search`'s node count isn't a fixed ground
//! truth (it depends on pruning and move-ordering quality, which are
//! expected to change), so throughput is measured against whatever node
//! count this run's own search actually reports, not a hardcoded constant.

#![allow(missing_docs, reason = "bench binaries aren't a public API surface")]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use turox_engine::board::Board;
use turox_engine::search::tt::Tt;
use turox_engine::search::Search;

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

fn bench_search(c: &mut Criterion, name: &str, fen: &str, depth: u8) {
    let board = Board::try_from_fen(fen).expect("valid FEN");
    let mut tt = Tt::new(Tt::DEFAULT_HASH_MB);
    let nodes = Search::new(Vec::new())
        .with_tt(&mut tt)
        .search(&board, depth)
        .nodes;

    let mut group = c.benchmark_group("search");
    group.throughput(Throughput::Elements(nodes));
    group.sample_size(10);
    group.bench_function(name, |b| {
        b.iter(|| {
            tt.clear();
            Search::new(Vec::new())
                .with_tt(&mut tt)
                .search(&board, depth)
        });
    });
    group.finish();
}

fn startpos_depth_6(c: &mut Criterion) {
    bench_search(c, "startpos_depth_6", STARTPOS, 6);
}

fn kiwipete_depth_6(c: &mut Criterion) {
    bench_search(c, "kiwipete_depth_6", KIWIPETE, 6);
}

criterion_group!(benches, startpos_depth_6, kiwipete_depth_6);
criterion_main!(benches);
