//! Micro-benchmarks for `Bitboard`'s operations.
//!
//! The trap this file is written to avoid: these operations inline to a handful of
//! instructions, so `b.iter(|| bb.flip_vertical())` over a *constant* input gets
//! const-folded away by LLVM and reports ~0ns, which would look like "the newtype
//! is free" for entirely the wrong reason (nothing actually ran). Every benchmark
//! here instead drives over a precomputed array of varied inputs and
//! `black_box`es both the input and the returned value, so the compiler can't
//! predict or eliminate the work.

// Not part of the crate's public API, so `missing_docs` doesn't apply here:
// criterion's own `criterion_group!`/`criterion_main!` macros generate an
// undocumented `fn main`.
#![allow(missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use turox_engine::{Bitboard, Direction};

const SAMPLE_COUNT: usize = 1024;

/// A small, deterministic, seedable PRNG (xorshift64) instead of a `rand`
/// dependency: the engine takes zero runtime deps, and reproducible benchmark
/// inputs are arguably better anyway (no dependency on `rand`'s default source
/// changing between runs or platforms).
struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// `SAMPLE_COUNT` varied, non-trivial bitboards (not `EMPTY`/`ALL`-heavy) to
/// benchmark over.
fn sample_bitboards() -> Vec<Bitboard> {
    let mut rng = XorShift64(0x9E37_79B9_7F4A_7C15); // arbitrary nonzero seed
    (0..SAMPLE_COUNT)
        .map(|_| Bitboard::from_bits(rng.next()))
        .collect()
}

fn bench_transform(c: &mut Criterion, name: &str, f: impl Fn(Bitboard) -> Bitboard) {
    let samples = sample_bitboards();
    let mut group = c.benchmark_group("bitboard");
    group.throughput(Throughput::Elements(samples.len() as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            for &bb in &samples {
                black_box(f(black_box(bb)));
            }
        });
    });
    group.finish();
}

fn flips_and_rotations(c: &mut Criterion) {
    bench_transform(c, "flip_horizontal", Bitboard::flip_horizontal);
    bench_transform(c, "flip_vertical", Bitboard::flip_vertical);
    bench_transform(c, "flip_diagonal_a1h8", Bitboard::flip_diagonal_a1h8);
    bench_transform(c, "flip_diagonal_a8h1", Bitboard::flip_diagonal_a8h1);
    bench_transform(c, "rotate_90_cw", Bitboard::rotate_90_cw);
    bench_transform(c, "rotate_90_ccw", Bitboard::rotate_90_ccw);
    bench_transform(c, "rotate_180", Bitboard::rotate_180);
}

fn scanning(c: &mut Criterion) {
    let samples = sample_bitboards();
    let mut group = c.benchmark_group("bitboard");
    group.throughput(Throughput::Elements(samples.len() as u64));

    group.bench_function("count", |b| {
        b.iter(|| {
            for &bb in &samples {
                black_box(black_box(bb).count());
            }
        });
    });

    group.bench_function("pop_lsb_full_drain", |b| {
        b.iter(|| {
            for &bb in &samples {
                let mut bb = black_box(bb);
                while let Some(sq) = bb.pop_lsb() {
                    black_box(sq);
                }
            }
        });
    });

    group.finish();
}

fn shifts(c: &mut Criterion) {
    let samples = sample_bitboards();
    let mut group = c.benchmark_group("bitboard");
    group.throughput(Throughput::Elements(samples.len() as u64));

    for dir in Direction::ALL {
        group.bench_function(format!("shift_{dir:?}"), |b| {
            b.iter(|| {
                for &bb in &samples {
                    black_box(black_box(bb).shift(dir));
                }
            });
        });
    }

    group.finish();
}

fn iteration(c: &mut Criterion) {
    // A start-position-like occupancy (32 scattered bits) rather than one constant
    // value, so branch/value prediction across the iteration loop doesn't flatter
    // the number.
    let samples: Vec<Bitboard> = sample_bitboards()
        .into_iter()
        .map(|bb| Bitboard::from_bits(bb.bits() & 0xFFFF_0000_0000_FFFF))
        .collect();

    c.bench_function("bitboard_full_iteration", |b| {
        b.iter(|| {
            for &bb in &samples {
                for sq in black_box(bb) {
                    black_box(sq);
                }
            }
        });
    });
}

fn subsets(c: &mut Criterion) {
    // A fixed 10-bit mask: enough subsets (1024) to be a meaningful workload
    // without making the benchmark itself slow to run.
    let mask = Bitboard::from_bits(0x0000_0000_03FF_0000);
    c.bench_function("bitboard_subsets_10_bits", |b| {
        b.iter(|| {
            for subset in black_box(mask).subsets() {
                black_box(subset);
            }
        });
    });
}

criterion_group!(
    benches,
    flips_and_rotations,
    scanning,
    shifts,
    iteration,
    subsets
);
criterion_main!(benches);
