//! Micro-benchmarks for FEN parsing and formatting.
//!
//! `parse_*` benchmarks will panic (via `todo!()` in `Bitboard`'s core arithmetic,
//! which `Board::place` depends on) until that part of the exercise is done; see
//! `benches/bitboard.rs` for why. `format_start_pos` has the same dependency
//! indirectly, since it first needs a populated `Board` to format.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use turox_engine::board::Board;

/// A handful of representative positions rather than just the start position, so
/// the benchmark reflects FEN strings of varying density (empty ranks, few pieces)
/// and not just the maximally-full starting rank pattern.
const POSITIONS: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    "8/8/8/4k3/8/8/4K3/8 w - - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
];

fn parse(c: &mut Criterion) {
    c.bench_function("fen_parse", |b| {
        b.iter(|| {
            for &fen in POSITIONS {
                black_box(Board::try_from_fen(black_box(fen)).expect("valid FEN"));
            }
        });
    });
}

fn format(c: &mut Criterion) {
    let boards: Vec<Board> = POSITIONS
        .iter()
        .map(|fen| Board::try_from_fen(fen).expect("valid FEN"))
        .collect();

    c.bench_function("fen_format", |b| {
        b.iter(|| {
            for board in &boards {
                black_box(black_box(board).to_fen());
            }
        });
    });
}

criterion_group!(benches, parse, format);
criterion_main!(benches);
