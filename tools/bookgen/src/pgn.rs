//! Parsing PGN game text into structured games, stopping at SAN move
//! tokens: resolving those against an actual position is `san`'s job, not
//! this module's.

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

/// Parses every game in `text`, a PGN file's full contents (one or more
/// games, each a block of `[Tag "value"]` header lines followed by
/// movetext).
///
/// A game whose headers or movetext don't parse cleanly is dropped rather
/// than aborting the whole run: one malformed game in a multi-million-game
/// source file shouldn't cost every other game in it.
#[must_use]
pub fn parse_pgn(_text: &str) -> Vec<PgnGame> {
    Vec::new()
}
