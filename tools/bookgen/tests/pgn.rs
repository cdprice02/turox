//! Concrete tests for `pgn::parse_pgn`.

use bookgen::pgn::{parse_pgn, GameResult};

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
