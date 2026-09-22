//! Scale factors for material balances the piece-square tables can't judge.
//!
//! Whether the position is winnable at all, not where its pieces stand.
//! Multiplies [`super::eval_white_pov`]'s already-interpolated score rather
//! than folding into the tapered mg/eg sum; see
//! `docs/adr/0004-endgame-scale-factors-multiply-the-interpolated-score.md`
//! for why.

use crate::board::Board;
use crate::{Bitboard, Color, Piece};

use super::Score;

/// A multiplier applied to an already-interpolated [`Score`].
///
/// Fixed-point rather than a float, because evaluation is
/// integer arithmetic throughout and a float here would make the same position
/// score differently on different targets.
///
/// A newtype rather than a bare divisor, which is what this started as. The
/// difference shows once there is more than one of these: a factor chosen by
/// pawn count, or a general unwinnability rule, each otherwise invents its own
/// arithmetic at its own call site. Here every rule produces a `ScaleFactor`
/// and the single application point stays [`Self::apply`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ScaleFactor(u16);

impl ScaleFactor {
    /// The numerator that leaves a score untouched; every factor is a fraction
    /// of this. Private: callers build factors through the constructors rather
    /// than doing the arithmetic themselves, which is the point of the type.
    const UNIT: u16 = 256;

    /// Leaves the score alone: the position's winnability is whatever the
    /// summed terms said it was.
    pub const ONE: Self = Self(Self::UNIT);

    /// Scales to exactly zero: a draw under the rules whatever the pieces are
    /// doing, so no amount of positional advantage means anything.
    pub const DRAW: Self = Self(0);

    /// One `divisor`th of the score.
    ///
    /// The chess-programming literature states these rules as divisions ("scale
    /// by 1/16"), so taking a divisor keeps the call sites reading the way their
    /// source does while the type stays a multiplier.
    ///
    /// A `divisor` of zero would be meaningless, and saturates to [`Self::DRAW`]
    /// rather than dividing by zero, which is the nearest honest answer.
    #[must_use]
    pub const fn from_reciprocal(divisor: u16) -> Self {
        match Self::UNIT.checked_div(divisor) {
            Some(numerator) => Self(numerator),
            None => Self::DRAW,
        }
    }

    /// `numerator` `UNIT`ths of the score, for rules whose curve isn't a
    /// clean reciprocal (a pawn-count ramp, say) and so can't be expressed
    /// through [`Self::from_reciprocal`].
    ///
    /// Clamps to [`Self::ONE`] rather than overflowing the fraction past
    /// unity: a caller's curve running past `UNIT` means "at least
    /// unscaled", not "scaled up", since nothing here ever amplifies a score.
    #[must_use]
    pub const fn from_numerator(numerator: u16) -> Self {
        if numerator >= Self::UNIT {
            Self::ONE
        } else {
            Self(numerator)
        }
    }

    /// Whether this factor leaves a score untouched, which is the common case
    /// and worth being able to ask about without comparing to a constant.
    #[must_use]
    pub const fn is_one(self) -> bool {
        self.0 == Self::UNIT
    }

    /// `score` scaled by this factor.
    ///
    /// Computed in `i32` and converted back: the intermediate product exceeds
    /// [`Score`]'s range for any real evaluation, so doing this in `i16` would
    /// overflow long before the division brought it back down.
    ///
    /// Truncates toward zero, matching the plain integer division this replaced,
    /// so a scaled score never rounds away from the draw.
    #[must_use]
    pub fn apply(self, score: Score) -> Score {
        if self.is_one() {
            return score;
        }
        let scaled = i32::from(score) * i32::from(self.0) / i32::from(Self::UNIT);
        Score::try_from(scaled).unwrap_or(score)
    }
}

/// The soft-draw numerator at a pawn-count difference of 0 or 1: the same
/// 1/16 the flat constant this replaced used unconditionally, kept as the
/// floor of the ramp so a bare or single-pawn edge scales exactly as before.
const SOFT_DRAW_BASE: u16 = 16;

/// How much the soft-draw numerator grows per pawn of advantage beyond the
/// first. Chosen so the ramp reaches [`ScaleFactor::ONE`] (no scaling at
/// all) at a five-pawn difference: CPW's framing is that a one-pawn edge in
/// an opposite-bishop ending is close to always a draw and a four-pawn edge
/// is often winning outright, so the curve should still be pulling the
/// score down at four and be functionally unscaled by five.
const SOFT_DRAW_STEP: u16 = 60;

/// The soft-draw numerator for a `pawn_diff`-pawn advantage in an
/// opposite-coloured-bishops ending, ramping linearly from
/// [`SOFT_DRAW_BASE`] up to `ScaleFactor::UNIT`.
#[must_use]
fn soft_draw_numerator(pawn_diff: u32) -> u16 {
    let extra = u16::try_from(pawn_diff.saturating_sub(1)).unwrap_or(u16::MAX);
    SOFT_DRAW_BASE.saturating_add(SOFT_DRAW_STEP.saturating_mul(extra))
}

/// Scales `score` down when `board`'s material balance is known to be a draw
/// or heuristically unwinnable, regardless of what the summed evaluation
/// terms reported:
///
/// - Insufficient material (KK, KNK, KBK, KNNK, and same-coloured KBBK vs a
///   bare king): scales to exactly 0, since these are draws under the rules
///   regardless of placement.
/// - Opposite-coloured bishops with no other minor or major pieces: scales
///   down by a curve indexed on the pawn-count difference between the two
///   sides, since a pawn advantage there is often not winnable but isn't a
///   guaranteed draw the way insufficient material is, and a larger
///   advantage is more often winnable than a smaller one.
#[must_use]
pub fn scale_factor(board: &Board) -> ScaleFactor {
    let piece_count = |piece: Piece| {
        board
            .pieces(Color::White, piece)
            .or(board.pieces(Color::Black, piece))
            .count()
    };
    if piece_count(Piece::Rook) > 0 || piece_count(Piece::Queen) > 0 {
        return ScaleFactor::ONE;
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
        let white_pawns = board.pieces(Color::White, Piece::Pawn).count();
        let black_pawns = board.pieces(Color::Black, Piece::Pawn).count();
        return soft_draw_factor(
            white_knights,
            black_knights,
            white_bishops,
            black_bishops,
            white_pawns.abs_diff(black_pawns),
        );
    }

    // Every hard-draw signature needs zero pawns too: a single pawn
    // anywhere can promote into whatever material the bare side is
    // missing, so it's never actually insufficient.
    if piece_count(Piece::Pawn) > 0 {
        return ScaleFactor::ONE;
    }

    if white_bare && black_bare {
        return ScaleFactor::DRAW; // KK
    }
    if white_bare {
        // White is bare; Black holds whatever minors are on the board.
        hard_draw_factor(black_knights, black_bishops)
    } else {
        // Black is bare; White holds whatever minors are on the board.
        hard_draw_factor(white_knights, white_bishops)
    }
}

/// Scales to 0 when the non-bare side's minors alone still can't force mate:
/// a single knight, a knight pair (`KNNK`), a single bishop (`KBK`), or two
/// same-coloured bishops (same-coloured `KBBK`). A knight-and-bishop pair or
/// two opposite-coloured bishops can each force mate against a lone king, so
/// `score` passes through unscaled for those.
#[must_use]
fn hard_draw_factor(knights: Bitboard, bishops: Bitboard) -> ScaleFactor {
    let is_drawn = match (knights.count(), bishops.count()) {
        (1 | 2, 0) | (0, 1) => true,
        (0, 2) => same_colored_bishops(bishops),
        _ => false,
    };
    if is_drawn {
        ScaleFactor::DRAW
    } else {
        ScaleFactor::ONE
    }
}

/// Scales down, not to zero (this is a heuristic, not a rules-guaranteed
/// draw), when both sides' only minor is one bishop each, on opposite-
/// coloured squares: the classic fortress case where a pawn advantage often
/// isn't enough to win. `score` passes through unscaled for any other
/// non-bare-vs-non-bare split (a knight facing a bishop, say), since this
/// issue only covers the opposite-coloured-bishops case. `pawn_diff` is the
/// absolute pawn-count difference between the two sides, feeding
/// [`soft_draw_numerator`]'s ramp.
#[must_use]
fn soft_draw_factor(
    white_knights: Bitboard,
    black_knights: Bitboard,
    white_bishops: Bitboard,
    black_bishops: Bitboard,
    pawn_diff: u32,
) -> ScaleFactor {
    let is_ocb = white_knights.is_empty()
        && black_knights.is_empty()
        && white_bishops.count() == 1
        && black_bishops.count() == 1
        && !same_colored_bishops(white_bishops.or(black_bishops));
    if is_ocb {
        ScaleFactor::from_numerator(soft_draw_numerator(pawn_diff))
    } else {
        ScaleFactor::ONE
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
