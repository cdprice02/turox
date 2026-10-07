//! Concrete tests for `pgn`.
//!
//! `parse_pgn`'s tests cover the parsing rules in detail. `PgnReader` is its
//! streaming counterpart, so its tests only confirm it applies the same rules
//! when reading from a `BufRead` instead of an in-memory `&str`, by comparing
//! its output with `parse_pgn`'s.

use std::io::Cursor;
use turox_notation::pgn::{parse_pgn, GameResult, PgnGame, PgnReader};

const SIMPLE_GAME: &str = r#"[Event "rated blitz game"]
[White "playerA"]
[Black "playerB"]
[Result "1-0"]
[WhiteElo "2450"]
[BlackElo "2410"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 1-0
"#;

#[test]
fn parses_headers_and_moves_from_a_single_game() {
    let games = parse_pgn(SIMPLE_GAME);
    assert_eq!(games.len(), 1, "expected exactly one game, got {games:?}");
    let game = &games[0];
    assert_eq!(game.white_elo, Some(2450));
    assert_eq!(game.black_elo, Some(2410));
    assert_eq!(game.result, GameResult::WhiteWins);
    assert_eq!(game.moves, vec!["e4", "e5", "Nf3", "Nc6", "Bb5"]);
}

#[test]
fn parses_every_result_tag_to_the_matching_variant() {
    let make = |result_tag: &str| {
        format!(
            "[Event \"e\"]\n[White \"a\"]\n[Black \"b\"]\n[Result \"{result_tag}\"]\n\n1. e4 e5 {result_tag}\n"
        )
    };

    assert_eq!(parse_pgn(&make("1-0"))[0].result, GameResult::WhiteWins);
    assert_eq!(parse_pgn(&make("0-1"))[0].result, GameResult::BlackWins);
    assert_eq!(parse_pgn(&make("1/2-1/2"))[0].result, GameResult::Draw);
    assert_eq!(parse_pgn(&make("*"))[0].result, GameResult::Unknown);
}

#[test]
fn a_missing_elo_tag_parses_as_none_not_a_dropped_game() {
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]
[WhiteElo "2450"]

1. e4 e5 1-0
"#;
    let games = parse_pgn(pgn);
    assert_eq!(
        games.len(),
        1,
        "a missing BlackElo tag must not drop the game"
    );
    assert_eq!(games[0].white_elo, Some(2450));
    assert_eq!(games[0].black_elo, None);
}

#[test]
fn parses_multiple_games_in_one_file() {
    let pgn = format!("{SIMPLE_GAME}\n{SIMPLE_GAME}");
    let games = parse_pgn(&pgn);
    assert_eq!(games.len(), 2, "expected two games, got {games:?}");
    assert_eq!(games[0], games[1]);
}

#[test]
fn strips_move_numbers_comments_and_nags_from_movetext() {
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]

1. e4 {a comment} e5 2. Nf3 $1 Nc6 3. Bb5 {another
comment spanning a line break} a6 1-0
"#;
    let games = parse_pgn(pgn);
    assert_eq!(games.len(), 1);
    assert_eq!(
        games[0].moves,
        vec!["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"],
        "comments, NAGs, and move numbers must not leak into the move list"
    );
}

#[test]
fn strips_castling_check_and_mate_suffixes_as_plain_tokens() {
    // Resolving what "O-O+" *means* is `san`'s job; this only checks that
    // the token survives tokenization intact, suffix and all, rather than
    // being mangled by whatever strips numeric move-number suffixes.
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 4. O-O Nf6 5. Re1 Bc5 6. c3 O-O 7. d4 Bb6 8. Qd3 Re8 9. Bg5 exd4 1-0
"#;
    let games = parse_pgn(pgn);
    assert_eq!(games.len(), 1);
    assert!(
        games[0].moves.contains(&"O-O".to_string()),
        "castling token must survive tokenization: {:?}",
        games[0].moves
    );
}

#[test]
fn an_empty_file_produces_no_games() {
    assert_eq!(parse_pgn(""), Vec::new());
}

#[test]
fn opening_tag_is_preferred_over_eco_when_both_are_present() {
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]
[ECO "C65"]
[Opening "Ruy Lopez: Berlin Defense"]

1. e4 e5 1-0
"#;
    let games = parse_pgn(pgn);
    assert_eq!(
        games[0].opening_name,
        Some("Ruy Lopez: Berlin Defense".to_string())
    );
}

#[test]
fn eco_is_used_when_opening_is_absent() {
    let pgn = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]
[ECO "C65"]

1. e4 e5 1-0
"#;
    let games = parse_pgn(pgn);
    assert_eq!(
        games[0].opening_name,
        Some("C65".to_string()),
        "ECO must be the fallback identifier when no Opening tag names the line"
    );
}

#[test]
fn opening_name_is_none_when_neither_tag_is_present() {
    let games = parse_pgn(SIMPLE_GAME);
    assert_eq!(
        games[0].opening_name, None,
        "SIMPLE_GAME carries neither an Opening nor an ECO tag"
    );
}

#[test]
fn opening_wins_over_eco_regardless_of_which_header_line_comes_first() {
    // The Seven Tag Roster orders these together but doesn't guarantee
    // which of the two comes first; `Opening` must win either way, not just
    // when it happens to be the later tag `parse_one_game` overwrites with.
    let opening_first = r#"[Event "e"]
[White "a"]
[Black "b"]
[Result "1-0"]
[Opening "Sicilian Defense"]
[ECO "B20"]

1. e4 c5 1-0
"#;
    assert_eq!(
        parse_pgn(opening_first)[0].opening_name,
        Some("Sicilian Defense".to_string())
    );
}

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
