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

// Import format: everything below is a form the PGN standard says a reader
// must accept, and real exports rarely or never produce, so each one is pinned
// by hand rather than by replaying real games. Every case checks both readers,
// because `PgnReader` splits games on blank lines before parsing and a form
// that crosses one can be right in `parse_pgn` and wrong there.

/// Wraps `movetext` in a minimal header block.
fn game(movetext: &str) -> String {
    format!("[Event \"e\"]\n[White \"a\"]\n[Black \"b\"]\n[Result \"1-0\"]\n\n{movetext}\n")
}

/// Asserts `pgn` holds exactly one game with mainline `expected`, through both
/// readers.
fn assert_mainline(pgn: &str, expected: &[&str]) {
    let games = parse_pgn(pgn);
    assert_eq!(
        games.len(),
        1,
        "expected one game from {pgn:?}, got {games:?}"
    );
    assert_eq!(games[0].moves, expected, "mainline of {pgn:?}");
    assert_eq!(read_all(pgn), games, "PgnReader disagrees on {pgn:?}");
}

#[test]
fn a_variation_is_skipped_not_read_as_mainline() {
    assert_mainline(
        &game("1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_variation_with_no_surrounding_whitespace_is_skipped() {
    // Parentheses are self-delimiting tokens, so nothing requires a space on
    // either side of one.
    assert_mainline(
        &game("1. e4 e5(1... c5 2. Nf3)2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_nested_variation_is_skipped_to_its_own_closing_parenthesis() {
    // Stopping at the first `)` would resume the mainline at `2. Nf3` inside
    // the outer variation and read it twice.
    assert_mainline(
        &game("1. e4 e5 (1... c5 (1... e6 2. d4) 2. Nf3) 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn parentheses_inside_a_comment_inside_a_variation_do_not_count() {
    // Within `{}` a parenthesis is comment text. Counting it would close the
    // variation at the `)` and then open a new one at the `(` that never closes.
    assert_mainline(
        &game("1. e4 e5 (1... c5 {a) tricky ( comment} 2. Nf3) 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_variation_ending_the_movetext_before_the_result_is_skipped() {
    assert_mainline(
        &game("1. e4 e5 2. Nf3 (2. Nc3 Nf6) 1-0"),
        &["e4", "e5", "Nf3"],
    );
}

#[test]
fn a_semicolon_comment_runs_to_the_end_of_its_line() {
    assert_mainline(
        &game("1. e4 ; e5 2. Nf3 is all comment\ne5 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_semicolon_comment_glued_to_a_move_ends_the_move() {
    assert_mainline(
        &game("1. e4;comment\ne5 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_brace_inside_a_semicolon_comment_opens_nothing() {
    // A `{` here is comment text. Treating it as an opener would swallow the
    // rest of the game looking for a `}` that never comes.
    assert_mainline(
        &game("1. e4 ; a { brace\ne5 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_semicolon_inside_a_brace_comment_does_not_swallow_the_line() {
    assert_mainline(
        &game("1. e4 {a ; b} e5 2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_percent_line_in_movetext_is_discarded() {
    assert_mainline(
        &game("1. e4 e5\n% 2. d4 is escaped, not played\n2. Nf3 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_percent_line_before_the_first_header_is_not_a_game() {
    let pgn = format!("% exported by some tool\n{}", game("1. e4 e5 1-0"));
    assert_mainline(&pgn, &["e4", "e5"]);
}

#[test]
fn a_percent_line_between_headers_is_discarded() {
    let pgn = "[Event \"e\"]\n% escaped\n[White \"a\"]\n[Black \"b\"]\n[Result \"1-0\"]\n[WhiteElo \"2450\"]\n\n1. e4 e5 1-0\n";
    assert_mainline(pgn, &["e4", "e5"]);
    assert_eq!(
        parse_pgn(pgn)[0].white_elo,
        Some(2450),
        "headers after the escape line still parse"
    );
}

#[test]
fn a_percent_inside_a_comment_is_not_an_escape() {
    // Lichess writes clock and eval annotations as `[%clk ...]` inside a brace
    // comment, and a comment's own line break can put that `%` at the start of
    // a line. Only column one of a line outside a comment escapes.
    assert_mainline(
        &game("1. e4 { [%clk 0:03:00] } 1... e5 {\n%still comment} 2. Nf3 1-0"),
        &["e4", "e5", "Nf3"],
    );
}

#[test]
fn lichess_clock_and_eval_annotations_are_skipped() {
    assert_mainline(
        &game(
            "1. e4 { [%eval 0.18] [%clk 0:03:00] } 1... e5 { [%eval 0.2] [%clk 0:03:00] } \
             2. Nf3?! { (0.20 \u{2192} -0.30) Inaccuracy. Nc3 was best. } { [%eval -0.3] [%clk 0:02:59] } \
             (2. Nc3 Nf6) 2... Nc6 { [%clk 0:02:58] } 1-0",
        ),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn suffix_annotations_are_stripped_from_moves() {
    assert_mainline(
        &game("1. e4! e5? 2. Nf3!? Nc6?! 3. Bb5!! a6?? 4. O-O!! Nf6 1-0"),
        &["e4", "e5", "Nf3", "Nc6", "Bb5", "a6", "O-O", "Nf6"],
    );
}

#[test]
fn suffix_annotations_after_check_mate_and_promotion_keep_those_marks() {
    // `+`, `#` and `=Q` are SAN, not annotation: only the trailing `!`/`?`
    // run goes.
    assert_mainline(
        &game("1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7#! 1-0"),
        &["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"],
    );
    assert_mainline(
        &game("1. e8=Q+!? Kd7 2. a1=N?! 1-0"),
        &["e8=Q+", "Kd7", "a1=N"],
    );
}

#[test]
fn a_nag_glued_to_a_move_is_stripped() {
    assert_mainline(
        &game("1. e4$1 e5$2 2. Nf3 $14 Nc6 1-0"),
        &["e4", "e5", "Nf3", "Nc6"],
    );
}

#[test]
fn a_stray_closing_parenthesis_is_ignored() {
    assert_mainline(&game("1. e4 ) e5 2. Nf3 1-0"), &["e4", "e5", "Nf3"]);
}

#[test]
fn an_unclosed_variation_at_end_of_input_terminates() {
    // Every pass must still consume something when the closer never arrives;
    // otherwise this hangs rather than failing.
    assert_mainline(&game("1. e4 e5 (1... c5 2. Nf3"), &["e4", "e5"]);
}

#[test]
fn an_unclosed_brace_comment_at_end_of_input_terminates() {
    assert_mainline(&game("1. e4 e5 {never closed 2. Nf3"), &["e4", "e5"]);
}

#[test]
fn an_unclosed_nested_variation_at_end_of_input_terminates() {
    assert_mainline(&game("1. e4 e5 (1... c5 (1... e6"), &["e4", "e5"]);
}

#[test]
fn a_comment_containing_a_blank_line_stays_inside_its_game() {
    // A comment may hold anything but `}`, blank lines included. Splitting
    // games on blank lines alone ends this one mid-comment.
    let pgn = format!(
        "{}{}",
        game("1. e4 {first paragraph\n\nsecond paragraph} e5 2. Nf3 1-0"),
        game("1. d4 d5 1-0"),
    );
    let games = parse_pgn(&pgn);
    assert_eq!(games.len(), 2, "{games:?}");
    assert_eq!(games[0].moves, ["e4", "e5", "Nf3"]);
    assert_eq!(games[1].moves, ["d4", "d5"]);
    assert_eq!(read_all(&pgn), games);
}

#[test]
fn a_variation_containing_a_blank_line_stays_inside_its_game() {
    let pgn = format!(
        "{}{}",
        game("1. e4 e5 (1... c5\n\n2. Nf3) 2. Nf3 1-0"),
        game("1. d4 d5 1-0"),
    );
    let games = parse_pgn(&pgn);
    assert_eq!(games.len(), 2, "{games:?}");
    assert_eq!(games[0].moves, ["e4", "e5", "Nf3"]);
    assert_eq!(games[1].moves, ["d4", "d5"]);
    assert_eq!(read_all(&pgn), games);
}

/// The `Opening` tag's parsed value for a header line written as
/// `[Opening <raw>]`, through both readers.
fn opening_tag(raw: &str) -> Option<String> {
    let pgn = format!("[Event \"e\"]\n[Opening {raw}]\n[Result \"1-0\"]\n\n1. e4 1-0\n");
    let games = parse_pgn(&pgn);
    assert_eq!(games.len(), 1, "{games:?}");
    assert_eq!(
        games[0].moves,
        ["e4"],
        "a tag value must not leak into movetext"
    );
    assert_eq!(read_all(&pgn), games, "PgnReader disagrees on {pgn:?}");
    games[0].opening_name.clone()
}

#[test]
fn an_escaped_quote_in_a_tag_value_is_unescaped() {
    assert_eq!(
        opening_tag(r#""The \"Fried\" Liver""#).as_deref(),
        Some(r#"The "Fried" Liver"#)
    );
}

#[test]
fn an_escaped_backslash_in_a_tag_value_is_unescaped() {
    assert_eq!(opening_tag(r#""a \\ b""#).as_deref(), Some(r"a \ b"));
}

#[test]
fn an_escaped_backslash_before_the_closing_quote_ends_the_value() {
    // `\\"` is an escaped backslash and then the real closing quote, not a
    // backslash followed by an escaped quote. Reading it the second way runs
    // the value on past the end of the tag.
    assert_eq!(
        opening_tag(r#""ends in \\""#).as_deref(),
        Some(r"ends in \")
    );
}

#[test]
fn a_closing_bracket_inside_a_quoted_tag_value_is_value_text() {
    assert_eq!(
        opening_tag(r#""Line [A] or ]B[""#).as_deref(),
        Some("Line [A] or ]B[")
    );
}
