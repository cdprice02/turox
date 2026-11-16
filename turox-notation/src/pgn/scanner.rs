//! Bytes in, tokens out: every lexical rule of PGN's import format lives
//! here, so the parser never sees whitespace, escape lines, string escapes or
//! the boundary between a SAN token and its suffix glyph.

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

impl Pos {
    /// Moves past `bytes`. The one place position is tracked, so every way of
    /// consuming input agrees on where it ends up.
    fn advance(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }
}

/// Whether `byte` may continue a SAN move or a tag name.
const fn is_symbol_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'#' | b'=' | b':' | b'-' | b'/')
}

/// Decodes collected bytes as UTF-8, replacing invalid sequences rather than
/// failing: a bad byte in a comment should not cost its game.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Turns a byte stream into [`Token`]s, tracking each one's position.
///
/// Reads through `BufRead`'s `fill_buf` and `consume` rather than `lines` or
/// `split`: those take the reader by value, `lines` fails on invalid UTF-8,
/// and a token can span lines or touch its neighbours with no space between.
pub(super) struct Scanner<R> {
    /// The source bytes.
    reader: R,
    /// Where the next unread byte is. A field of its own, not loose counters
    /// on `Scanner`, so it can advance while a slice borrowed from `reader`'s
    /// buffer is still alive: a `&mut self` method could not.
    pos: Pos,
    /// One token of lookahead, filled by `peek` and drained by `next`.
    peeked: Option<(Token, Pos)>,
    /// Where the last token ended, if it was a `Symbol`. A suffix glyph is
    /// only a glyph when it starts exactly here; anywhere else a `!` or `?`
    /// is a stray byte.
    symbol_end: Option<Pos>,
    /// How many `%` escape lines have been skipped so far. The parser reads
    /// the difference across a gap to tell a blank line from an escape line,
    /// since both leave the same hole in the line numbers.
    escape_lines: usize,
}

impl<R: BufRead> Scanner<R> {
    /// Wraps `reader`, positioned at line 1, column 1.
    pub const fn new(reader: R) -> Self {
        Self {
            reader,
            pos: Pos { line: 1, column: 1 },
            peeked: None,
            symbol_end: None,
            escape_lines: 0,
        }
    }

    /// How many `%` escape lines the scanner has skipped so far.
    pub const fn escape_lines(&self) -> usize {
        self.escape_lines
    }

    /// The next token and where it starts, without consuming it. `None` at
    /// the end of input.
    pub fn peek(&mut self) -> Result<Option<&(Token, Pos)>, PgnError> {
        // Checked and then filled, not `if let Some(t) = &self.peeked {
        // return .. }`: returning a borrow from one branch and assigning in
        // the other is a case the borrow checker rejects.
        if self.peeked.is_none() {
            self.peeked = self.scan()?;
        }
        Ok(self.peeked.as_ref())
    }

    /// The next token and where it starts, consuming it. `None` at the end of
    /// input.
    pub fn next(&mut self) -> Result<Option<(Token, Pos)>, PgnError> {
        self.peeked
            .take()
            .map_or_else(|| self.scan(), |token| Ok(Some(token)))
    }

    /// Reads one token from the bytes, ignoring `peeked`.
    fn scan(&mut self) -> Result<Option<(Token, Pos)>, PgnError> {
        let symbol_end = self.symbol_end.take();
        loop {
            self.skip_while(|byte| byte.is_ascii_whitespace())?;
            if self.pos.column == 1 && self.peek_byte()? == Some(b'%') {
                self.skip_while(|byte| byte != b'\n')?;
                self.escape_lines += 1;
            } else {
                break;
            }
        }
        let start = self.pos;
        let Some(byte) = self.peek_byte()? else {
            return Ok(None);
        };
        let token = match byte {
            b'[' => self.single(Token::LBracket)?,
            b']' => self.single(Token::RBracket)?,
            b'(' => self.single(Token::LParen)?,
            b')' => self.single(Token::RParen)?,
            b'*' => self.single(Token::Termination(GameResult::Unknown))?,
            b'"' => self.string(start)?,
            b'{' => self.brace_comment(start)?,
            b';' => {
                self.next_byte()?;
                let mut text = Vec::new();
                self.take_while(|byte| byte != b'\n' && byte != b'\r', &mut text)?;
                Token::Comment(lossy(&text))
            }
            b'$' => self.nag(start)?,
            b'.' => {
                self.skip_while(|byte| byte == b'.')?;
                Token::MoveNumber
            }
            b'!' | b'?' if symbol_end == Some(start) => self.glyph(byte)?,
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                let mut text = Vec::new();
                self.take_while(is_symbol_byte, &mut text)?;
                if text.iter().all(u8::is_ascii_digit) {
                    self.skip_while(|byte| byte == b'.')?;
                    Token::MoveNumber
                } else {
                    match text.as_slice() {
                        b"1-0" => Token::Termination(GameResult::WhiteWins),
                        b"0-1" => Token::Termination(GameResult::BlackWins),
                        b"1/2-1/2" => Token::Termination(GameResult::Draw),
                        _ => {
                            self.symbol_end = Some(self.pos);
                            Token::Symbol(lossy(&text))
                        }
                    }
                }
            }
            _ => return Err(start.error(PgnErrorKind::UnexpectedByte(byte))),
        };
        Ok(Some((token, start)))
    }

    /// Consumes the one byte already peeked and returns `token`.
    fn single(&mut self, token: Token) -> Result<Token, PgnError> {
        self.next_byte()?;
        Ok(token)
    }

    /// A quoted string starting at `start`. Ends at the closing quote; a
    /// line ending first is `UnterminatedString`, because tag values are
    /// single-line and a missing quote would otherwise swallow the rest of
    /// the file. A backslash escapes only `"` and `\`; before anything else
    /// it is kept as a literal backslash.
    fn string(&mut self, start: Pos) -> Result<Token, PgnError> {
        self.next_byte()?;
        let mut text = Vec::new();
        loop {
            match self.next_byte()? {
                None | Some(b'\n') => {
                    return Err(start.error(PgnErrorKind::UnterminatedString));
                }
                Some(b'"') => break,
                Some(b'\\') => match self.peek_byte()? {
                    Some(escaped @ (b'"' | b'\\')) => {
                        self.next_byte()?;
                        text.push(escaped);
                    }
                    _ => text.push(b'\\'),
                },
                Some(byte) => text.push(byte),
            }
        }
        Ok(Token::Str(lossy(&text)))
    }

    /// A `{...}` comment starting at `start`. Does not nest, and runs across
    /// line endings.
    fn brace_comment(&mut self, start: Pos) -> Result<Token, PgnError> {
        self.next_byte()?;
        let mut text = Vec::new();
        self.take_while(|byte| byte != b'}', &mut text)?;
        if self.next_byte()? != Some(b'}') {
            return Err(start.error(PgnErrorKind::UnterminatedComment));
        }
        Ok(Token::Comment(lossy(&text)))
    }

    /// `$` and its digits, starting at `start`. Anything that is not a number
    /// from 0 to 255, including no digits at all, is `InvalidNag`.
    fn nag(&mut self, start: Pos) -> Result<Token, PgnError> {
        self.next_byte()?;
        let mut digits = Vec::new();
        self.take_while(|byte| byte.is_ascii_digit(), &mut digits)?;
        std::str::from_utf8(&digits)
            .ok()
            .and_then(|text| text.parse().ok())
            .map(Token::Nag)
            .ok_or_else(|| start.error(PgnErrorKind::InvalidNag))
    }

    /// A suffix glyph whose first byte, `first`, is peeked and not yet
    /// consumed: `!` or `?`, optionally followed by one more of either.
    fn glyph(&mut self, first: u8) -> Result<Token, PgnError> {
        self.next_byte()?;
        let second = match self.peek_byte()? {
            Some(byte @ (b'!' | b'?')) => {
                self.next_byte()?;
                Some(byte)
            }
            _ => None,
        };
        Ok(Token::Glyph(match (first, second) {
            (b'!', Some(b'!')) => 3,
            (b'!', Some(_)) => 5,
            (b'!', None) => 1,
            (b'?', Some(b'?')) => 4,
            (_, None) => 2,
            (_, Some(_)) => 6,
        }))
    }

    /// Discards input up to the next `[` that starts a line, or to the end,
    /// and drops any peeked token.
    ///
    /// Works on raw bytes rather than tokens, because the input it is
    /// recovering from may not tokenize: an unclosed comment or string is
    /// exactly what can leave a reader here.
    ///
    /// A peeked `[` that starts a line is kept rather than dropped. A line
    /// stops by peeking the next game's bracket, so an error found there (a
    /// variation still open, say) has that bracket already read out of
    /// `reader`, and dropping it would skip a game that parses.
    ///
    /// An I/O error stops the skip where it is; the next read meets it again
    /// and reports it.
    pub fn skip_to_next_game(&mut self) {
        if matches!(self.peeked, Some((Token::LBracket, Pos { column: 1, .. }))) {
            return;
        }
        self.peeked = None;
        loop {
            if self.skip_while(|byte| byte != b'[').is_err() {
                return;
            }
            match self.peek_byte() {
                Ok(Some(_)) if self.pos.column != 1 => {
                    let _ = self.next_byte();
                }
                _ => return,
            }
        }
    }

    /// The next byte, without consuming it. `None` at the end of input.
    fn peek_byte(&mut self) -> Result<Option<u8>, PgnError> {
        let pos = self.pos;
        let buf = self
            .reader
            .fill_buf()
            .map_err(|err| pos.error(PgnErrorKind::Io(err.kind())))?;
        Ok(buf.first().copied())
    }

    /// The next byte, consuming it and advancing `pos` past it. `None` at the
    /// end of input.
    fn next_byte(&mut self) -> Result<Option<u8>, PgnError> {
        let byte = self.peek_byte()?;
        if let Some(byte) = byte {
            self.reader.consume(1);
            self.pos.advance(&[byte]);
        }
        Ok(byte)
    }

    /// Appends bytes to `out` while `keep` holds, stopping before the first
    /// byte it rejects (left unread, for the caller to inspect) or at the end
    /// of input.
    fn take_while(
        &mut self,
        keep: impl FnMut(u8) -> bool,
        out: &mut Vec<u8>,
    ) -> Result<(), PgnError> {
        self.consume_while(keep, Some(out))
    }

    /// [`Self::take_while`] with nowhere to put the bytes.
    fn skip_while(&mut self, keep: impl FnMut(u8) -> bool) -> Result<(), PgnError> {
        self.consume_while(keep, None)
    }

    /// Consumes a whole buffered chunk at a time rather than a byte at a
    /// time, refilling only when `keep` accepted all of it.
    fn consume_while(
        &mut self,
        mut keep: impl FnMut(u8) -> bool,
        mut out: Option<&mut Vec<u8>>,
    ) -> Result<(), PgnError> {
        loop {
            let pos = self.pos;
            let buf = self
                .reader
                .fill_buf()
                .map_err(|err| pos.error(PgnErrorKind::Io(err.kind())))?;
            let available = buf.len();
            let taken = buf
                .iter()
                .position(|&byte| !keep(byte))
                .unwrap_or(available);
            if let Some(out) = out.as_deref_mut() {
                out.extend_from_slice(&buf[..taken]);
            }
            self.pos.advance(&buf[..taken]);
            self.reader.consume(taken);
            if taken < available || available == 0 {
                return Ok(());
            }
        }
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
    fn skip_to_next_game_keeps_a_peeked_bracket_starting_a_line() {
        // The parser peeks the next game's `[` to end a line, then reports an
        // error about the line; recovery must leave that game to be read.
        let mut scanner = Scanner::new(&b"e4\n[Event"[..]);
        assert_eq!(scanner.next().expect("scans"), Some((sym("e4"), at(1, 1))));
        assert_eq!(
            scanner.peek().expect("scans"),
            Some(&(Token::LBracket, at(2, 1)))
        );
        scanner.skip_to_next_game();
        assert_eq!(
            scanner.next().expect("scans"),
            Some((Token::LBracket, at(2, 1)))
        );
    }

    #[test]
    fn skip_to_next_game_with_no_next_game_reaches_the_end() {
        let mut scanner = Scanner::new(&b"e4 ) e5\n"[..]);
        scanner.skip_to_next_game();
        assert_eq!(scanner.next().expect("scans"), None);
    }
}
