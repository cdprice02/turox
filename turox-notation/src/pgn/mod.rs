//! Parsing PGN text into structured games, stopping at SAN move tokens:
//! resolving those against an actual position is `san`'s job, not this
//! module's, so nothing here ever builds a `Board`.
//!
//! [`PgnGame`] keeps everything PGN's import grammar defines (every tag pair,
//! comments, NAGs and variations) rather than only the mainline, so a test or
//! a fuzz target can check what was parsed and not only that a parse
//! happened. Callers that want the old flat view use the accessors on
//! [`PgnGame`] and [`Line::mainline`].

use std::io::BufRead;

/// One game: its tag pairs and its movetext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgnGame {
    /// Every tag pair, in file order.
    pub tags: Tags,
    /// The mainline, with variations and annotations hanging off its moves.
    pub movetext: Line,
    /// The termination marker that ends the movetext.
    ///
    /// Kept apart from the `Result` tag, which [`PgnGame::result`] reads,
    /// because the two are written independently and can disagree. `None`
    /// when the input ended, or the next game's tags began, before a marker
    /// appeared.
    pub termination: Option<GameResult>,
}

/// A game's tag pairs, in the order the file gives them.
///
/// A list rather than a map: order is part of the file, the standard does
/// not forbid a repeated name, and a game carries few enough tags that a
/// linear lookup is not worth a hash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags(pub Vec<TagPair>);

/// One `[Name "value"]` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagPair {
    /// The tag name, case-sensitive as the standard requires.
    pub name: String,
    /// The value with its quotes removed and `\"` and `\\` unescaped.
    pub value: String,
}

/// A sequence of moves: the mainline, or one variation.
///
/// One type for both because the grammar uses one rule for both: a
/// variation is an element sequence like the mainline, and may itself hold
/// variations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    /// Comments before the first move: a note on the whole game for the
    /// mainline, an introduction for a variation.
    pub comments: Vec<String>,
    /// The line's moves in play order.
    pub moves: Vec<MoveNode>,
}

/// One move and the annotations attached to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveNode {
    /// The SAN token as written, with any suffix glyph removed and check or
    /// mate marks kept, since `+` and `#` are SAN rather than annotation.
    pub san: String,
    /// Numeric annotation glyphs in the order they appeared. A suffix glyph
    /// is stored as the NAG the standard defines it to equal: `!` is 1, `?`
    /// 2, `!!` 3, `??` 4, `!?` 5, `?!` 6.
    pub nags: Vec<u8>,
    /// Comments after this move, verbatim between their delimiters, in
    /// order. Includes comments after this move's variations, since nothing
    /// else precedes them to attach to.
    pub comments: Vec<String>,
    /// Alternatives to this move, each played from the position before it.
    /// A variation replaces the move it follows, so it belongs to that move
    /// rather than to a position in the line.
    pub variations: Vec<Line>,
}

/// A game's outcome, as a termination marker or a `Result` tag writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// `1-0`.
    WhiteWins,
    /// `0-1`.
    BlackWins,
    /// `1/2-1/2`.
    Draw,
    /// `*`: unknown, ongoing, or abandoned. Also what [`PgnGame::result`]
    /// reports for a `Result` tag that is missing or is none of the four.
    Unknown,
}

/// Why a game failed to parse, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgnError {
    /// 1-based line in the whole input, not within the game.
    pub line: usize,
    /// 1-based byte column within that line.
    pub column: usize,
    /// What went wrong at that position.
    pub kind: PgnErrorKind,
}

/// The kinds of malformed input, each reported at the position that
/// identifies it: the byte that opened an unclosed construct, otherwise the
/// offending byte itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgnErrorKind {
    /// A `[` not followed by a tag name, a quoted value and a `]`.
    MalformedTagPair,
    /// A tag value's opening `"` with no closing quote before the end of its
    /// line.
    UnterminatedString,
    /// A `{` with no `}` before the end of input.
    UnterminatedComment,
    /// A `(` with no matching `)` before the game's termination marker or
    /// the end of input.
    UnclosedVariation,
    /// A `)` with no open variation to close.
    UnmatchedCloseParen,
    /// A `(` before any move of its line, which leaves it nothing to
    /// replace.
    VariationWithoutMove,
    /// A NAG before any move of its line, which leaves it nothing to
    /// annotate.
    NagWithoutMove,
    /// A `$` not followed by a number from 0 to 255.
    InvalidNag,
    /// A byte that starts no token, including the reserved `<` and `>`.
    UnexpectedByte(u8),
    /// The underlying reader failed. Kept as the kind alone because
    /// [`std::io::Error`] is neither `Clone` nor `Eq`.
    Io(std::io::ErrorKind),
}

impl Tags {
    /// The value of the first tag named `name`, matched case-sensitively.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|pair| pair.name == name)
            .map(|pair| pair.value.as_str())
    }
}

impl Line {
    /// The line's SAN tokens in play order, ignoring every annotation and
    /// variation.
    pub fn mainline(&self) -> impl Iterator<Item = &str> {
        self.moves.iter().map(|node| node.san.as_str())
    }
}

impl PgnGame {
    /// The `WhiteElo` tag, or `None` if absent or not a number.
    #[must_use]
    pub fn white_elo(&self) -> Option<u32> {
        self.tags.get("WhiteElo")?.parse().ok()
    }

    /// The `BlackElo` tag, or `None` if absent or not a number.
    #[must_use]
    pub fn black_elo(&self) -> Option<u32> {
        self.tags.get("BlackElo")?.parse().ok()
    }

    /// The `Result` tag. See [`PgnGame::termination`] for the marker that
    /// ends the movetext, which can disagree with it.
    #[must_use]
    pub fn result(&self) -> GameResult {
        match self.tags.get("Result") {
            Some("1-0") => GameResult::WhiteWins,
            Some("0-1") => GameResult::BlackWins,
            Some("1/2-1/2") => GameResult::Draw,
            _ => GameResult::Unknown,
        }
    }

    /// A human-readable name for the game's opening: the `Opening` tag if
    /// present and non-empty, else the `ECO` code as a compact fallback.
    #[must_use]
    pub fn opening_name(&self) -> Option<&str> {
        let non_empty = |name| self.tags.get(name).filter(|value| !value.is_empty());
        non_empty("Opening").or_else(|| non_empty("ECO"))
    }

    /// The `FEN` tag the game starts from, or `None` for the standard
    /// starting position.
    ///
    /// The standard pairs `FEN` with `SetUp "1"`, but exporters routinely
    /// omit `SetUp`, and replaying a set-up game from the initial position
    /// is silently wrong. So a `FEN` tag counts unless `SetUp` explicitly
    /// says `0`.
    #[must_use]
    pub fn starting_fen(&self) -> Option<&str> {
        if self.tags.get("SetUp") == Some("0") {
            return None;
        }
        self.tags.get("FEN")
    }

    /// The mainline's SAN tokens. Shorthand for [`Line::mainline`] on
    /// [`PgnGame::movetext`].
    pub fn mainline(&self) -> impl Iterator<Item = &str> {
        self.movetext.mainline()
    }
}

/// Reads PGN games one at a time from `reader`, holding no more than one
/// game's parsed data at once.
///
/// A malformed game yields an `Err` and the reader carries on from the next
/// line that starts with `[`, so one bad game in a multi-gigabyte source
/// costs only itself.
pub struct PgnReader<R> {
    /// The source bytes. Scanned rather than split into lines first, because
    /// a comment or variation may span blank lines and only the grammar knows
    /// where a game ends.
    #[expect(dead_code, reason = "read by the scanner, which is a stub")]
    reader: R,
}

impl<R: BufRead> PgnReader<R> {
    /// Wraps `reader`. Reads nothing until the first call to `next`.
    #[must_use]
    pub const fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: BufRead> Iterator for PgnReader<R> {
    type Item = Result<PgnGame, PgnError>;

    #[expect(clippy::todo, reason = "the scanner and parser are stubs")]
    fn next(&mut self) -> Option<Self::Item> {
        todo!("scan and parse one game")
    }
}

/// Parses every game in `text`.
///
/// The same parser as [`PgnReader`], over the text's bytes, so the two
/// cannot disagree.
#[must_use]
pub const fn parse_pgn(text: &str) -> PgnReader<&[u8]> {
    PgnReader::new(text.as_bytes())
}

/// Parses a bare movetext section, one with no tag pairs ahead of it, such
/// as an opening line stored on its own.
///
/// # Errors
///
/// Any [`PgnErrorKind`] that applies to movetext, positioned within `text`.
#[expect(clippy::todo, reason = "the scanner and parser are stubs")]
pub fn parse_movetext(text: &str) -> Result<Line, PgnError> {
    let _ = text;
    todo!("scan and parse one movetext section")
}
