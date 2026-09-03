//! Game-phase detection and the packed midgame/endgame score pair tapered
//! eval blends between.
//!
//! `game_phase` turns a position's remaining non-pawn material into a
//! `0..=256` slider; [`Tapered`] carries a midgame and an endgame score
//! together through accumulation so `eval::eval_white_pov` only has to
//! interpolate once, at the very end, rather than keeping two separate
//! running totals in sync by hand.

use crate::board::Board;
use crate::types::{Color, Piece};

use super::Score;

/// Per-piece phase weight (indexed by [`Piece::index`]): how much removing
/// one of that piece nudges the game toward the endgame end of the scale.
/// The standard tapered-eval weighting originating with the Fruit engine
/// and reproduced across most open-source engines since: a knight or
/// bishop counts for one point, a rook two, a queen four. Pawns and kings
/// are zero because their count doesn't track how much fighting material
/// is left.
const PHASE_WEIGHT: [u32; 6] = [0, 1, 1, 2, 4, 0];

/// The phase total at the start of a game: two knights, two bishops, two
/// rooks, and a queen, per side, weighted by [`PHASE_WEIGHT`] and summed
/// over both colors.
const TOTAL_PHASE: u32 = 24;

/// How far into the endgame `board` is, scaled to `0..=256`: 0 is full
/// non-pawn material (pure midgame), 256 is none left (pure endgame).
///
/// Walks every piece but `Pawn` and `King` (neither one's count says
/// anything about how much fighting material remains) and subtracts its
/// weighted count from [`TOTAL_PHASE`], so a fresh board scores 0 and a
/// bare-kings board scores `TOTAL_PHASE`. Each subtraction saturates at 0
/// rather than wrapping: a real game never exceeds `TOTAL_PHASE`, but an
/// arbitrary or proptest-generated board can carry more non-pawn material
/// than that (several extra queens, say). `phase` is unsigned, so letting
/// that subtraction run unguarded would underflow rather than go negative,
/// which panics in a debug build and wraps to a huge value in release;
/// `saturating_sub` keeps it pinned at 0 (pure midgame) for those boards
/// instead.
pub fn game_phase(board: &Board) -> u32 {
    let mut phase = TOTAL_PHASE;
    for piece in Piece::ALL {
        let count =
            board.pieces(Color::White, piece).count() + board.pieces(Color::Black, piece).count();
        phase = phase.saturating_sub(count * PHASE_WEIGHT[piece.index()]);
    }
    (phase * 256 + TOTAL_PHASE / 2) / TOTAL_PHASE
}

/// Packed `(mg, eg)` score pair: `mg` in the high 16 bits, `eg` in the low
/// 16 bits of one `i32`, so accumulating several terms is a single `i32`
/// add instead of two separate `Score` adds.
pub type Tapered = i32;
const _: () = assert!(
    Tapered::BITS == Score::BITS * 2,
    "Tapered must hold exactly two Score-sized lanes"
);

/// Packs `mg` and `eg` into one [`Tapered`] value.
#[must_use]
#[allow(
    clippy::as_conversions,
    reason = "`From`/`TryFrom` aren't const-stable yet, so a const fn widening an i16 into an i32 has to reach for `as`; both casts are sign-extending widens of a value already known to fit, not a lossy narrowing"
)]
pub const fn pack(mg: Score, eg: Score) -> Tapered {
    // Built with `+`, not `|`: `(mg << BITS) + eg` is exactly `mg * 2^BITS
    // + eg` as a true integer, so `pack(a) + pack(b) == pack(a.mg + b.mg,
    // a.eg + b.eg)` by plain distributivity, for any accumulation of
    // terms, not just one. Masking `eg` down to an unsigned low-16 residue
    // (as an OR-based version would) breaks exactly that: a negative `eg`
    // would fold in a hidden extra `2^BITS`, and summing several terms
    // leaves a leftover count of how many individual `eg`s were negative
    // stuck in the `mg` lane, not just the sign of their final sum.
    ((mg as Tapered) << Score::BITS) + (eg as Tapered)
}

/// Blends a [`Tapered`] accumulator down to a single [`Score`] at the given
/// `phase` (0..=256, from [`game_phase`]: 0 is pure midgame, 256 is pure
/// endgame).
#[must_use]
pub fn interpolate(t: Tapered, phase: u32) -> Score {
    // `pack` adds `eg` in rather than OR-ing a masked copy, so a negative
    // combined `eg` "borrows" from the `mg` lane the same way subtracting
    // a small number from a round multiple of `2^BITS` borrows from the
    // next digit up: a plain `t >> BITS` comes out exactly 1 low whenever
    // the combined `eg` is negative. Adding half of `2^BITS` before the
    // shift cancels that borrow unconditionally: `eg + 2^(BITS-1)` always
    // lands in `0..2^BITS` for `eg` in `Score`'s range, so the shift can
    // never be pushed into the next digit by it.
    let mg = Score::try_from((t + (1 << (Score::BITS - 1))) >> Score::BITS).unwrap_or(Score::MAX);
    // The low lane's raw bits, taken as unsigned, are never negative and
    // always fit `u16` by construction (masked to exactly `Score::BITS`
    // bits) — `cast_signed` then reinterprets that bit pattern as the
    // properly sign-extended `Score` it was packed from, which a plain
    // `try_from` on the masked value can't do: the mask alone recovers
    // `eg`'s bits as a positive number (e.g. -150 comes back as 65386),
    // which is outside `i16`'s range for any negative `eg`.
    let low_bits = u16::try_from(t & ((1 << Score::BITS) - 1))
        .expect("masked to Score::BITS bits, always fits u16");
    let eg = low_bits.cast_signed();
    let phase = i32::try_from(phase).expect("`game_phase` returns 0..=256");
    // `phase` is 0..=256, so `256 - phase` is 256..=0. The sum of the two
    // weights is always 256, so the weighted average is always in the
    // range spanned by `mg` and `eg`, never outside it. No rounding bias
    // added here: at the two extremes this division is already exact
    // (`mg * 256 / 256`, `eg * 256 / 256`), and a `+ 128` bias applied
    // unconditionally would disturb that exactness in a sign-dependent
    // way under Rust's truncating (toward zero) integer division. The
    // sub-centipawn precision a bias would buy elsewhere isn't worth that.
    let weighted_score = (i32::from(mg) * (256 - phase) + i32::from(eg) * phase) / 256;
    Score::try_from(weighted_score).unwrap_or(Score::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// `interpolate` at the two ends of its `phase` domain: 0 discards the
    /// endgame term entirely, 256 discards the midgame term entirely. This is
    /// what makes tapering a strict generalization of the pre-tapering,
    /// single-table eval rather than a different thing that happens to agree
    /// with it at the edges by coincidence.
    #[test]
    fn interpolate_reduces_to_pure_midgame_or_pure_endgame_at_the_extremes() {
        let t = pack(300, -150);
        assert_eq!(interpolate(t, 0), 300);
        assert_eq!(interpolate(t, 256), -150);
    }

    proptest! {
        /// A blend can't land outside the range its two inputs span, at any
        /// phase: `interpolate` is a weighted average, not an extrapolation.
        /// Catches a sign error or a swapped mg/eg lane immediately, since
        /// either one sends the result outside this range for almost any
        /// input.
        #[test]
        fn interpolate_stays_within_the_span_of_mg_and_eg(
            mg in -10_000i16..10_000,
            eg in -10_000i16..10_000,
            phase in 0u32..=256,
        ) {
            let blended = interpolate(pack(mg, eg), phase);
            let (lo, hi) = if mg < eg { (mg, eg) } else { (eg, mg) };
            prop_assert!(blended >= lo && blended <= hi);
        }

        /// The entire reason to pack `(mg, eg)` into one `i32` instead of
        /// tracking two running `Score` totals by hand: accumulating packed
        /// terms with plain `i32` addition has to agree with summing the two
        /// halves first and packing the result directly. If the two lanes
        /// ever carried into each other, this is what would catch it. The
        /// bound on the four inputs here (matching `Score`'s own doc: real
        /// eval terms stay well under a fraction of `i16::MAX`) keeps a
        /// two-term sum inside `Score`'s range on each side, since surviving
        /// an actual overflow is a documented non-goal, not a case this
        /// needs to handle.
        #[test]
        fn packed_addition_matches_summing_then_packing(
            a_mg in -10_000i16..10_000, a_eg in -10_000i16..10_000,
            b_mg in -10_000i16..10_000, b_eg in -10_000i16..10_000,
            phase in 0u32..=256,
        ) {
            let accumulated = pack(a_mg, a_eg) + pack(b_mg, b_eg);
            let combined = pack(a_mg + b_mg, a_eg + b_eg);
            prop_assert_eq!(interpolate(accumulated, phase), interpolate(combined, phase));
        }
    }
}
