//! Micro-benchmarks for `move_gen`'s generators: per-piece pseudolegal
//! generation, the combined `pseudo_legal_moves`, and the check-filtered
//! `legal_moves` on top of it.
//!
//! The gap between `pseudo_legal_moves` and `legal_moves` on the same corpus
//! is the cost of the check filter itself (one `make_move` + `in_check` per
//! pseudolegal move). Before routing `in_check` through `attackers_of` instead
//! of a full `attacked_by` scan, and filtering `legal_moves` in place instead
//! of copying into a second `MoveList`, that gap measured `legal_moves` at
//! 80.3µs against `pseudo_legal_moves`'s 6.3µs on this corpus; after both
//! changes, `legal_moves` measures 43.6-46.9µs, a 42-45% drop, confirmed by
//! Criterion's own before/after comparison against a saved baseline.
//!
//! Same anti-const-folding shape as `benches/bitboard.rs`/`benches/magic.rs`:
//! every benchmark drives over a precomputed corpus and `black_box`es both
//! the inputs and the outputs.

// Not part of the crate's public API, so `missing_docs` doesn't apply here:
// criterion's own `criterion_group!`/`criterion_main!` macros generate an
// undocumented `fn main`.
#![allow(missing_docs, reason = "bench binaries aren't a public API surface")]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::move_gen::move_list::MoveList;
use turox_engine::move_gen::pseudo_legal::{
    castling_moves, king_moves, knight_moves, pawn_moves, pseudo_legal_moves, slider_moves,
};

// The same six standard perft positions as `tests/perft.rs`, chosen there for
// covering structurally distinct move shapes (quiet opening, a busy
// middlegame with both-side castling rights, an open endgame with no
// castling, heavy promotion pressure, ...); duplicated here rather than
// shared, since a bench compiles as its own binary and only sees `pub` API,
// same reason `benches/magic.rs` carries its own `XorShift64` copy instead of
// reaching into `tests/`.
const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
const POSITION_5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
const POSITION_6: &str = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";

/// The six base positions plus, for each, its first four legal children
/// (deterministic: `legal_moves`'s own push order, not randomly sampled),
/// which pulls in captures, promotions, and castling without hand-authoring
/// more FEN strings. 30 positions total, none of them near-empty or
/// near-full: real middlegame/endgame occupancy throughout.
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

fn bench_generator(c: &mut Criterion, name: &str, f: impl Fn(&Board, &mut MoveList)) {
    let boards = sample_boards();
    let mut group = c.benchmark_group("move_gen");
    group.throughput(Throughput::Elements(
        u64::try_from(boards.len()).expect("fits u64"),
    ));
    group.bench_function(name, |b| {
        b.iter(|| {
            for board in &boards {
                let mut list = MoveList::new();
                f(black_box(board), &mut list);
                black_box(list.len());
            }
        });
    });
    group.finish();
}

fn per_piece_generators(c: &mut Criterion) {
    bench_generator(c, "pawn_moves", pawn_moves);
    bench_generator(c, "knight_moves", knight_moves);
    bench_generator(c, "king_moves", king_moves);
    bench_generator(c, "slider_moves", slider_moves);
    bench_generator(c, "castling_moves", castling_moves);
}

fn combined_pseudo_legal(c: &mut Criterion) {
    bench_generator(c, "pseudo_legal_moves", pseudo_legal_moves);
}

fn legal(c: &mut Criterion) {
    let boards = sample_boards();
    let mut group = c.benchmark_group("move_gen");
    group.throughput(Throughput::Elements(
        u64::try_from(boards.len()).expect("fits u64"),
    ));
    group.bench_function("legal_moves", |b| {
        b.iter(|| {
            for board in &boards {
                black_box(legal_moves(black_box(board)).len());
            }
        });
    });
    group.finish();
}

criterion_group!(benches, per_piece_generators, combined_pseudo_legal, legal);
criterion_main!(benches);
