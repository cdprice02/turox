//! Static position evaluation: material (below) and piece-square tables
//! (`pst`), returned from the side-to-move's perspective via `evaluate`.
//!
//! `eval_white_pov` is the absolute (White-relative) sum of terms;
//! `evaluate` is the side-to-move-relative wrapper negamax search wants.
//! Kept as two functions rather than one: the mirror-symmetry property test
//! in `tests/eval_props.rs` (`eval_white_pov(b) == -eval_white_pov(mirrored(b))`)
//! is only cleanly expressible against the absolute version, and a printed
//! eval breakdown is readable in White-POV and confusing in side-relative.

use crate::board::Board;
use crate::eval::pst::{pst_value, pst_value_eg};
use crate::types::Color;
use crate::Piece;

mod phase;
pub mod pst;

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
pub(crate) const PIECE_VALUES: [Score; 6] = [100, 320, 330, 500, 900, 0];

/// Material plus piece-square sum from White's perspective: positive means
/// White is ahead, regardless of who's actually to move.
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
    phase::interpolate(score, phase::game_phase(board))
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
