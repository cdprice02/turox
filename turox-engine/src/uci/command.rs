//! Parsing UCI input into a typed [`Command`], free of I/O.
//!
//! `parse` reads nothing and has no side effects, so it's directly testable against plain
//! strings without a stdin/stdout session harness. `super::session` is what actually
//! reads lines and owns any state across them.

use crate::board::Board;
use crate::move_gen::legal::legal_moves;
use crate::types::Move;
use std::iter::Peekable;
use std::time::Duration;

/// One parsed line of UCI input.
///
/// `Position`'s `Board` is fully resolved (`startpos`/`fen` plus every move in the
/// `moves` list already applied), not a raw spec the loop has to finish interpreting: a
/// `position` line is always a complete, self-contained restatement of the game from a
/// known start, never an incremental diff from wherever the engine currently is, so
/// resolving it needs nothing beyond the line itself.
///
/// Deliberately doesn't model the *entire* UCI spec: `debug`, `register`, `ponderhit`, and
/// `go`'s `ponder`/`searchmoves`/`mate` sub-options have no effect on this engine (no debug
/// logging, no pondering, no restricted-move or mate-distance search modes). `parse`
/// returning `None` for a line built from one of these isn't a parse failure to report
/// anywhere: UCI's own spec already treats input the engine doesn't act on as something to
/// ignore, not reject, so this is that same behavior, not a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `uci`: identify ourselves and switch to UCI mode.
    Uci,
    /// `isready`: report whether we're ready for more commands.
    IsReady,
    /// `ucinewgame`: the next `position`/`go` starts a new game, not a
    /// continuation (relevant once a transposition table exists to clear
    /// between games).
    NewGame,
    /// `position [startpos | fen <fen>] [moves <move> ...]`, resolved to
    /// the final `Board`.
    Position(Board),
    /// `go [...]`: start searching under `GoOptions`.
    Go(GoOptions),
    /// `setoption name <name> [value <value>]`: configure an engine option.
    /// `name`/`value` can each be multiple whitespace-separated words per
    /// the UCI spec (an option like `Debug Log File` is a real example),
    /// so both are collected and rejoined with single spaces rather than
    /// kept as separate tokens. An option this engine doesn't recognize
    /// still parses to a real `Command`; the ignore-what-you-don't-support
    /// decision belongs to whatever handles this command, not to parsing.
    SetOption {
        /// The option's name, e.g. `Hash`.
        name: String,
        /// The option's new value, e.g. `"64"`. `None` for a value-less
        /// `setoption name <name>` line (some UCI options, like a `button`
        /// type, never take one).
        value: Option<String>,
    },
    /// `stop`: stop searching and report the best move found so far.
    Stop,
    /// `quit`: exit.
    Quit,
}

/// `go`'s own sub-options.
///
/// All independent and all optional per the UCI spec (a bare `go` with none of them set
/// is itself a valid command, meaning "use your own judgement"). Maps directly onto
/// `search::Search`'s existing budget mechanisms: `depth`/`nodes` line up with
/// `Search::search`'s own `max_depth` parameter and `Search::with_max_nodes`, `movetime`
/// with `Search::with_deadline`, and `wtime`/`btime`/`winc`/`binc`/`movestogo` with
/// `search::time::allocate_time`. Turning a `GoOptions` into an actual
/// search call is the UCI loop's job, not this module's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GoOptions {
    /// `depth <x>`: search exactly `x` plies, per iterative deepening.
    pub depth: Option<u32>,
    /// `nodes <x>`: abort once `x` nodes have been searched.
    pub nodes: Option<u64>,
    /// `movetime <x>`: search for exactly `x` milliseconds.
    pub movetime: Option<Duration>,
    /// `infinite`: search until `stop`, no depth/time budget at all.
    pub infinite: bool,
    /// `wtime <x>`: White's remaining time on the clock, in milliseconds.
    pub wtime: Option<Duration>,
    /// `btime <x>`: Black's remaining time on the clock, in milliseconds.
    pub btime: Option<Duration>,
    /// `winc <x>`: White's per-move increment, in milliseconds.
    pub winc: Option<Duration>,
    /// `binc <x>`: Black's per-move increment, in milliseconds.
    pub binc: Option<Duration>,
    /// `movestogo <x>`: moves remaining until the next time control.
    pub movestogo: Option<u32>,
}

/// Parses one line of UCI input into a [`Command`].
///
/// Returns `None` for a line that isn't the start of a command this engine recognizes or
/// acts on, or one that's malformed in a way that makes it unsafe to guess at (an invalid
/// FEN, an unresolvable move in a `moves` list, a `go` value that isn't the number it
/// claims to be): never panics, whatever the input, per UCI's own robustness
/// expectations.
///
/// Splits on whitespace with [`str::split_whitespace`], not a literal
/// `split(' ')`: real UCI input can have repeated spaces or tabs between
/// tokens (`split_whitespace` already trims and collapses all of that,
/// including any leading/trailing whitespace, so nothing extra is needed
/// there), and a literal single-space split would produce empty tokens
/// between them instead of collapsing them. Dispatches on the first token
/// and hands the rest to `parse_position`/`parse_go` for the two commands
/// with their own sub-grammar.
#[must_use]
pub fn parse(line: &str) -> Option<Command> {
    let mut tokens = line.split_whitespace().peekable();
    match tokens.next()? {
        "uci" => Some(Command::Uci),
        "isready" => Some(Command::IsReady),
        "ucinewgame" => Some(Command::NewGame),
        "position" => parse_position(tokens),
        "go" => parse_go(tokens),
        "setoption" => parse_set_option(tokens),
        "stop" => Some(Command::Stop),
        "quit" => Some(Command::Quit),
        _ => None,
    }
}

/// `position [startpos | fen <fen>] [moves <move> ...]`, everything after
/// the `position` token itself.
///
/// The trickiest part is finding where the FEN ends and `moves` begins:
/// `startpos` is a single token, but a FEN is *six* whitespace-separated
/// fields that got split apart by the same `split_whitespace` call that
/// split everything else, so they have to be collected back up before
/// `Board::try_from_fen` (which validates the field count itself, so this
/// doesn't need to hardcode "exactly six") can parse them. [`Peekable::peek`]
/// is what makes that boundary detection possible without guessing: keep
/// collecting fields while the *next* token isn't `moves` and isn't the end
/// of input, without consuming that token, so whichever one stopped the
/// loop (a real `moves` keyword, or simply running out) is still there for
/// the shared check below to see.
///
/// `moves` is optional after either `startpos` or `fen ...`: `position
/// startpos` alone (no moves played yet) is a completely ordinary, common
/// command, not a malformed one, so "no more tokens at all" and "a real
/// `moves` keyword" are the two *valid* outcomes here, and anything else
/// (a token that's present but isn't `moves`) is the only one that's
/// actually malformed.
///
/// If a `moves` list is present, each token is resolved via
/// `Move::from_uci` against `legal_moves` of the position the *previous*
/// move produced (not the original base position), advancing the board one
/// move at a time. An unresolvable move anywhere in the list fails the
/// whole command (`?` propagates `None` out of `parse` entirely) rather
/// than silently keeping whatever prefix did apply: a partially-applied
/// position is a worse failure mode than "we ignored this line," since the
/// engine would go on to search a position the GUI never actually asked
/// for.
fn parse_position<'a>(mut tokens: Peekable<impl Iterator<Item = &'a str>>) -> Option<Command> {
    let mut board = match tokens.next()? {
        "startpos" => Board::start_pos(),
        "fen" => {
            let mut fields = Vec::with_capacity(6);
            while !matches!(tokens.peek(), None | Some(&"moves")) {
                fields.push(tokens.next()?);
            }
            Board::try_from_fen(&fields.join(" ")).ok()?
        }
        _ => return None,
    };

    match tokens.next() {
        None => {}
        Some("moves") => {
            for uci_move in tokens {
                let legal = legal_moves(&board);
                board = board.make_move(Move::from_uci(uci_move, &legal)?);
            }
        }
        Some(_) => return None,
    }

    Some(Command::Position(board))
}

/// `go [...]`, everything after the `go` token itself: an unordered set of
/// independent keyword/value pairs (`infinite` the one bare keyword with no
/// value), so this just loops over whatever tokens remain and fills in
/// whichever field each recognized keyword names.
///
/// A *present* value that isn't the number it claims to be (`go depth
/// abc`) fails the whole command, the same "don't guess" policy
/// `parse_position` uses for an unresolvable move: `tokens.next()?` alone
/// only guards a *missing* value (`go depth` with nothing after it) and
/// would silently leave the option unset if it stopped there, so
/// `.parse().ok()?` chains a second failure point onto the same
/// short-circuit for a value that's present but invalid.
///
/// An unrecognized token (`ponder`, `searchmoves ...`, anything real UCI
/// defines that this engine doesn't act on) is deliberately treated as
/// nothing to do, not a reason to reject the rest of the line: unlike
/// `position`, a `go` command usually carries real budget information
/// (`depth`/`movetime`/the clock fields) that would be a shame to discard
/// over one token this engine doesn't support, and ignoring input you
/// don't act on is exactly what UCI's own spec already asks for.
fn parse_go<'a>(mut tokens: impl Iterator<Item = &'a str>) -> Option<Command> {
    let mut options = GoOptions::default();
    while let Some(keyword) = tokens.next() {
        match keyword {
            "depth" => options.depth = Some(tokens.next()?.parse().ok()?),
            "nodes" => options.nodes = Some(tokens.next()?.parse().ok()?),
            "movetime" => {
                options.movetime = Some(Duration::from_millis(tokens.next()?.parse().ok()?));
            }
            "infinite" => options.infinite = true,
            "wtime" => options.wtime = Some(Duration::from_millis(tokens.next()?.parse().ok()?)),
            "btime" => options.btime = Some(Duration::from_millis(tokens.next()?.parse().ok()?)),
            "winc" => options.winc = Some(Duration::from_millis(tokens.next()?.parse().ok()?)),
            "binc" => options.binc = Some(Duration::from_millis(tokens.next()?.parse().ok()?)),
            "movestogo" => options.movestogo = Some(tokens.next()?.parse().ok()?),
            _ => {}
        }
    }
    Some(Command::Go(options))
}

/// `setoption name <name> [value <value>]`, everything after the
/// `setoption` token itself.
///
/// The `name` keyword is required (an unconditional `?` below): unlike
/// `go`'s sub-options, there's no meaningful `SetOption` without one.
/// Everything up to `value` (or the end of the line, if there's no
/// `value`) is the name; everything after `value` is, well, the value.
/// Multi-word names/values are real (`Debug Log File`, or a `string`-type
/// value containing spaces), so both are collected token by token and
/// rejoined with `join(" ")` rather than assumed to be a single token.
fn parse_set_option<'a>(mut tokens: impl Iterator<Item = &'a str>) -> Option<Command> {
    if tokens.next()? != "name" {
        return None;
    }

    let rest: Vec<&str> = tokens.collect();
    let (name_words, value) = rest.iter().position(|&token| token == "value").map_or_else(
        || (&rest[..], None),
        |i| (&rest[..i], Some(rest[i + 1..].join(" "))),
    );

    Some(Command::SetOption {
        name: name_words.join(" "),
        value,
    })
}
