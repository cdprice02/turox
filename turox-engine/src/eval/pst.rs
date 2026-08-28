//! Piece-square tables: per-square, per-piece positional bonuses summed
//! alongside material in `eval::eval_white_pov`.
//!
//! Values are Tomasz Michniewski's "Simplified Evaluation Function"
//! (public, widely republished, e.g. on chessprogramming.org), midgame
//! variants only. Single-phase, not tapered: one table per piece, no
//! midgame/endgame interpolation. Known limitation accepted here: with a
//! midgame king table and no phase blending, the engine will keep its king
//! cornered in endgames where it should centralize instead. Tapered eval is
//! tracked separately in the Backlog, once self-play can measure whether
//! it's worth the added complexity.

use super::Score;
use crate::types::{Color, Piece, Square};

/// Raw piece-square values, authored in **visual board order**: flat index
/// `0` is a8, index `7` is h8, index `56` is a1, index `63` is h1, matching
/// how these tables are conventionally published and read (top rank first,
/// a-file to h-file). This is the *reverse* of `Square`'s own LERF indexing,
/// where `Square::A1.index() == 0` and `Square::A8.index() == 56` — see
/// `pst_value`'s doc for the reindexing this implies and the gotcha it
/// creates. Indexed by `Piece as usize`, same convention `eval::PIECE_VALUES`
/// uses.
#[rustfmt::skip]
const VISUAL_PST: [[Score; 64]; 6] = [
    // Pawn
    [
         0,   0,   0,   0,   0,   0,   0,   0,
        50,  50,  50,  50,  50,  50,  50,  50,
        10,  10,  20,  30,  30,  20,  10,  10,
         5,   5,  10,  25,  25,  10,   5,   5,
         0,   0,   0,  20,  20,   0,   0,   0,
         5,  -5, -10,   0,   0, -10,  -5,   5,
         5,  10,  10, -20, -20,  10,  10,   5,
         0,   0,   0,   0,   0,   0,   0,   0,
    ],
    // Knight
    [
        -50, -40, -30, -30, -30, -30, -40, -50,
        -40, -20,   0,   0,   0,   0, -20, -40,
        -30,   0,  10,  15,  15,  10,   0, -30,
        -30,   5,  15,  20,  20,  15,   5, -30,
        -30,   0,  15,  20,  20,  15,   0, -30,
        -30,   5,  10,  15,  15,  10,   5, -30,
        -40, -20,   0,   5,   5,   0, -20, -40,
        -50, -40, -30, -30, -30, -30, -40, -50,
    ],
    // Bishop
    [
        -20, -10, -10, -10, -10, -10, -10, -20,
        -10,   0,   0,   0,   0,   0,   0, -10,
        -10,   0,   5,  10,  10,   5,   0, -10,
        -10,   5,   5,  10,  10,   5,   5, -10,
        -10,   0,  10,  10,  10,  10,   0, -10,
        -10,  10,  10,  10,  10,  10,  10, -10,
        -10,   5,   0,   0,   0,   0,   5, -10,
        -20, -10, -10, -10, -10, -10, -10, -20,
    ],
    // Rook
    [
          0,   0,   0,   0,   0,   0,   0,   0,
          5,  10,  10,  10,  10,  10,  10,   5,
         -5,   0,   0,   0,   0,   0,   0,  -5,
         -5,   0,   0,   0,   0,   0,   0,  -5,
         -5,   0,   0,   0,   0,   0,   0,  -5,
         -5,   0,   0,   0,   0,   0,   0,  -5,
         -5,   0,   0,   0,   0,   0,   0,  -5,
          0,   0,   0,   5,   5,   0,   0,   0,
    ],
    // Queen
    [
        -20, -10, -10,  -5,  -5, -10, -10, -20,
        -10,   0,   0,   0,   0,   0,   0, -10,
        -10,   0,   5,   5,   5,   5,   0, -10,
         -5,   0,   5,   5,   5,   5,   0,  -5,
          0,   0,   5,   5,   5,   5,   0,  -5,
        -10,   5,   5,   5,   5,   5,   0, -10,
        -10,   0,   5,   0,   0,   0,   0, -10,
        -20, -10, -10,  -5,  -5, -10, -10, -20,
    ],
    // King (midgame)
    [
        -30, -40, -40, -50, -50, -40, -40, -30,
        -30, -40, -40, -50, -50, -40, -40, -30,
        -30, -40, -40, -50, -50, -40, -40, -30,
        -30, -40, -40, -50, -50, -40, -40, -30,
        -20, -30, -30, -40, -40, -30, -30, -20,
        -10, -20, -20, -20, -20, -20, -20, -10,
         20,  20,   0,   0,   0,   0,  20,  20,
         20,  30,  10,   0,   0,  10,  30,  20,
    ],
];

/// `piece`'s positional bonus for a piece of `color` sitting on `sq`, from
/// that piece's own side's perspective (positive is good for `color`,
/// regardless of whether `color` is actually to move) — the same
/// White-relative-but-per-side convention `eval::eval_white_pov` sums this
/// into.
///
/// Two things to get right, both silent-failure-shaped rather than
/// panic-shaped if wrong:
///
/// 1. **Reindexing.** `VISUAL_PST` is authored top-rank-first (see its own
///    doc); `Square::index()` is bottom-rank-first (LERF). A White query
///    needs `sq`'s rank *reversed* before indexing into the flat array, not
///    `sq.index()` directly.
/// 2. **The color flip.** For Black, mirror the query square with
///    `Square::flip_rank()` *before* that same reindexed lookup (not
///    `Bitboard::mirror_for`, which flips a whole bitboard, not one square).
///    Same table for both colors: "stay near your own back rank early" is a
///    symmetric idea, just relative to each side's own orientation, so
///    Black's own home squares should read the same as White's home squares
///    do, not their absolute mirror image.
///
/// Getting either of these backwards doesn't panic or fail to compile: it
/// produces an engine that plays measurably worse (develops backwards,
/// centralizes the wrong king) while looking entirely reasonable on a
/// read-through. `tests/eval_props.rs`'s orientation-anchor tests exist
/// specifically to catch this, the same `{Color}x{direction}` shape that's
/// bitten this crate before.
///
/// One legitimate implementation path, not the only one: recompute the
/// reindexed lookup per call first (matching `eval_white_pov`'s own
/// "from-scratch, not incremental" starting point), and leave building a
/// single precomputed LERF-ordered `const` table via a `const fn` (`while`
/// loop over indices, same shape `Board::build_start_pos` already uses) as
/// a later, benchmark-driven step once this is correct and tested.
pub fn pst_value(color: Color, piece: Piece, sq: Square) -> Score {
    let sq = match color {
        Color::White => sq.flip_rank(), // LERF to BERLEF
        Color::Black => sq,
    };
    VISUAL_PST[piece as usize][sq.index() as usize]
}
