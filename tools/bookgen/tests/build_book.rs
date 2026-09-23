//! End-to-end test: a small synthetic PGN fixture, all the way through
//! `bookgen::build_book`, to a `Book` a real position can be looked up in.
//! `tests/pgn.rs`, `tests/san.rs`, `tests/aggregate.rs`, and `tests/weight.rs`
//! cover each stage in isolation; this is the one place that proves they
//! actually compose.

use bookgen::aggregate::BuildOptions;
use bookgen::build_book;
use turox_chess::board::Board;
use turox_chess::{Move, MoveFlags, Square};

const PGN: &str = r#"[White "a"]
[Black "b"]
[Result "1-0"]
[WhiteElo "2450"]
[BlackElo "2410"]

1. e4 e5 2. Nf3 Nc6 1-0

[White "c"]
[Black "d"]
[Result "1-0"]
[WhiteElo "2500"]
[BlackElo "2480"]

1. e4 c5 2. Nf3 d6 1-0

[White "e"]
[Black "f"]
[Result "0-1"]
[WhiteElo "1200"]
[BlackElo "1150"]

1. d4 d5 1-0
"#;

#[test]
fn a_move_played_by_enough_strong_games_ends_up_in_the_book() {
    let options = BuildOptions {
        min_rating: 2300,
        min_sample_size: 2,
        max_ply: 10,
    };

    let book = build_book(PGN, &options);

    let e4 = Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush);
    let start_hash = Board::start_pos().hash();
    let moves = book.moves(start_hash);

    assert_eq!(
        moves.len(),
        1,
        "e4 is the only startpos move played by a rating-qualifying game at least twice: {moves:?}"
    );
    assert_eq!(moves[0].mv, e4);
    assert!(
        moves[0].weight > 0,
        "a recorded move must carry a positive weight"
    );
}

#[test]
fn the_weak_players_game_never_reaches_the_book_at_all() {
    let options = BuildOptions {
        min_rating: 2300,
        min_sample_size: 1,
        max_ply: 10,
    };

    let book = build_book(PGN, &options);

    let d4 = Move::new(Square::D2, Square::D4, MoveFlags::DoublePawnPush);
    let start_hash = Board::start_pos().hash();
    let moves = book.moves(start_hash);

    assert!(
        !moves.iter().any(|bm| bm.mv == d4),
        "the 1200-rated game's d4 must be excluded by the rating floor: {moves:?}"
    );
}
