//! Bytes in, tokens out: every lexical rule of PGN's import format lives
//! here, so the parser never sees whitespace, escape lines, string escapes or
//! the boundary between a SAN token and its suffix glyph.

#![expect(
    clippy::needless_pass_by_ref_mut,
    reason = "the stubs take self mutably for the bodies that will read and advance"
)]
#![expect(
    dead_code,
    reason = "the parser is a stub, so nothing reads tokens yet"
)]

use std::io::BufRead;

use super::{GameResult, PgnError, PgnErrorKind};

/// Where a token starts: 1-based line in the whole input, 1-based byte
/// column within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pos {
    /// 1-based, counted across the whole input rather than per game.
    pub line: usize,
    /// 1-based, in bytes rather than characters.
    pub column: usize,
}

impl Pos {
    /// An error of `kind` reported at this position.
    pub const fn error(self, kind: PgnErrorKind) -> PgnError {
        PgnError {
            line: self.line,
            column: self.column,
            kind,
        }
    }
}

/// One lexical unit of PGN. Text carried in a token is already decoded and
/// unescaped, so the parser compares values and never bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    /// `[`, opening a tag pair.
    LBracket,
    /// `]`, closing a tag pair.
    RBracket,
    /// `(`, opening a variation.
    LParen,
    /// `)`, closing a variation.
    RParen,
    /// A quoted string with its quotes removed and `\"` and `\\` resolved.
    Str(String),
    /// A SAN move or a tag name: a letter or digit, then letters, digits and
    /// `_+#=:-/`. Never all digits, and never a termination marker, which
    /// have tokens of their own.
    Symbol(String),
    /// A move number indication (`12`, `12.`, `12...`), or a run of periods
    /// detached from its number by whitespace. Carries nothing because the
    /// parser ignores it.
    MoveNumber,
    /// `$n`.
    Nag(u8),
    /// A suffix glyph read as the NAG it equals: `!` 1, `?` 2, `!!` 3, `??`
    /// 4, `!?` 5, `?!` 6. Only ever produced directly after a `Symbol`, with
    /// no whitespace between.
    Glyph(u8),
    /// The text between `{` and `}`, or after `;` up to but not including the
    /// line ending.
    Comment(String),
    /// `1-0`, `0-1`, `1/2-1/2` or `*`.
    Termination(GameResult),
}

/// Turns a byte stream into [`Token`]s, tracking each one's position.
///
/// Reads through `BufRead` one byte at a time with `fill_buf` and `consume`,
/// so a refill can land anywhere, mid-token included, without changing what
/// is scanned.
pub(super) struct Scanner<R> {
    /// The source bytes.
    reader: R,
    /// The line of the next unread byte.
    line: usize,
    /// The column of the next unread byte.
    column: usize,
    /// True when the next unread byte starts a line, which is the only place
    /// a `%` escapes the rest of it.
    at_line_start: bool,
    /// One token of lookahead, filled by `peek` and drained by `next`.
    peeked: Option<(Token, Pos)>,
}

impl<R: BufRead> Scanner<R> {
    /// Wraps `reader`, positioned at line 1, column 1.
    pub const fn new(reader: R) -> Self {
        Self {
            reader,
            line: 1,
            column: 1,
            at_line_start: true,
            peeked: None,
        }
    }

    /// The next token and where it starts, without consuming it. `None` at
    /// the end of input.
    #[expect(clippy::todo, reason = "the scanner is a stub")]
    pub fn peek(&mut self) -> Result<Option<&(Token, Pos)>, PgnError> {
        todo!("fill self.peeked if empty, then borrow it")
    }

    /// The next token and where it starts, consuming it. `None` at the end of
    /// input.
    #[expect(clippy::todo, reason = "the scanner is a stub")]
    pub fn next(&mut self) -> Result<Option<(Token, Pos)>, PgnError> {
        todo!("take self.peeked, or scan a token")
    }

    /// Discards input up to the next `[` that starts a line, or to the end,
    /// and drops any peeked token.
    ///
    /// Works on raw bytes rather than tokens, because the input it is
    /// recovering from may not tokenize: an unclosed comment or string is
    /// exactly what can leave a reader here.
    #[expect(clippy::todo, reason = "the scanner is a stub")]
    pub fn skip_to_next_game(&mut self) {
        todo!("skip bytes until a `[` with at_line_start set")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    /// Every token in `text`, or the first error.
    fn tokens(text: &str) -> Result<Vec<Token>, PgnError> {
        Ok(spanned(text)?.into_iter().map(|(token, _)| token).collect())
    }

    /// Every token in `text` with its position, or the first error.
    fn spanned(text: &str) -> Result<Vec<(Token, Pos)>, PgnError> {
        let mut scanner = Scanner::new(text.as_bytes());
        let mut out = Vec::new();
        while let Some(token) = scanner.next()? {
            out.push(token);
        }
        Ok(out)
    }

    fn sym(text: &str) -> Token {
        Token::Symbol(text.to_string())
    }

    fn comment(text: &str) -> Token {
        Token::Comment(text.to_string())
    }

    const fn at(line: usize, column: usize) -> Pos {
        Pos { line, column }
    }

    #[test]
    fn empty_and_blank_input_has_no_tokens() {
        assert_eq!(tokens(""), Ok(vec![]));
        assert_eq!(tokens(" \n\t\r\n  "), Ok(vec![]));
    }

    #[test]
    fn brackets_and_parentheses_are_single_byte_tokens() {
        assert_eq!(
            tokens("[]()"),
            Ok(vec![
                Token::LBracket,
                Token::RBracket,
                Token::LParen,
                Token::RParen
            ])
        );
    }

    #[test]
    fn a_tag_pair_is_four_tokens() {
        assert_eq!(
            tokens(r#"[Event "Rated blitz game"]"#),
            Ok(vec![
                Token::LBracket,
                sym("Event"),
                Token::Str("Rated blitz game".to_string()),
                Token::RBracket,
            ])
        );
    }

    #[test]
    fn string_escapes_are_resolved() {
        assert_eq!(
            tokens(r#""a \"b\" \\ c""#),
            Ok(vec![Token::Str(r#"a "b" \ c"#.to_string())])
        );
    }

    #[test]
    fn an_escaped_backslash_before_the_closing_quote_closes_the_string() {
        assert_eq!(
            tokens(r#""ends \\" x"#),
            Ok(vec![Token::Str(r"ends \".to_string()), sym("x")])
        );
    }

    #[test]
    fn brackets_inside_a_string_are_string_text() {
        assert_eq!(
            tokens(r#""[a] ]b[""#),
            Ok(vec![Token::Str("[a] ]b[".to_string())])
        );
    }

    #[test]
    fn a_string_unclosed_at_end_of_line_is_reported_at_its_quote() {
        assert_eq!(
            tokens("x \"abc\n\"rest"),
            Err(at(1, 3).error(PgnErrorKind::UnterminatedString))
        );
    }

    #[test]
    fn a_string_unclosed_at_end_of_input_is_reported_at_its_quote() {
        assert_eq!(
            tokens("\"abc"),
            Err(at(1, 1).error(PgnErrorKind::UnterminatedString))
        );
    }

    #[test]
    fn move_numbers_in_every_form_are_one_token() {
        assert_eq!(
            tokens("1. 12... 3 4.e4 5...e5"),
            Ok(vec![
                Token::MoveNumber,
                Token::MoveNumber,
                Token::MoveNumber,
                Token::MoveNumber,
                sym("e4"),
                Token::MoveNumber,
                sym("e5"),
            ])
        );
    }

    #[test]
    fn periods_detached_from_their_number_are_a_move_number() {
        assert_eq!(
            tokens("6 . Bb5 7 ... a6"),
            Ok(vec![
                Token::MoveNumber,
                Token::MoveNumber,
                sym("Bb5"),
                Token::MoveNumber,
                Token::MoveNumber,
                sym("a6"),
            ])
        );
    }

    #[test]
    fn termination_markers_have_their_own_token() {
        assert_eq!(
            tokens("1-0 0-1 1/2-1/2 *"),
            Ok(vec![
                Token::Termination(GameResult::WhiteWins),
                Token::Termination(GameResult::BlackWins),
                Token::Termination(GameResult::Draw),
                Token::Termination(GameResult::Unknown),
            ])
        );
    }

    #[test]
    fn san_keeps_check_mate_promotion_and_castling_characters() {
        assert_eq!(
            tokens("O-O O-O-O+ Qxf7# e8=Q+ dxc1=N R1a3 Nbd7"),
            Ok(vec![
                sym("O-O"),
                sym("O-O-O+"),
                sym("Qxf7#"),
                sym("e8=Q+"),
                sym("dxc1=N"),
                sym("R1a3"),
                sym("Nbd7"),
            ])
        );
    }

    #[test]
    fn every_suffix_glyph_becomes_its_nag() {
        assert_eq!(
            tokens("a! b? c!! d?? e!? f?!"),
            Ok(vec![
                sym("a"),
                Token::Glyph(1),
                sym("b"),
                Token::Glyph(2),
                sym("c"),
                Token::Glyph(3),
                sym("d"),
                Token::Glyph(4),
                sym("e"),
                Token::Glyph(5),
                sym("f"),
                Token::Glyph(6),
            ])
        );
    }

    #[test]
    fn a_glyph_follows_check_and_mate_marks() {
        assert_eq!(
            tokens("Qxf7#! e8=Q+!?"),
            Ok(vec![
                sym("Qxf7#"),
                Token::Glyph(1),
                sym("e8=Q+"),
                Token::Glyph(5)
            ])
        );
    }

    #[test]
    fn a_glyph_separated_from_its_move_is_unexpected() {
        assert_eq!(
            tokens("e4 !"),
            Err(at(1, 4).error(PgnErrorKind::UnexpectedByte(b'!')))
        );
    }

    #[test]
    fn nags_parse_their_number() {
        assert_eq!(
            tokens("$0 $14 $255 e4$1"),
            Ok(vec![
                Token::Nag(0),
                Token::Nag(14),
                Token::Nag(255),
                sym("e4"),
                Token::Nag(1),
            ])
        );
    }

    #[test]
    fn a_dollar_with_no_digits_is_an_invalid_nag() {
        assert_eq!(
            tokens("e4 $ e5"),
            Err(at(1, 4).error(PgnErrorKind::InvalidNag))
        );
    }

    #[test]
    fn a_nag_above_255_is_invalid() {
        assert_eq!(
            tokens("e4 $256"),
            Err(at(1, 4).error(PgnErrorKind::InvalidNag))
        );
    }

    #[test]
    fn a_brace_comment_is_kept_verbatim() {
        assert_eq!(
            tokens("{ a  b }{\n\nc}"),
            Ok(vec![comment(" a  b "), comment("\n\nc")])
        );
    }

    #[test]
    fn a_semicolon_comment_stops_before_the_line_ending() {
        assert_eq!(
            tokens(";rest of line\nx ;crlf\r\ny"),
            Ok(vec![
                comment("rest of line"),
                sym("x"),
                comment("crlf"),
                sym("y")
            ])
        );
    }

    #[test]
    fn a_semicolon_comment_at_end_of_input_ends_there() {
        assert_eq!(tokens("x ;tail"), Ok(vec![sym("x"), comment("tail")]));
    }

    #[test]
    fn comment_delimiters_do_not_nest_or_mix() {
        assert_eq!(
            tokens("{a ; b ( c} ;d { e\nf"),
            Ok(vec![comment("a ; b ( c"), comment("d { e"), sym("f")])
        );
    }

    #[test]
    fn a_comment_glued_to_a_move_ends_the_move() {
        assert_eq!(
            tokens("e4{c}e5;d"),
            Ok(vec![sym("e4"), comment("c"), sym("e5"), comment("d")])
        );
    }

    #[test]
    fn an_unclosed_brace_comment_is_reported_at_its_brace() {
        assert_eq!(
            tokens("e4\n  {never closed"),
            Err(at(2, 3).error(PgnErrorKind::UnterminatedComment))
        );
    }

    #[test]
    fn a_percent_line_is_skipped() {
        assert_eq!(
            tokens("%first line\ne4\n%skipped e5\nNf3"),
            Ok(vec![sym("e4"), sym("Nf3")])
        );
    }

    #[test]
    fn a_percent_after_column_one_is_unexpected() {
        assert_eq!(
            tokens("e4\n %x"),
            Err(at(2, 2).error(PgnErrorKind::UnexpectedByte(b'%')))
        );
    }

    #[test]
    fn a_percent_at_the_start_of_a_line_inside_a_comment_is_comment_text() {
        assert_eq!(tokens("{\n%in}"), Ok(vec![comment("\n%in")]));
    }

    #[test]
    fn reserved_and_stray_bytes_are_unexpected() {
        for (text, byte) in [("<", b'<'), (">", b'>'), ("@", b'@'), ("}", b'}')] {
            assert_eq!(
                tokens(text),
                Err(at(1, 1).error(PgnErrorKind::UnexpectedByte(byte))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn positions_mark_where_each_token_starts() {
        assert_eq!(
            spanned("1. e4\n  {c} (\n\"s\""),
            Ok(vec![
                (Token::MoveNumber, at(1, 1)),
                (sym("e4"), at(1, 4)),
                (comment("c"), at(2, 3)),
                (Token::LParen, at(2, 7)),
                (Token::Str("s".to_string()), at(3, 1)),
            ])
        );
    }

    #[test]
    fn positions_count_lines_inside_multi_line_tokens() {
        assert_eq!(
            spanned("{a\nb} e4\r\n e5"),
            Ok(vec![
                (comment("a\nb"), at(1, 1)),
                (sym("e4"), at(2, 4)),
                (sym("e5"), at(3, 2)),
            ])
        );
    }

    #[test]
    fn a_glyph_is_positioned_at_its_first_byte() {
        assert_eq!(
            spanned("Nf3?!"),
            Ok(vec![(sym("Nf3"), at(1, 1)), (Token::Glyph(6), at(1, 4))])
        );
    }

    #[test]
    fn peek_does_not_consume() {
        let mut scanner = Scanner::new(&b"e4 e5"[..]);
        let first = scanner.peek().expect("scans").cloned();
        let again = scanner.peek().expect("scans").cloned();
        assert_eq!(first, again);
        assert_eq!(scanner.next().expect("scans"), first);
        assert_eq!(scanner.next().expect("scans"), Some((sym("e5"), at(1, 4))));
        assert_eq!(scanner.next().expect("scans"), None);
    }

    #[test]
    fn text_decodes_as_utf8_and_lossily_on_a_bad_byte() {
        assert_eq!(
            tokens("\"Ren\u{e9}\" {caf\u{e9} \u{2192}}"),
            Ok(vec![
                Token::Str("Ren\u{e9}".to_string()),
                comment("caf\u{e9} \u{2192}")
            ])
        );
        let mut scanner = Scanner::new(&b"{bad \xff byte}"[..]);
        assert_eq!(
            scanner.next().expect("a bad byte in a comment still scans"),
            Some((comment("bad \u{fffd} byte"), at(1, 1)))
        );
    }

    #[test]
    fn a_one_byte_buffer_scans_identically() {
        // Every refill boundary lands mid-token somewhere in here: inside a
        // string escape, a CRLF, a glyph pair, a NAG's digits, a comment.
        let text = "[Event \"a \\\"q\\\" b\"]\r\n%esc\n1. e4!? $14 {c\n} (1... c5) ;x\r\n1/2-1/2";
        let mut one_byte = Scanner::new(BufReader::with_capacity(1, text.as_bytes()));
        let mut buffered = Vec::new();
        while let Some(token) = one_byte.next().expect("scans") {
            buffered.push(token);
        }
        assert_eq!(Ok(buffered), spanned(text));
    }

    #[test]
    fn skip_to_next_game_stops_at_a_bracket_starting_a_line() {
        let mut scanner = Scanner::new(&b"x ) [not this\n{ unclosed\n[Event"[..]);
        scanner.skip_to_next_game();
        assert_eq!(
            scanner.next().expect("scans"),
            Some((Token::LBracket, at(3, 1)))
        );
    }

    #[test]
    fn skip_to_next_game_drops_a_peeked_token() {
        let mut scanner = Scanner::new(&b"e4\n[Event"[..]);
        assert!(scanner.peek().expect("scans").is_some(), "e4 is peeked");
        scanner.skip_to_next_game();
        assert_eq!(
            scanner.next().expect("scans"),
            Some((Token::LBracket, at(2, 1)))
        );
    }

    #[test]
    fn skip_to_next_game_at_a_bracket_already_starting_a_line_stays_there() {
        // The parser stops a game by peeking the next game's `[`; recovering
        // from an error there must not skip that game too.
        let mut scanner = Scanner::new(&b"[Event"[..]);
        scanner.skip_to_next_game();
        assert_eq!(
            scanner.next().expect("scans"),
            Some((Token::LBracket, at(1, 1)))
        );
    }

    #[test]
    fn skip_to_next_game_with_no_next_game_reaches_the_end() {
        let mut scanner = Scanner::new(&b"e4 ) e5\n"[..]);
        scanner.skip_to_next_game();
        assert_eq!(scanner.next().expect("scans"), None);
    }
}
