//! Tokens in, games out: one function per grammar rule, each deciding what
//! comes next by peeking a single token.
//!
//! No rule consumes a token that ends it unless that token belongs to it. A
//! `line` stops before a termination marker, a `)`, a `[` or the end of
//! input and leaves the token for its caller, because only the caller knows
//! which of those it was expecting: a game wants a marker, a variation wants
//! a `)`, and neither ever owns the next game's `[`.

use std::io::BufRead;

use super::scanner::{Pos, Scanner, Token};
use super::{Line, MoveNode, PgnError, PgnErrorKind, PgnGame, TagPair, Tags, MAX_VARIATION_DEPTH};

/// One game: an optional tag section, then a line, then an optional
/// termination marker.
///
/// `Ok(None)` when the input has no tokens left, which is the only way
/// iteration ends. A line stopped by a `)` here has no variation to close,
/// so that is `UnmatchedCloseParen`; stopped by a `[` or the end of input,
/// the game ends with `termination: None`.
pub(super) fn game<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Option<PgnGame>, PgnError> {
    if scanner.peek()?.is_none() {
        return Ok(None);
    }
    let tags = tags(scanner)?;
    let movetext = line(scanner, 0)?;
    let termination = match scanner.peek()? {
        Some((Token::Termination(result), _)) => {
            let result = *result;
            scanner.next()?;
            Some(result)
        }
        Some((Token::RParen, pos)) => return Err(pos.error(PgnErrorKind::UnmatchedCloseParen)),
        _ => None,
    };
    Ok(Some(PgnGame {
        tags,
        movetext,
        termination,
    }))
}

/// A bare movetext section: a line and an optional termination marker, then
/// the end of input. Anything after the marker is an error rather than a
/// second game, because the caller asked for one line.
pub(super) fn movetext<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Line, PgnError> {
    let movetext = line(scanner, 0)?;
    if matches!(scanner.peek()?, Some((Token::Termination(_), _))) {
        scanner.next()?;
    }
    match scanner.next()? {
        None => Ok(movetext),
        Some((Token::RParen, pos)) => Err(pos.error(PgnErrorKind::UnmatchedCloseParen)),
        Some((_, pos)) => Err(pos.error(PgnErrorKind::UnexpectedToken)),
    }
}

/// Tag pairs for as long as the next token is a `[` on the line after the
/// previous pair, or the first one. Stops before anything else without
/// consuming it, so an empty section is not an error.
///
/// A blank line ends the section: the scanner discards whitespace, so line
/// numbers are the only trace of it, and without this rule two header blocks
/// with no movetext between them would merge into one game. Escape lines
/// leave the same gap in the numbers, so the scanner's count of them is
/// subtracted first.
fn tags<R: BufRead>(scanner: &mut Scanner<R>) -> Result<Tags, PgnError> {
    let mut pairs = Vec::new();
    let mut last_close: Option<(usize, usize)> = None;
    while let Some((Token::LBracket, pos)) = scanner.peek()? {
        let line = pos.line;
        if let Some((close_line, escapes)) = last_close {
            let skipped = scanner.escape_lines() - escapes;
            if line - close_line - 1 > skipped {
                break;
            }
        }
        let Some((_, open)) = scanner.next()? else {
            break;
        };
        let (pair, close) = tag_pair(scanner, open)?;
        pairs.push(pair);
        last_close = Some((close.line, scanner.escape_lines()));
    }
    Ok(Tags(pairs))
}

/// A symbol, a string and a `]`, in that order, after a `[` already consumed
/// at `open`, returning the pair and where its `]` was. Any other token anywhere in the sequence, or the end of input,
/// is `MalformedTagPair` at the `[`, because the bracket is where a reader
/// looking at the file would start.
///
/// Each token is checked by peeking before it is consumed, so a wrong one is
/// left for recovery to find: it may be the next game's `[`.
fn tag_pair<R: BufRead>(scanner: &mut Scanner<R>, open: Pos) -> Result<(TagPair, Pos), PgnError> {
    let malformed = || open.error(PgnErrorKind::MalformedTagPair);
    if !matches!(scanner.peek()?, Some((Token::Symbol(_), _))) {
        return Err(malformed());
    }
    let Some((Token::Symbol(name), _)) = scanner.next()? else {
        return Err(malformed());
    };
    if !matches!(scanner.peek()?, Some((Token::Str(_), _))) {
        return Err(malformed());
    }
    let Some((Token::Str(value), _)) = scanner.next()? else {
        return Err(malformed());
    };
    if !matches!(scanner.peek()?, Some((Token::RBracket, _))) {
        return Err(malformed());
    }
    let Some((_, close)) = scanner.next()? else {
        return Err(malformed());
    };
    Ok((TagPair { name, value }, close))
}

/// Moves with their NAGs, comments and variations, until a termination
/// marker, `)`, `[` or the end of input, none of which it consumes.
///
/// A comment before the line's first move belongs to the line; every later
/// one belongs to the last move pushed. A NAG, glyph or `(` with no move
/// before it in this line is an error, since there is nothing for it to
/// annotate or replace. `depth` is how many variations enclose this line.
fn line<R: BufRead>(scanner: &mut Scanner<R>, depth: usize) -> Result<Line, PgnError> {
    let mut line = Line::default();
    while let Some((token, _)) = scanner.peek()? {
        if matches!(
            token,
            Token::Termination(_) | Token::RParen | Token::LBracket
        ) {
            break;
        }
        let Some((token, pos)) = scanner.next()? else {
            break;
        };
        match token {
            Token::Symbol(san) => line.moves.push(MoveNode {
                san,
                ..MoveNode::default()
            }),
            Token::MoveNumber => {}
            Token::Nag(nag) | Token::Glyph(nag) => line
                .moves
                .last_mut()
                .ok_or_else(|| pos.error(PgnErrorKind::NagWithoutMove))?
                .nags
                .push(nag),
            Token::Comment(text) => match line.moves.last_mut() {
                Some(node) => node.comments.push(text),
                None => line.comments.push(text),
            },
            Token::LParen => {
                let node = line
                    .moves
                    .last_mut()
                    .ok_or_else(|| pos.error(PgnErrorKind::VariationWithoutMove))?;
                node.variations.push(variation(scanner, pos, depth + 1)?);
            }
            Token::RBracket | Token::Str(_) => {
                return Err(pos.error(PgnErrorKind::UnexpectedToken));
            }
            Token::LBracket | Token::RParen | Token::Termination(_) => break,
        }
    }
    Ok(line)
}

/// The line inside a variation, called with its `(` already consumed and
/// `open` holding where it was, `depth` levels down from the mainline.
/// Consumes the `)` that closes it; anything else that stops the inner line
/// is `UnclosedVariation` at `open`, the one position that names which
/// variation never closed.
fn variation<R: BufRead>(
    scanner: &mut Scanner<R>,
    open: Pos,
    depth: usize,
) -> Result<Line, PgnError> {
    if depth > MAX_VARIATION_DEPTH {
        return Err(open.error(PgnErrorKind::VariationTooDeep));
    }
    let inner = line(scanner, depth)?;
    if matches!(scanner.peek()?, Some((Token::RParen, _))) {
        scanner.next()?;
        Ok(inner)
    } else {
        Err(open.error(PgnErrorKind::UnclosedVariation))
    }
}
