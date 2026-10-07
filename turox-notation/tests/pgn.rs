//! Concrete tests for `pgn`.
//!
//! Everything goes through [`parse`], which also reads the same input through
//! a `PgnReader` with a one-byte buffer and requires an identical result: a
//! scanner that mishandles a refill at a token, comment or line boundary
//! shows up there rather than only on a 3 GB file.
//!
//! Most forms here are ones real exports rarely or never produce, so each is
//! pinned by hand rather than by replaying real games. Error tests assert the
//! whole `PgnError`, position included, not only its kind.

use std::io::BufReader;
use turox_notation::pgn::{
    parse_movetext, parse_pgn, GameResult, Line, MoveNode, PgnError, PgnErrorKind, PgnGame,
    PgnReader,
};

/// Every game in `pgn`, through `parse_pgn` and through a one-byte-buffered
/// `PgnReader`, which must agree.
fn parse(pgn: &str) -> Vec<Result<PgnGame, PgnError>> {
    let whole: Vec<_> = parse_pgn(pgn).collect();
    let byte_at_a_time: Vec<_> =
        PgnReader::new(BufReader::with_capacity(1, pgn.as_bytes())).collect();
    assert_eq!(
        byte_at_a_time, whole,
        "a one-byte buffer changed the parse of {pgn:?}"
    );
    whole
}

/// The single game in `pgn`, which must parse cleanly.
#[expect(
    clippy::panic,
    reason = "test-only helper, not itself a #[test] fn, so clippy's test-context detection doesn't cover it; the input and the unexpected value are worth keeping in the message"
)]
fn one_game(pgn: &str) -> PgnGame {
    let mut results = parse(pgn);
    assert_eq!(
        results.len(),
        1,
        "expected one game from {pgn:?}, got {results:?}"
    );
    match results.remove(0) {
        Ok(game) => game,
        Err(err) => panic!("expected {pgn:?} to parse, got {err:?}"),
    }
}

/// The single result in `pgn`, which must be an error.
#[expect(
    clippy::panic,
    reason = "test-only helper, not itself a #[test] fn, so clippy's test-context detection doesn't cover it; the input and the unexpected value are worth keeping in the message"
)]
fn one_error(pgn: &str) -> PgnError {
    let mut results = parse(pgn);
    assert_eq!(
        results.len(),
        1,
        "expected one result from {pgn:?}, got {results:?}"
    );
    match results.remove(0) {
        Ok(game) => panic!("expected {pgn:?} to fail, got {game:?}"),
        Err(err) => err,
    }
}

fn mainline(game: &PgnGame) -> Vec<&str> {
    game.mainline().collect()
}

/// Wraps `movetext` in a four-tag header block. The movetext starts on line
/// 6, which error positions below depend on.
fn game(movetext: &str) -> String {
    format!("[Event \"e\"]\n[White \"a\"]\n[Black \"b\"]\n[Result \"1-0\"]\n\n{movetext}\n")
}

fn assert_mainline(pgn: &str, expected: &[&str]) {
    let game = one_game(pgn);
    assert_eq!(mainline(&game), expected, "mainline of {pgn:?}");
}

const fn error(line: usize, column: usize, kind: PgnErrorKind) -> PgnError {
    PgnError { line, column, kind }
}

fn node(san: &str) -> MoveNode {
    MoveNode {
        san: san.to_string(),
        ..MoveNode::default()
    }
}

const SIMPLE_GAME: &str = r#"[Event "rated blitz game"]
[White "playerA"]
[Black "playerB"]
[Result "1-0"]
[WhiteElo "2450"]
[BlackElo "2410"]

1. e4 e5 2. Nf3 Nc6 3. Bb5 1-0
"#;

// Tags.

#[test]
fn parses_headers_and_moves_from_a_single_game() {
    let game = one_game(SIMPLE_GAME);
    assert_eq!(game.white_elo(), Some(2450));
    assert_eq!(game.black_elo(), Some(2410));
    assert_eq!(game.result(), GameResult::WhiteWins);
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3", "Nc6", "Bb5"]);
}

#[test]
fn every_tag_pair_is_kept_in_file_order() {
    let game = one_game(SIMPLE_GAME);
    let names: Vec<&str> = game.tags.0.iter().map(|pair| pair.name.as_str()).collect();
    assert_eq!(
        names,
        ["Event", "White", "Black", "Result", "WhiteElo", "BlackElo"]
    );
    assert_eq!(game.tags.get("Event"), Some("rated blitz game"));
}

#[test]
fn tag_lookup_is_case_sensitive() {
    assert_eq!(one_game(SIMPLE_GAME).tags.get("event"), None);
}

#[test]
fn a_repeated_tag_is_kept_and_lookup_returns_the_first() {
    let game = one_game("[Event \"first\"]\n[Event \"second\"]\n\n1. e4 *\n");
    assert_eq!(game.tags.0.len(), 2);
    assert_eq!(game.tags.get("Event"), Some("first"));
}

#[test]
fn parses_every_result_tag_to_the_matching_variant() {
    let result_of =
        |tag: &str| one_game(&format!("[Result \"{tag}\"]\n\n1. e4 e5 {tag}\n")).result();
    assert_eq!(result_of("1-0"), GameResult::WhiteWins);
    assert_eq!(result_of("0-1"), GameResult::BlackWins);
    assert_eq!(result_of("1/2-1/2"), GameResult::Draw);
    assert_eq!(result_of("*"), GameResult::Unknown);
}

#[test]
fn a_missing_result_tag_reads_as_unknown() {
    assert_eq!(
        one_game("[Event \"e\"]\n\n1. e4 1-0\n").result(),
        GameResult::Unknown
    );
}

#[test]
fn a_missing_elo_tag_parses_as_none_not_a_dropped_game() {
    let game = one_game("[Result \"1-0\"]\n[WhiteElo \"2450\"]\n\n1. e4 e5 1-0\n");
    assert_eq!(game.white_elo(), Some(2450));
    assert_eq!(game.black_elo(), None);
}

#[test]
fn a_non_numeric_elo_tag_parses_as_none() {
    assert_eq!(one_game("[WhiteElo \"?\"]\n\n1. e4 *\n").white_elo(), None);
}

/// The `Opening` tag's value for a header line written as `[Opening <raw>]`.
fn opening_tag(raw: &str) -> Option<String> {
    let game = one_game(&format!(
        "[Event \"e\"]\n[Opening {raw}]\n[Result \"1-0\"]\n\n1. e4 1-0\n"
    ));
    assert_eq!(
        mainline(&game),
        ["e4"],
        "a tag value must not leak into movetext"
    );
    game.tags.get("Opening").map(str::to_string)
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

#[test]
fn an_empty_tag_value_is_kept() {
    assert_eq!(opening_tag(r#""""#).as_deref(), Some(""));
}

#[test]
fn opening_tag_is_preferred_over_eco_when_both_are_present() {
    let game = one_game("[ECO \"C65\"]\n[Opening \"Ruy Lopez: Berlin Defense\"]\n\n1. e4 *\n");
    assert_eq!(game.opening_name(), Some("Ruy Lopez: Berlin Defense"));
}

#[test]
fn opening_wins_over_eco_regardless_of_which_header_line_comes_first() {
    let game = one_game("[Opening \"Sicilian Defense\"]\n[ECO \"B20\"]\n\n1. e4 c5 *\n");
    assert_eq!(game.opening_name(), Some("Sicilian Defense"));
}

#[test]
fn eco_is_used_when_opening_is_absent_or_empty() {
    let absent = one_game("[ECO \"C65\"]\n\n1. e4 *\n");
    assert_eq!(absent.opening_name(), Some("C65"));
    let empty = one_game("[Opening \"\"]\n[ECO \"C65\"]\n\n1. e4 *\n");
    assert_eq!(empty.opening_name(), Some("C65"));
}

#[test]
fn opening_name_is_none_when_neither_tag_is_present() {
    assert_eq!(one_game(SIMPLE_GAME).opening_name(), None);
}

#[test]
fn a_fen_tag_is_the_starting_position_unless_setup_says_otherwise() {
    const FEN: &str = "8/8/8/8/8/8/4k3/4K3 w - - 0 1";
    let with = |tags: &str| one_game(&format!("{tags}\n\n1. Kd1 *\n"));

    let paired = with(&format!("[SetUp \"1\"]\n[FEN \"{FEN}\"]"));
    assert_eq!(paired.starting_fen(), Some(FEN));
    let bare = with(&format!("[FEN \"{FEN}\"]"));
    assert_eq!(bare.starting_fen(), Some(FEN), "exporters often omit SetUp");
    let disabled = with(&format!("[SetUp \"0\"]\n[FEN \"{FEN}\"]"));
    assert_eq!(disabled.starting_fen(), None);
    assert_eq!(one_game(SIMPLE_GAME).starting_fen(), None);
}

// Games and termination.

#[test]
fn an_empty_file_produces_no_games() {
    assert_eq!(parse(""), Vec::new());
    assert_eq!(parse("\n\n  \n"), Vec::new());
}

#[test]
fn parses_multiple_games_in_one_file() {
    let results = parse(&format!("{SIMPLE_GAME}\n{SIMPLE_GAME}"));
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0], results[1]);
}

#[test]
fn games_need_no_blank_line_between_them() {
    let results = parse(&format!("{SIMPLE_GAME}{SIMPLE_GAME}"));
    assert_eq!(results.len(), 2, "{results:?}");
}

#[test]
fn the_termination_marker_is_kept_apart_from_the_result_tag() {
    // The two are written independently; a file can carry both and have
    // them disagree.
    let game = one_game("[Result \"1-0\"]\n\n1. e4 e5 0-1\n");
    assert_eq!(game.result(), GameResult::WhiteWins);
    assert_eq!(game.termination, Some(GameResult::BlackWins));
}

#[test]
fn every_termination_marker_is_recognised() {
    let marker = |m: &str| one_game(&game(&format!("1. e4 {m}"))).termination;
    assert_eq!(marker("1-0"), Some(GameResult::WhiteWins));
    assert_eq!(marker("0-1"), Some(GameResult::BlackWins));
    assert_eq!(marker("1/2-1/2"), Some(GameResult::Draw));
    assert_eq!(marker("*"), Some(GameResult::Unknown));
}

#[test]
fn input_ending_before_a_termination_marker_still_yields_the_game() {
    let game = one_game(&game("1. e4 e5 2. Nf3"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    assert_eq!(game.termination, None);
}

#[test]
fn a_header_block_with_no_movetext_is_a_game_with_no_moves() {
    let results = parse("[Event \"a\"]\n\n[Event \"b\"]\n\n1. e4 *\n");
    assert_eq!(results.len(), 2, "{results:?}");
    let first = results[0].clone().expect("the empty game parses");
    assert_eq!(first.movetext, Line::default());
    assert_eq!(first.termination, None);
    let second = results[1].clone().expect("the next game parses");
    assert_eq!(mainline(&second), ["e4"]);
}

#[test]
fn a_game_with_only_a_termination_marker_has_no_moves() {
    let game = one_game(&game("1-0"));
    assert!(game.movetext.moves.is_empty(), "{game:?}");
    assert_eq!(game.termination, Some(GameResult::WhiteWins));
}

#[test]
fn can_be_consumed_one_game_at_a_time() {
    let pgn = format!("{SIMPLE_GAME}\n{SIMPLE_GAME}\n{SIMPLE_GAME}");
    let mut reader = PgnReader::new(pgn.as_bytes());

    let first = reader.next().expect("first game");
    let second = reader.next().expect("second game");
    let third = reader.next().expect("third game");
    assert!(reader.next().is_none(), "exactly three games in this input");

    assert_eq!(first, second);
    assert_eq!(second, third);
}

// Movetext tokens.

#[test]
fn move_numbers_are_not_moves() {
    assert_mainline(
        &game("1. e4 1... e5 2.Nf3 2...Nc6 3 . Bb5 1-0"),
        &["e4", "e5", "Nf3", "Nc6", "Bb5"],
    );
}

#[test]
fn castling_check_and_mate_marks_are_part_of_the_san() {
    assert_mainline(
        &game("1. O-O O-O-O+ 2. Qxf7# 1-0"),
        &["O-O", "O-O-O+", "Qxf7#"],
    );
}

#[test]
fn promotion_is_part_of_the_san() {
    assert_mainline(&game("1. e8=Q+ dxc1=N 1-0"), &["e8=Q+", "dxc1=N"]);
}

#[test]
fn suffix_glyphs_become_their_nags() {
    let game = one_game(&game("1. e4! e5? 2. Nf3!! Nc6?? 3. Bb5!? a6?! 1-0"));
    let moves = &game.movetext.moves;
    let nags: Vec<&[u8]> = moves.iter().map(|m| m.nags.as_slice()).collect();
    assert_eq!(nags, [[1], [2], [3], [4], [5], [6]]);
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"]);
}

#[test]
fn a_suffix_glyph_after_check_mate_or_promotion_leaves_those_marks() {
    let game = one_game(&game("1. Qxf7#! e8=Q+!? 1-0"));
    assert_eq!(game.movetext.moves[0].san, "Qxf7#");
    assert_eq!(game.movetext.moves[0].nags, [1]);
    assert_eq!(game.movetext.moves[1].san, "e8=Q+");
    assert_eq!(game.movetext.moves[1].nags, [5]);
}

#[test]
fn nags_attach_to_the_move_before_them_in_order() {
    let game = one_game(&game("1. e4!? $14 $32 e5 $0 $255 1-0"));
    assert_eq!(game.movetext.moves[0].nags, [5, 14, 32]);
    assert_eq!(game.movetext.moves[1].nags, [0, 255]);
}

#[test]
fn a_nag_glued_to_a_move_ends_the_move() {
    let game = one_game(&game("1. e4$1 e5$2 1-0"));
    assert_eq!(
        game.movetext.moves,
        [
            MoveNode {
                nags: vec![1],
                ..node("e4")
            },
            MoveNode {
                nags: vec![2],
                ..node("e5")
            },
        ]
    );
}

// Comments.

#[test]
fn a_comment_attaches_to_the_move_before_it_verbatim() {
    let game = one_game(&game("1. e4 { a  spaced\ncomment } e5 1-0"));
    assert_eq!(game.movetext.moves[0].comments, [" a  spaced\ncomment "]);
    assert!(game.movetext.moves[1].comments.is_empty(), "{game:?}");
}

#[test]
fn a_comment_before_the_first_move_belongs_to_the_line() {
    let game = one_game(&game("{about this game} 1. e4 {a} {b} e5 1-0"));
    assert_eq!(game.movetext.comments, ["about this game"]);
    assert_eq!(game.movetext.moves[0].comments, ["a", "b"]);
}

#[test]
fn a_semicolon_comment_runs_to_the_end_of_its_line() {
    let game = one_game(&game("1. e4 ;e5 2. Nf3 is all comment\ne5 2. Nf3 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    assert_eq!(
        game.movetext.moves[0].comments,
        ["e5 2. Nf3 is all comment"]
    );
}

#[test]
fn a_semicolon_comment_glued_to_a_move_ends_the_move() {
    assert_mainline(&game("1. e4;comment\ne5 2. Nf3 1-0"), &["e4", "e5", "Nf3"]);
}

#[test]
fn a_brace_inside_a_semicolon_comment_opens_nothing() {
    // A `{` here is comment text. Treating it as an opener would swallow the
    // rest of the game looking for a `}` that never comes.
    assert_mainline(
        &game("1. e4 ; a { brace\ne5 2. Nf3 1-0"),
        &["e4", "e5", "Nf3"],
    );
}

#[test]
fn a_semicolon_inside_a_brace_comment_does_not_swallow_the_line() {
    let game = one_game(&game("1. e4 {a ; b} e5 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5"]);
    assert_eq!(game.movetext.moves[0].comments, ["a ; b"]);
}

#[test]
fn a_comment_containing_a_blank_line_stays_inside_its_game() {
    // A comment may hold anything but `}`, blank lines included, so a game
    // cannot be found by splitting on blank lines.
    let results = parse(&format!(
        "{}{}",
        game("1. e4 {first paragraph\n\nsecond paragraph} e5 1-0"),
        game("1. d4 d5 1-0"),
    ));
    assert_eq!(results.len(), 2, "{results:?}");
    let first = results[0].clone().expect("first game parses");
    assert_eq!(mainline(&first), ["e4", "e5"]);
    assert_eq!(
        first.movetext.moves[0].comments,
        ["first paragraph\n\nsecond paragraph"]
    );
}

#[test]
fn lichess_clock_and_eval_annotations_are_comment_text() {
    let game = one_game(&game(
        "1. e4 { [%eval 0.18] [%clk 0:03:00] } 1... e5 { [%clk 0:03:00] } \
         2. Nf3?! { (0.20 \u{2192} -0.30) Inaccuracy. Nc3 was best. } { [%eval -0.3] } \
         (2. Nc3 Nf6) 2... Nc6 1-0",
    ));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3", "Nc6"]);
    let nf3 = &game.movetext.moves[2];
    assert_eq!(nf3.nags, [6]);
    assert_eq!(
        nf3.comments,
        [
            " (0.20 \u{2192} -0.30) Inaccuracy. Nc3 was best. ",
            " [%eval -0.3] "
        ]
    );
    assert_eq!(nf3.variations.len(), 1);
}

// Variations.

#[test]
fn a_variation_belongs_to_the_move_it_replaces() {
    let game = one_game(&game("1. e4 e5 (1... c5 2. Nf3) (1... e6) 2. Nf3 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    let moves = &game.movetext.moves;
    assert!(moves[0].variations.is_empty(), "{moves:?}");
    let alternatives: Vec<Vec<&str>> = moves[1]
        .variations
        .iter()
        .map(|line| line.mainline().collect())
        .collect();
    assert_eq!(alternatives, [vec!["c5", "Nf3"], vec!["e6"]]);
    assert!(moves[2].variations.is_empty(), "{moves:?}");
}

#[test]
fn a_variation_with_no_surrounding_whitespace_is_still_a_variation() {
    // Parentheses are self-delimiting tokens, so nothing requires a space on
    // either side of one.
    let game = one_game(&game("1. e4 e5(1... c5)2. Nf3 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    assert_eq!(game.movetext.moves[1].variations.len(), 1);
}

#[test]
fn a_nested_variation_belongs_to_the_move_inside_its_parent() {
    // Stopping at the first `)` would resume the mainline inside the outer
    // variation and read `2. Nf3` as a mainline move twice.
    let game = one_game(&game("1. e4 e5 (1... c5 2. Nf3 (2. c3 d5)) 2. Nf3 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    let outer = &game.movetext.moves[1].variations[0];
    assert_eq!(outer.mainline().collect::<Vec<_>>(), ["c5", "Nf3"]);
    let inner = &outer.moves[1].variations[0];
    assert_eq!(inner.mainline().collect::<Vec<_>>(), ["c3", "d5"]);
}

#[test]
fn parentheses_inside_a_comment_inside_a_variation_do_not_count() {
    // Within `{}` a parenthesis is comment text. Counting it would close the
    // variation at the `)` and open a new one at the `(` that never closes.
    let game = one_game(&game(
        "1. e4 e5 (1... c5 {a) tricky ( comment} 2. Nf3) 2. Nf3 1-0",
    ));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    let variation = &game.movetext.moves[1].variations[0];
    assert_eq!(variation.moves[0].comments, ["a) tricky ( comment"]);
}

#[test]
fn a_comment_opening_a_variation_belongs_to_the_variation() {
    let game = one_game(&game("1. e4 e5 ({intro} 1... c5) 1-0"));
    let variation = &game.movetext.moves[1].variations[0];
    assert_eq!(variation.comments, ["intro"]);
    assert!(variation.moves[0].comments.is_empty(), "{variation:?}");
}

#[test]
fn comments_either_side_of_a_variation_belong_to_the_move_it_replaces() {
    let game = one_game(&game("1. e4 e5 {before} (1... c5) {after} 2. Nf3 1-0"));
    let e5 = &game.movetext.moves[1];
    assert_eq!(e5.comments, ["before", "after"]);
    assert_eq!(e5.variations.len(), 1);
}

#[test]
fn a_variation_ending_the_movetext_before_the_marker_is_a_variation() {
    let game = one_game(&game("1. e4 e5 2. Nf3 (2. Nc3 Nf6) 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5", "Nf3"]);
    assert_eq!(game.termination, Some(GameResult::WhiteWins));
}

#[test]
fn a_variation_containing_a_blank_line_stays_inside_its_game() {
    let results = parse(&format!(
        "{}{}",
        game("1. e4 e5 (1... c5\n\n2. Nf3) 2. Nf3 1-0"),
        game("1. d4 d5 1-0"),
    ));
    assert_eq!(results.len(), 2, "{results:?}");
    let first = results[0].clone().expect("first game parses");
    assert_eq!(mainline(&first), ["e4", "e5", "Nf3"]);
}

// Escape lines.

#[test]
fn a_percent_line_in_movetext_is_discarded() {
    assert_mainline(
        &game("1. e4 e5\n% 2. d4 is escaped, not played\n2. Nf3 1-0"),
        &["e4", "e5", "Nf3"],
    );
}

#[test]
fn a_percent_line_before_the_first_tag_is_not_a_game() {
    assert_mainline(
        &format!("% exported by some tool\n{}", game("1. e4 e5 1-0")),
        &["e4", "e5"],
    );
}

#[test]
fn a_percent_line_between_tags_is_discarded() {
    let game = one_game("[Event \"e\"]\n% escaped\n[WhiteElo \"2450\"]\n\n1. e4 e5 1-0\n");
    assert_eq!(game.tags.0.len(), 2, "{game:?}");
    assert_eq!(game.white_elo(), Some(2450));
}

#[test]
fn a_percent_inside_a_comment_is_comment_text() {
    // A comment's own line break can put a `%` in column one. It is still
    // inside the comment.
    let game = one_game(&game("1. e4 {\n%still comment} e5 1-0"));
    assert_eq!(mainline(&game), ["e4", "e5"]);
    assert_eq!(game.movetext.moves[0].comments, ["\n%still comment"]);
}

#[test]
fn a_percent_after_column_one_is_not_an_escape() {
    let err = one_error(&game("1. e4\n %not an escape\ne5 1-0"));
    assert_eq!(err, error(7, 2, PgnErrorKind::UnexpectedByte(b'%')));
}

// Encoding and line endings.

#[test]
fn crlf_line_endings_parse_like_lf() {
    let lf = game("1. e4 ;note\ne5 {a\nb} 2. Nf3 1-0");
    let crlf = lf.replace('\n', "\r\n");
    let lf_game = one_game(&lf);
    let crlf_game = one_game(&crlf);
    assert_eq!(mainline(&crlf_game), mainline(&lf_game));
    assert_eq!(crlf_game.tags, lf_game.tags);
    assert_eq!(
        crlf_game.movetext.moves[0].comments,
        ["note"],
        "a line comment ends before the whole line ending, carriage return included"
    );
}

#[test]
fn comments_and_tag_values_decode_as_utf8() {
    let game = one_game("[White \"Ren\u{e9}\"]\n\n1. e4 {caf\u{e9} \u{2192} \u{265e}} *\n");
    assert_eq!(game.tags.get("White"), Some("Ren\u{e9}"));
    assert_eq!(
        game.movetext.moves[0].comments,
        ["caf\u{e9} \u{2192} \u{265e}"]
    );
}

#[test]
fn invalid_utf8_in_a_comment_decodes_lossily_rather_than_failing() {
    let bytes = b"[Event \"e\"]\n\n1. e4 {bad \xff byte} *\n";
    let results: Vec<_> = PgnReader::new(&bytes[..]).collect();
    assert_eq!(results.len(), 1, "{results:?}");
    let game = results[0]
        .clone()
        .expect("a bad byte in a comment is not a parse error");
    assert_eq!(game.movetext.moves[0].comments, ["bad \u{fffd} byte"]);
}

// Errors. The header block from `game` puts movetext on line 6.

#[test]
fn an_unclosed_brace_comment_is_reported_at_its_brace() {
    let err = one_error(&game("1. e4 e5 {never closed 2. Nf3"));
    assert_eq!(err, error(6, 10, PgnErrorKind::UnterminatedComment));
}

#[test]
fn an_unclosed_variation_at_end_of_input_is_reported_at_its_parenthesis() {
    let err = one_error(&game("1. e4 e5 (1... c5 2. Nf3"));
    assert_eq!(err, error(6, 10, PgnErrorKind::UnclosedVariation));
}

#[test]
fn an_unclosed_variation_at_the_termination_marker_is_reported() {
    let err = one_error(&game("1. e4 e5 (1... c5 1-0"));
    assert_eq!(err, error(6, 10, PgnErrorKind::UnclosedVariation));
}

#[test]
fn an_unclosed_nested_variation_is_reported_at_the_innermost_parenthesis() {
    let err = one_error(&game("1. e4 e5 (1... c5 (1... e6"));
    assert_eq!(err, error(6, 19, PgnErrorKind::UnclosedVariation));
}

#[test]
fn a_stray_closing_parenthesis_is_reported() {
    let err = one_error(&game("1. e4 ) e5 2. Nf3 1-0"));
    assert_eq!(err, error(6, 7, PgnErrorKind::UnmatchedCloseParen));
}

#[test]
fn a_variation_before_any_mainline_move_is_reported() {
    let err = one_error(&game("(1. d4) 1. e4 1-0"));
    assert_eq!(err, error(6, 1, PgnErrorKind::VariationWithoutMove));
}

#[test]
fn a_variation_before_any_move_of_its_parent_variation_is_reported() {
    let err = one_error(&game("1. e4 ((1. d4) 1. c4) 1-0"));
    assert_eq!(err, error(6, 8, PgnErrorKind::VariationWithoutMove));
}

#[test]
fn a_nag_before_any_move_is_reported() {
    let err = one_error(&game("$1 1. e4 1-0"));
    assert_eq!(err, error(6, 1, PgnErrorKind::NagWithoutMove));
}

#[test]
fn a_dollar_with_no_number_is_reported() {
    let err = one_error(&game("1. e4 $ e5 1-0"));
    assert_eq!(err, error(6, 7, PgnErrorKind::InvalidNag));
}

#[test]
fn a_nag_above_255_is_reported() {
    let err = one_error(&game("1. e4 $256 1-0"));
    assert_eq!(err, error(6, 7, PgnErrorKind::InvalidNag));
}

#[test]
fn a_reserved_angle_bracket_is_reported() {
    let err = one_error(&game("1. e4 < e5 1-0"));
    assert_eq!(err, error(6, 7, PgnErrorKind::UnexpectedByte(b'<')));
}

#[test]
fn a_tag_pair_with_no_value_is_reported_at_its_bracket() {
    let err = one_error("[Event]\n\n1. e4 1-0\n");
    assert_eq!(err, error(1, 1, PgnErrorKind::MalformedTagPair));
}

#[test]
fn a_tag_value_unclosed_at_end_of_line_is_reported_at_its_quote() {
    let err = one_error("[Event \"abc]\n\n1. e4 1-0\n");
    assert_eq!(err, error(1, 8, PgnErrorKind::UnterminatedString));
}

#[test]
fn a_malformed_game_costs_only_itself() {
    let results = parse(&format!(
        "{}{}",
        game("1. e4 ) e5 1-0"),
        game("1. d4 d5 1-0")
    ));
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(
        results[0],
        Err(error(6, 7, PgnErrorKind::UnmatchedCloseParen))
    );
    let second = results[1].clone().expect("the next game parses");
    assert_eq!(mainline(&second), ["d4", "d5"]);
}

#[test]
fn error_lines_count_from_the_start_of_the_input_not_the_game() {
    let results = parse(&format!("{}{}", game("1. e4 ) 1-0"), game("1. d4 ) 1-0")));
    assert_eq!(
        results,
        [
            Err(error(6, 7, PgnErrorKind::UnmatchedCloseParen)),
            Err(error(12, 7, PgnErrorKind::UnmatchedCloseParen)),
        ]
    );
}

// Bare movetext.

#[test]
fn parse_movetext_reads_a_line_with_no_tags_or_marker() {
    let line = parse_movetext("1. e4 e5 2. Nf3 (2. Nc3) Nc6").expect("parses");
    assert_eq!(
        line.mainline().collect::<Vec<_>>(),
        ["e4", "e5", "Nf3", "Nc6"]
    );
    assert_eq!(line.moves[2].variations.len(), 1);
}

#[test]
fn parse_movetext_accepts_a_trailing_termination_marker() {
    let line = parse_movetext("1. e4 e5 *").expect("parses");
    assert_eq!(line.mainline().collect::<Vec<_>>(), ["e4", "e5"]);
}

#[test]
fn parse_movetext_of_nothing_is_an_empty_line() {
    assert_eq!(parse_movetext(""), Ok(Line::default()));
}

#[test]
fn parse_movetext_reports_positions_within_its_own_text() {
    assert_eq!(
        parse_movetext("1. e4 )"),
        Err(error(1, 7, PgnErrorKind::UnmatchedCloseParen))
    );
}
