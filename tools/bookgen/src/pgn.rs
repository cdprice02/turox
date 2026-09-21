//! Parsing PGN game text into structured games, stopping at SAN move
//! tokens: resolving those against an actual position is `san`'s job, not
//! this module's.

use std::collections::VecDeque;
use std::io::BufRead;

/// One parsed game: the header fields the generator's filtering and
/// opening-name attribution use, and the game's SAN move tokens in order.
///
/// Everything else a PGN header carries (site, date, event name, ...) is
/// deliberately not kept here: nothing downstream of this module reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgnGame {
    /// The `WhiteElo` tag, or `None` if absent or unparseable.
    pub white_elo: Option<u32>,
    /// The `BlackElo` tag, or `None` if absent or unparseable.
    pub black_elo: Option<u32>,
    /// The `Result` tag.
    pub result: GameResult,
    /// A human-readable name for this game's opening, if its headers
    /// carried one: the `Opening` tag's own value if present, else the
    /// `ECO` tag's code as a compact fallback identifier, else `None`.
    /// `Opening` wins when both are present since it's the readable name
    /// (e.g. "Ruy Lopez: Berlin Defense") an ECO code alone doesn't convey.
    pub opening_name: Option<String>,
    /// SAN move tokens in play order, move numbers and comments already
    /// stripped. Resolving one against a position is `san::resolve_san`'s
    /// job, not this module's: SAN is context-dependent, and this module
    /// never builds a `Board` at all.
    pub moves: Vec<String>,
}

/// A game's outcome, from the PGN `Result` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// `1-0`.
    WhiteWins,
    /// `0-1`.
    BlackWins,
    /// `1/2-1/2`.
    Draw,
    /// `*`, or a `Result` tag missing or not one of the above.
    Unknown,
}

/// Parses every game in `text`, a PGN file's full contents (one or more
/// games, each a block of `[Tag "value"]` header lines followed by
/// movetext).
///
/// Holds the whole of `text` in memory at once (as both the `&str` itself
/// and, per game, a small [`VecDeque<char>`] over just that game's slice
/// of it), which is fine for a small synthetic fixture or a test but not
/// for a real multi-gigabyte source file; see [`PgnReader`] for the
/// streaming version a real generator run should use instead.
///
/// A game whose headers or movetext don't parse cleanly is dropped rather
/// than aborting the whole run: one malformed game in a multi-million-game
/// source file shouldn't cost every other game in it.
#[must_use]
pub fn parse_pgn(text: &str) -> Vec<PgnGame> {
    let mut games = Vec::new();
    let mut rest = text.chars().collect::<VecDeque<_>>();

    while let Some(game) = parse_one_game(&mut rest) {
        games.push(game);
    }

    games
}

/// Reads PGN games one at a time from `reader`, so a caller processing a
/// large file never holds more than one game's raw text (read into its
/// own small buffer, in memory only until it's parsed and yielded) at
/// once, rather than the whole file plus every game it contains
/// simultaneously the way [`parse_pgn`] does.
///
/// Splits games apart by PGN's own regular shape (a header block, a blank
/// line, movetext, a blank line before the next header block) rather than
/// scanning for `[` the way [`parse_one_game`] does over an
/// already-in-memory `VecDeque`: at this layer nothing has been read yet,
/// so there's no lookahead to scan.
pub struct PgnReader<R> {
    lines: std::io::Lines<R>,
}

impl<R: BufRead> PgnReader<R> {
    /// Wraps `reader`. Doesn't read anything yet; each call to `next`
    /// reads exactly as much as one game needs.
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self {
            lines: reader.lines(),
        }
    }
}

impl<R: BufRead> Iterator for PgnReader<R> {
    type Item = PgnGame;

    fn next(&mut self) -> Option<PgnGame> {
        let mut chunk = String::new();
        let mut in_movetext = false;

        while let Some(Ok(line)) = self.lines.next() {
            if line.trim().is_empty() {
                if chunk.is_empty() {
                    continue; // a blank line before this game has started
                }
                if in_movetext {
                    break; // the blank line ending this game's movetext
                }
                in_movetext = true; // the blank line between headers and movetext
                continue;
            }

            chunk.push_str(&line);
            chunk.push('\n');
        }

        if chunk.is_empty() {
            return None;
        }

        let mut rest: VecDeque<char> = chunk.chars().collect();
        parse_one_game(&mut rest)
    }
}

/// Parses one game (a header block, optionally followed by movetext) from
/// the front of `rest`, consuming exactly what belongs to that game and
/// leaving anything after it untouched. `None` once `rest`, after
/// skipping any leading whitespace, has nothing left to parse.
///
/// The one piece [`parse_pgn`] and [`PgnReader`] share: both eventually
/// need "parse a header block plus movetext out of a character stream",
/// they just differ in how much of the source text is in memory around
/// that stream at any one time.
fn parse_one_game(rest: &mut VecDeque<char>) -> Option<PgnGame> {
    while matches!(rest.front(), Some(c) if c.is_whitespace()) {
        chop_one(rest);
    }
    if rest.is_empty() {
        return None;
    }

    let mut game = PgnGame {
        white_elo: None,
        black_elo: None,
        result: GameResult::Unknown,
        opening_name: None,
        moves: Vec::new(),
    };
    // `Opening` wins over `ECO` regardless of which tag the header names
    // first: PGN's Seven Tag Roster orders them together but doesn't
    // guarantee which comes first, so this is resolved once after the
    // whole header block rather than by "whichever tag is seen first".
    let mut eco: Option<String> = None;

    while rest.front() == Some(&'[') {
        chop_one(rest); // the '[' itself
        let tag_name = split_once(rest, ' ');
        let raw_value = split_once(rest, ']');
        let value = strip_quotes(&raw_value);
        match tag_name.as_str() {
            "WhiteElo" => game.white_elo = value.parse::<u32>().ok(),
            "BlackElo" => game.black_elo = value.parse::<u32>().ok(),
            "Result" => {
                game.result = match value {
                    "1-0" => GameResult::WhiteWins,
                    "0-1" => GameResult::BlackWins,
                    "1/2-1/2" => GameResult::Draw,
                    _ => GameResult::Unknown,
                }
            }
            "Opening" if !value.is_empty() => game.opening_name = Some(value.to_string()),
            "ECO" if !value.is_empty() => eco = Some(value.to_string()),
            _ => {}
        }
        while matches!(rest.front(), Some(c) if c.is_whitespace()) {
            chop_one(rest);
        }
    }
    game.opening_name = game.opening_name.or(eco);

    game.moves = parse_movetext(rest);
    Some(game)
}

/// Pops and returns the next character in `rest`, or `None` at the end.
fn chop_one(rest: &mut VecDeque<char>) -> Option<char> {
    rest.pop_front()
}

/// [`chop_until`], and also pops the delimiter itself, if `rest` had one to
/// pop. A trailing `delim`-less `rest` (the last segment in the text) is
/// not an error: whatever was collected is still returned.
fn split_once(rest: &mut VecDeque<char>, delim: char) -> String {
    let token = chop_until(rest, delim);
    if rest.front() == Some(&delim) {
        rest.pop_front();
    }
    token
}

/// Pops and collects characters from `rest` up to (not including) the next
/// `delim`, or to the end of `rest` if `delim` never appears.
fn chop_until(rest: &mut VecDeque<char>, delim: char) -> String {
    let mut token = String::new();
    while let Some(&c) = rest.front() {
        if c == delim {
            break;
        }
        token.push(c);
        rest.pop_front();
    }
    token
}

/// A tag's `"value"`, with the surrounding quotes stripped. PGN always
/// quotes tag values, so a value missing either quote is malformed and
/// returned as-is rather than guessed at.
fn strip_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .unwrap_or(value)
        .strip_suffix('"')
        .unwrap_or(value)
}

/// Tokenizes one game's movetext (everything after its header block, up to
/// its own result token, the next game's `[`, or the end of the text),
/// discarding everything that isn't a SAN move token: move numbers,
/// `{...}` comments (which may themselves contain anything, including
/// further whitespace or a line break), `$n` NAGs, and the trailing result
/// token itself.
fn parse_movetext(rest: &mut VecDeque<char>) -> Vec<String> {
    let mut moves = Vec::new();

    loop {
        while matches!(rest.front(), Some(c) if c.is_whitespace()) {
            chop_one(rest);
        }

        match rest.front() {
            None | Some('[') => break,
            Some('{') => {
                chop_one(rest); // the '{' itself
                chop_until(rest, '}');
                chop_one(rest); // the '}' itself, if present
                continue;
            }
            Some('$') => {
                chop_one(rest); // the '$' itself
                while matches!(rest.front(), Some(c) if c.is_ascii_digit()) {
                    chop_one(rest);
                }
                continue;
            }
            Some(_) => {}
        }

        let mut word = String::new();
        while let Some(&c) = rest.front() {
            if c.is_whitespace() || c == '{' || c == '[' {
                break;
            }
            word.push(c);
            chop_one(rest);
        }

        if is_result_token(&word) {
            break;
        }
        if !is_move_number(&word) {
            moves.push(word);
        }
    }

    moves
}

/// True for one of PGN's four result tokens, which is how movetext marks
/// its own end: a real game's move list is always followed by one.
fn is_result_token(word: &str) -> bool {
    matches!(word, "1-0" | "0-1" | "1/2-1/2" | "*")
}

/// True for a move-number token (`"1."`, `"12..."`): one or more digits
/// followed by one or more `.`, and nothing else. Real SAN never takes
/// this shape (no legal move is all digits and dots), so this can't
/// misclassify a real move.
fn is_move_number(word: &str) -> bool {
    let digits = word.trim_end_matches('.');
    !digits.is_empty() && digits.len() < word.len() && digits.bytes().all(|b| b.is_ascii_digit())
}
