//! The tuned magnitudes every evaluation term is built from, in one place.
//!
//! Public because they are the engine's tuning surface: self-play is what
//! decides these numbers, and a tuner, a report, or a test has to be able to
//! read the same value the search uses. The alternative is a second copy
//! somewhere, which is only ever as correct as the last person to update both.
//!
//! Midgame and endgame halves are kept separate here and packed into the
//! tapered representation at each use site, so the packing stays next to the
//! term that needs it and this module holds numbers rather than layout.
//!
//! Every value is a first-pass placeholder sized by reasoning rather than by
//! measured games, and is expected to move.

use super::Score;

/// Centipawn value of each piece, indexed by `Piece::index`.
///
/// Kings are zero: both sides always have exactly one, so any nonzero value
/// cancels, and a real value would make an illegal king-count position score
/// wildly rather than merely oddly.
pub const PIECE_VALUES: [Score; 6] = [100, 320, 330, 500, 900, 0];

/// How much each piece contributes to "how much material is still on the
/// board", indexed by `Piece::index`.
///
/// Pawns and kings count zero because their count does not track how much
/// fighting material is left.
pub const PHASE_WEIGHT: [u32; 6] = [0, 1, 1, 2, 4, 0];

/// [`PHASE_WEIGHT`] summed over a full starting position, both colours.
pub const TOTAL_PHASE: u32 = 24;

/// `(midgame, endgame)` penalty per pawn beyond the first on a file.
///
/// Worse in the endgame, where a doubled pawn's inability to be defended by a
/// neighbour matters more and there are fewer pieces to compensate.
pub const DOUBLED_PENALTY: (Score, Score) = (-10, -20);

/// `(midgame, endgame)` penalty per pawn with no friendly pawn on an adjacent
/// file, which can therefore never be defended by another pawn.
pub const ISOLATED_PENALTY: (Score, Score) = (-10, -10);

/// `(midgame, endgame)` bonus per pawn with a clear path to promotion. Worth
/// more in the endgame, with fewer pieces left to stop it and a king free to
/// escort it.
pub const PASSED_BONUS: (Score, Score) = (10, 20);

/// `(midgame, endgame)` penalty per king-zone file with no friendly pawn on it.
///
/// Zero in the endgame throughout king safety: with the queens and rooks
/// traded, a bare king is a fighting piece rather than a target, so the whole
/// term should stop applying rather than shrink.
pub const SHELTER_PENALTY: (Score, Score) = (-15, 0);

/// `(midgame, endgame)` additional penalty per fully open king-zone file.
///
/// A file with no pawn of *either* colour. Stacks on [`SHELTER_PENALTY`]
/// rather than replacing it: a fully open file is strictly more dangerous
/// than a semi-open one, not a different category of danger.
pub const OPEN_FILE_PENALTY: (Score, Score) = (-25, 0);

/// `(midgame, endgame)` penalty per enemy pawn advancing inside the king's
/// storm zone, threatening to crack the shelter open.
pub const STORM_PENALTY: (Score, Score) = (-10, 0);

/// How many ranks ahead of the king the storm zone reaches. A pawn still
/// further back than this has not threatened anything yet, and every game
/// starts with pawns there.
pub const STORM_RANGE: u8 = 3;

/// `(midgame, endgame)` bonus for holding both bishops: Kaufman's widely
/// cited figure of roughly half a pawn, flat across both phases.
///
/// The pawn-count-sensitive refinement (bishops gain more as the position
/// opens) is deliberately deferred, so this stays a single number rather
/// than a phase-skewed pair.
pub const BISHOP_PAIR_BONUS: (Score, Score) = (50, 50);

/// `(midgame, endgame)` bonus per rook on a file with no pawn of either
/// colour: the middle of the standard 8-20cp range, flat across both
/// phases for the same reason `BISHOP_PAIR_BONUS` is.
pub const ROOK_OPEN_FILE_BONUS: (Score, Score) = (15, 15);

/// `(midgame, endgame)` bonus per rook on a file with no friendly pawn but
/// at least one enemy pawn: smaller than [`ROOK_OPEN_FILE_BONUS`].
///
/// The middle of the standard 4-10cp range, since an enemy pawn can still
/// block or be defended along the file.
pub const ROOK_SEMI_OPEN_FILE_BONUS: (Score, Score) = (7, 7);
