//! Static position evaluation, returned from the side-to-move's perspective
//! via `evaluate`.
//!
//! Combines material (below) with the terms in this module's submodules,
//! blended between midgame and endgame by `phase`. The submodule list is the
//! term list; repeating it here is how it goes stale.
//!
//! `eval_white_pov` is the absolute (White-relative) sum of terms;
//! `evaluate` is the side-to-move-relative wrapper negamax search wants.
//! Kept as two functions rather than one: the mirror-symmetry property test
//! in `tests/eval_props.rs` (`eval_white_pov(b) == -eval_white_pov(mirrored(b))`)
//! is only cleanly expressible against the absolute version, and a printed
//! eval breakdown is readable in White-POV and confusing in side-relative.

use crate::eval::pst::{pst_value, pst_value_eg};
use turox_chess::board::Board;
use turox_chess::types::Color;
use turox_chess::Piece;

mod bishop_pair;
pub mod endgame_scale;
mod king_safety;
mod outposts;
mod pawn_structure;
mod phase;
pub mod pst;
mod rook_files;
mod tempo;
pub mod weights;

/// A position score in centipawns. Positive favors whoever the score is
/// relative to: White for `eval_white_pov`, the side to move for `evaluate`.
///
/// `i16`, not a wider type: material plus piece-square terms never approach
/// even a fraction of `i16::MAX` (a full board's worth of extra queens from
/// promotion is still in the low five figures), and [`crate::search::MATE`]'s
/// own magnitude leaves headroom under it too. Keeping this narrow is what
/// lets `search::tt::Entry` store a score directly, with nothing to narrow
/// or widen at the boundary.
pub type Score = i16;

/// Standard piece values in centipawns, indexed by `Piece::index`. Lives
/// here rather than as a `Piece::value()` method: what a knight is worth is
/// an evaluation policy that will change as the engine gets tuned, not an
/// intrinsic property of the type, and `types` shouldn't depend on `eval`'s
/// opinions.
///
/// Kings score 0: every position `legal_moves` can reach has exactly one per
/// side, so a king value would cancel identically and only invite overflow.
///
/// `pub(crate)` rather than private: `search`'s MVV-LVA move ordering reuses
/// this same value scale for ranking captures, rather than maintaining a
/// second table that could drift out of sync with this one.
pub(crate) use weights::PIECE_VALUES;

/// Every term that is a function of piece placement, in summation order.
///
/// A table rather than a hand-written `+= White` / `-= Black` pair each,
/// because the pairs are what go stale: this module's own doc already says
/// the submodule list is the term list, and four of the five most recent
/// eval merges each added one pair by hand. A term added to this array is
/// summed for both colours or not at all.
///
/// `tempo` is deliberately absent: it is side-to-move-relative rather than
/// per-colour, so it has no Black half to subtract and cannot be expressed
/// in this shape. See `tempo::tempo_score`.
const TERMS: &[fn(&Board, Color) -> phase::Tapered] = &[
    pawn_structure::pawn_structure_score,
    king_safety::king_safety_score,
    bishop_pair::bishop_pair_score,
    rook_files::rook_files_score,
    outposts::outpost_score,
    outposts::king_pawn_tropism_score,
];

/// Material, piece-square, and every other term's sum from White's
/// perspective: positive means White is ahead.
///
/// Depends on `board.side_to_move()` only through `tempo`, the one term
/// that isn't a function of piece placement; every other term here is
/// placement-only.
///
/// Accumulates a midgame and an endgame term together (packed into one
/// `phase::Tapered` running total) and blends them into a single `Score`
/// only once, at the end, rather than computing two full passes over the
/// board and interpolating term-by-term: a piece's midgame and endgame
/// contributions are already known the moment its square is visited, so
/// there's no reason to walk the board twice to get them both.
///
/// Iterates `board.pieces` (a `Bitboard`, so `for sq in ...` walks its set
/// squares), not `board.piece_at` over `Square::ALL`: the mailbox walk is
/// reserved for `tests/eval_props.rs`'s independent reference, which this
/// gets checked against and shouldn't share code with.
#[must_use]
pub fn eval_white_pov(board: &Board) -> Score {
    let mut score: phase::Tapered = 0;
    for piece in Piece::ALL {
        for sq in board.pieces(Color::White, piece) {
            score += phase::pack(
                PIECE_VALUES[piece.index()] + pst_value(Color::White, piece, sq),
                PIECE_VALUES[piece.index()] + pst_value_eg(Color::White, piece, sq),
            );
        }
        for sq in board.pieces(Color::Black, piece) {
            score -= phase::pack(
                PIECE_VALUES[piece.index()] + pst_value(Color::Black, piece, sq),
                PIECE_VALUES[piece.index()] + pst_value_eg(Color::Black, piece, sq),
            );
        }
    }
    for term in TERMS {
        score += term(board, Color::White);
        score -= term(board, Color::Black);
    }
    // Not a per-colour pair like every term above: tempo depends on
    // `board.side_to_move()` directly, see `tempo::tempo_score`'s own doc.
    score += tempo::tempo_score(board);
    let score = phase::interpolate(score, phase::game_phase(board));
    endgame_scale::scale_factor(board).apply(score)
}

/// Side-to-move-relative score: positive means the side to move is ahead.
/// The convention negamax search wants.
#[must_use]
pub fn evaluate(board: &Board) -> Score {
    match board.side_to_move() {
        Color::White => eval_white_pov(board),
        Color::Black => -eval_white_pov(board),
    }
}
