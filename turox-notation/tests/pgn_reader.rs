//! Concrete tests for `pgn::PgnReader`, the streaming counterpart to
//! `parse_pgn`. `tests/pgn.rs` covers the parsing rules themselves in
//! detail; this file only needs to confirm the streaming reader applies
//! those same rules identically when reading from a `BufRead` instead of
//! an in-memory `&str`.

use std::io::Cursor;
use turox_notation::pgn::{parse_pgn, PgnGame, PgnReader};

const SIMPLE_GAME: &str = r#"[Event "rated blitz game"]
[White "playerA"]
[Black "playerB"]
[Result "1-0"]
[WhiteElo "2450"]
[BlackElo "2410"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 1-0
"#;

fn read_all(text: &str) -> Vec<PgnGame> {
    PgnReader::new(Cursor::new(text.as_bytes())).collect()
}

#[test]
fn matches_parse_pgn_for_a_single_game() {
    assert_eq!(read_all(SIMPLE_GAME), parse_pgn(SIMPLE_GAME));
}

#[test]
fn matches_parse_pgn_for_multiple_games() {
    let pgn = format!("{SIMPLE_GAME}\n{SIMPLE_GAME}");
    assert_eq!(read_all(&pgn), parse_pgn(&pgn));
}

#[test]
fn matches_parse_pgn_with_comments_and_nags_and_multiline_movetext() {
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]

1. e4 {a comment} e5 2. Nf3 $1 Nc6 3. Bb5 {another
comment spanning a line break} a6 1-0
"#;
    assert_eq!(read_all(pgn), parse_pgn(pgn));
}

#[test]
fn an_empty_reader_yields_no_games() {
    assert_eq!(read_all(""), Vec::new());
}

#[test]
fn can_be_consumed_one_game_at_a_time() {
    // Iterator::next(), not collect(): the one thing a real test can
    // confirm about this being demand-driven rather than reading
    // everything up front, since memory use itself isn't observable here.
    let pgn = format!("{SIMPLE_GAME}\n{SIMPLE_GAME}\n{SIMPLE_GAME}");
    let mut reader = PgnReader::new(Cursor::new(pgn.as_bytes()));

    let first = reader.next().expect("first game");
    let second = reader.next().expect("second game");
    let third = reader.next().expect("third game");
    assert!(reader.next().is_none(), "exactly three games in this input");

    assert_eq!(first, second);
    assert_eq!(second, third);
}
