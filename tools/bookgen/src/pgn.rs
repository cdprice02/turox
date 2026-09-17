//! Parsing PGN game text into structured games, stopping at SAN move
//! tokens: resolving those against an actual position is `san`'s job, not
//! this module's.

use std::collections::VecDeque;

/// One parsed game: the two header fields the generator's filtering
/// actually uses, and the game's SAN move tokens in order.
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

/// Pops and returns the next character in `rest`, or `None` at the end.
fn chop_one(rest: &mut VecDeque<char>) -> Option<char> {
    rest.pop_front()
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

/// True for a move-number token (`"1."`, `"12..."`): one or more digits
/// followed by one or more `.`, and nothing else. Real SAN never takes
/// this shape (no legal move is all digits and dots), so this can't
/// misclassify a real move.
fn is_move_number(word: &str) -> bool {
    let digits = word.trim_end_matches('.');
    !digits.is_empty() && digits.len() < word.len() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// True for one of PGN's four result tokens, which is how movetext marks
/// its own end: a real game's move list is always followed by one.
fn is_result_token(word: &str) -> bool {
    matches!(word, "1-0" | "0-1" | "1/2-1/2" | "*")
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

/// Parses every game in `text`, a PGN file's full contents (one or more
/// games, each a block of `[Tag "value"]` header lines followed by
/// movetext).
///
/// A game whose headers or movetext don't parse cleanly is dropped rather
/// than aborting the whole run: one malformed game in a multi-million-game
/// source file shouldn't cost every other game in it.
#[must_use]
pub fn parse_pgn(text: &str) -> Vec<PgnGame> {
    let mut games = Vec::new();
    let mut rest = text.chars().collect::<VecDeque<_>>();

    // note: ignores an incomplete game at the end of the text
    loop {
        while matches!(rest.front(), Some(c) if c.is_whitespace()) {
            chop_one(&mut rest);
        }
        if rest.is_empty() {
            break;
        }

        let mut game = PgnGame {
            white_elo: None,
            black_elo: None,
            result: GameResult::Unknown,
            moves: Vec::new(),
        };

        while rest.front() == Some(&'[') {
            chop_one(&mut rest); // the '[' itself
            let tag_name = split_once(&mut rest, ' ');
            let raw_value = split_once(&mut rest, ']');
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
                _ => {}
            }
            while matches!(rest.front(), Some(c) if c.is_whitespace()) {
                chop_one(&mut rest);
            }
        }

        game.moves = parse_movetext(&mut rest);
        games.push(game);
    }

    games
}
