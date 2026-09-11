//! Scale factors for material balances the piece-square tables can't judge:
//! whether the position is winnable at all, not where its pieces stand.
//! Multiplies [`super::eval_white_pov`]'s already-interpolated score rather
//! than folding into the tapered mg/eg sum; see
//! `docs/adr/0004-endgame-scale-factors-multiply-the-interpolated-score.md`
//! for why.

use crate::board::Board;
use crate::{Bitboard, Color, Piece};

use super::Score;

const SOFT_DRAW_SCALE_FACTOR: Score = 16;

/// Scales `score` down when `board`'s material balance is known to be a draw
/// or heuristically unwinnable, regardless of what the summed evaluation
/// terms reported:
///
/// - Insufficient material (KK, KNK, KBK, KNNK, and same-coloured KBBK vs a
///   bare king): scales to exactly 0, since these are draws under the rules
///   regardless of placement.
/// - Opposite-coloured bishops with no other minor or major pieces: scales
///   down by a flat constant, since a pawn advantage there is often not
///   winnable but isn't a guaranteed draw the way insufficient material is.
#[must_use]
pub fn scale(score: Score, board: &Board) -> Score {
    let piece_count = |piece: Piece| {
        board
            .pieces(Color::White, piece)
            .or(board.pieces(Color::Black, piece))
            .count()
    };
    if piece_count(Piece::Rook) > 0 || piece_count(Piece::Queen) > 0 {
        return score;
    }

    let white_knights = board.pieces(Color::White, Piece::Knight);
    let black_knights = board.pieces(Color::Black, Piece::Knight);
    let white_bishops = board.pieces(Color::White, Piece::Bishop);
    let black_bishops = board.pieces(Color::Black, Piece::Bishop);

    let white_bare = white_knights.is_empty() && white_bishops.is_empty();
    let black_bare = black_knights.is_empty() && black_bishops.is_empty();

    // Opposite-coloured bishops is the one signature that tolerates pawns:
    // a pawn advantage that's often not enough to win is the entire point
    // of it, unlike every hard-draw signature below, so it has to be
    // checked before the pawn guard rules everything else out.
    if !white_bare && !black_bare {
        return soft_draw_scale(
            score,
            white_knights,
            black_knights,
            white_bishops,
            black_bishops,
        );
    }

    // Every hard-draw signature needs zero pawns too: a single pawn
    // anywhere can promote into whatever material the bare side is
    // missing, so it's never actually insufficient.
    if piece_count(Piece::Pawn) > 0 {
        return score;
    }

    if white_bare && black_bare {
        return 0; // KK
    }
    if white_bare {
        // White is bare; Black holds whatever minors are on the board.
        hard_draw_scale(score, black_knights, black_bishops)
    } else {
        // Black is bare; White holds whatever minors are on the board.
        hard_draw_scale(score, white_knights, white_bishops)
    }
}

/// Scales to 0 when the non-bare side's minors alone still can't force mate:
/// a single knight, a knight pair (`KNNK`), a single bishop (`KBK`), or two
/// same-coloured bishops (same-coloured `KBBK`). A knight-and-bishop pair or
/// two opposite-coloured bishops can each force mate against a lone king, so
/// `score` passes through unscaled for those.
#[must_use]
fn hard_draw_scale(score: Score, knights: Bitboard, bishops: Bitboard) -> Score {
    let is_drawn = match (knights.count(), bishops.count()) {
        (1 | 2, 0) | (0, 1) => true,
        (0, 2) => same_colored_bishops(bishops),
        _ => false,
    };
    if is_drawn {
        0
    } else {
        score
    }
}

/// Scales down, not to zero (this is a heuristic, not a rules-guaranteed
/// draw), when both sides' only minor is one bishop each, on opposite-
/// coloured squares: the classic fortress case where a pawn advantage often
/// isn't enough to win. `score` passes through unscaled for any other
/// non-bare-vs-non-bare split (a knight facing a bishop, say), since this
/// issue only covers the opposite-coloured-bishops case.
#[must_use]
fn soft_draw_scale(
    score: Score,
    white_knights: Bitboard,
    black_knights: Bitboard,
    white_bishops: Bitboard,
    black_bishops: Bitboard,
) -> Score {
    let is_ocb = white_knights.is_empty()
        && black_knights.is_empty()
        && white_bishops.count() == 1
        && black_bishops.count() == 1
        && !same_colored_bishops(white_bishops.or(black_bishops));
    if is_ocb {
        score / SOFT_DRAW_SCALE_FACTOR
    } else {
        score
    }
}

/// Whether the two set squares in `bishops` share a square colour. Used both
/// for one side's own bishop pair (same-coloured `KBBK` can't force mate,
/// unlike an opposite-coloured pair) and, negated, for one bishop per side
/// (opposite-coloured bishops are the fortress case `soft_draw_scale`
/// looks for): a `Bitboard` doesn't care which side each bit came from, so
/// one check serves both callers.
///
/// Any other bit count returns `false`; every caller here only ever passes
/// exactly two set bits.
#[must_use]
fn same_colored_bishops(bishops: Bitboard) -> bool {
    let mut squares = bishops.into_iter();
    match (squares.next(), squares.next()) {
        (Some(a), Some(b)) if squares.next().is_none() => a.is_light() == b.is_light(),
        _ => false,
    }
}
