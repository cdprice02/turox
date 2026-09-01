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
/// where `Square::A1.index() == 0` and `Square::A8.index() == 56`: see
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

/// `piece`'s positional bonus for a piece of `color` sitting on `sq`.
///
/// From that piece's own side's perspective (positive is good for `color`,
/// regardless of whether `color` is actually to move): the same
/// White-relative-but-per-side convention `eval::eval_white_pov` sums this
/// into.
///
/// Looks backwards at a glance: `sq.flip_rank()` is applied for **White**,
/// not Black, and Black's query goes straight to
/// `VISUAL_PST[piece][sq.index()]` with no flip at all. That's correct, not
/// a swapped-color bug, once the two things happening at once are told
/// apart:
///
/// 1. **Reindexing.** `VISUAL_PST` is authored top-rank-first (see its own
///    doc); `Square::index()` is bottom-rank-first (LERF). Reading a White
///    query by `sq.index()` directly would land on the wrong rank entirely.
///    "Reverse the rank, then read the LERF index" is exactly what
///    `sq.flip_rank().index()` computes, so a single `flip_rank()` does the
///    whole reindex.
/// 2. **The color flip.** Black's own home squares should read the same
///    table entries White's own home squares do (a table entry means "good
///    for a piece near its own side," not "good on this absolute square"),
///    which is a *second* `flip_rank()` on top of the first: one flip for
///    the reindex, one more for Black's perspective. Two `flip_rank()`
///    calls cancel out (`flip_rank` is its own inverse), so Black's net
///    transform is no flip at all, and White's net transform is the single
///    flip this function actually applies.
///
/// Getting this backwards (flipping for Black instead of White, or not
/// flipping at all) doesn't panic or fail to compile: it produces an
/// engine that plays measurably worse (develops backwards, centralizes the
/// wrong king) while looking entirely reasonable on a read-through.
/// `tests/eval_props.rs`'s orientation-anchor tests exist specifically to
/// catch this, the same `{Color}x{direction}` shape that's bitten this
/// crate before.
#[must_use]
pub const fn pst_value(color: Color, piece: Piece, sq: Square) -> Score {
    let sq = match color {
        // The composed reindex-then-Black-flip cancels to nothing for
        // Black (see this function's own doc); White is left holding the
        // single flip that does the reindex alone.
        Color::White => sq.flip_rank(),
        Color::Black => sq,
    };
    VISUAL_PST[piece as usize][sq.index() as usize]
}
