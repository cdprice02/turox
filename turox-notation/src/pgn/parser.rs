//! Tokens in, games out: one function per grammar rule, each deciding what
//! comes next by peeking a single token.
//!
//! No rule consumes a token that ends it unless that token belongs to it. A
//! `line` stops before a termination marker, a `)`, a `[` or the end of
//! input and leaves the token for its caller, because only the caller knows
//! which of those it was expecting: a game wants a marker, a variation wants
//! a `)`, and neither ever owns the next game's `[`.

#![expect(
    clippy::needless_pass_by_ref_mut,
    reason = "the stubs take the scanner mutably for the bodies that will advance it"
)]
#![expect(
    dead_code,
    reason = "the rules below `game` and `movetext` have no caller until those are written"
)]

use std::io::BufRead;

use super::scanner::{Pos, Scanner};
use super::{Line, PgnError, PgnGame, TagPair, Tags};

/// One game: an optional tag section, then a line, then an optional
/// termination marker.
///
/// `Ok(None)` when the input has no tokens left, which is the only way
/// iteration ends. A line stopped by a `)` here has no variation to close,
/// so that is `UnmatchedCloseParen`; stopped by a `[` or the end of input,
/// the game ends with `termination: None`.
#[expect(clippy::todo, reason = "the parser is a stub")]
pub(super) fn game<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Option<PgnGame>, PgnError> {
    let _ = scanner;
    todo!("peek for end of input, then tags, line, and the token that stopped the line")
}

/// A bare movetext section: a line and an optional termination marker, then
/// the end of input. Anything after the marker is an error rather than a
/// second game, because the caller asked for one line.
#[expect(clippy::todo, reason = "the parser is a stub")]
pub(super) fn movetext<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Line, PgnError> {
    let _ = scanner;
    todo!("line, then an optional termination marker, then end of input")
}

/// Tag pairs for as long as the next token is `[`. Stops before anything
/// else without consuming it, so an empty section is not an error.
#[expect(clippy::todo, reason = "the parser is a stub")]
fn tags<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Tags, PgnError> {
    let _ = scanner;
    todo!("tag_pair while the next token is LBracket")
}

/// `[`, a symbol, a string, `]`, in that order. Any other token anywhere in
/// the sequence, or the end of input, is `MalformedTagPair` at the `[`,
/// because the bracket is where a reader looking at the file would start.
#[expect(clippy::todo, reason = "the parser is a stub")]
fn tag_pair<R: BufRead>(scanner: &mut Scanner<R>) -> Result<TagPair, PgnError> {
    let _ = scanner;
    todo!("expect LBracket, Symbol, Str, RBracket")
}

/// Moves with their NAGs, comments and variations, until a termination
/// marker, `)`, `[` or the end of input, none of which it consumes.
///
/// A comment before the line's first move belongs to the line; every later
/// one belongs to the last move pushed. A NAG, glyph or `(` with no move
/// before it in this line is an error, since there is nothing for it to
/// annotate or replace.
#[expect(clippy::todo, reason = "the parser is a stub")]
fn line<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Line, PgnError> {
    let _ = scanner;
    todo!("loop on the peeked token until one that ends a line")
}

/// The line inside a variation, called with its `(` already consumed and
/// `open` holding where it was. Consumes the `)` that closes it; anything
/// else that stops the inner line is `UnclosedVariation` at `open`, the
/// one position that names which variation never closed.
#[expect(clippy::todo, reason = "the parser is a stub")]
fn variation<R: BufRead>(scanner: &mut Scanner<R>, open: Pos) -> Result<Line, PgnError> {
    let _ = (scanner, open);
    todo!("line, then expect RParen")
}
