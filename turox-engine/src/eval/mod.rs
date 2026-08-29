//! Static position evaluation: material (below), piece-square tables
//! (`pst`), and pawn structure (#23), returned from the side-to-move's
//! perspective via `evaluate`.
//!
//! `eval_white_pov` is the absolute (White-relative) sum of terms;
//! `evaluate` is the side-to-move-relative wrapper negamax search wants.
//! Kept as two functions rather than one: the mirror-symmetry property test
//! in `tests/eval_props.rs` (`eval_white_pov(b) == -eval_white_pov(mirrored(b))`)
//! is only cleanly expressible against the absolute version, and a printed
//! eval breakdown is readable in White-POV and confusing in side-relative.

use crate::board::Board;
use crate::eval::pst::pst_value;
use crate::types::Color;
use crate::Piece;

pub mod pst;

/// A position score in centipawns. Positive favors whoever the score is
/// relative to: White for `eval_white_pov`, the side to move for `evaluate`.
pub type Score = i32;

/// Standard piece values in centipawns, indexed by `Piece as usize`. Lives
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
/// For every `(Color, Piece)` pair, sums `PIECE_VALUES[piece] + pst_value`
/// over each of that color's squares, White's total minus Black's. Iterates
/// `board.pieces` (a `Bitboard`, so `for sq in ...` walks its set squares),
/// not `board.piece_at` over `Square::ALL`: the mailbox walk is reserved for
/// `tests/eval_props.rs`'s independent reference, which this gets checked
/// against and shouldn't share code with.
pub fn eval_white_pov(board: &Board) -> Score {
    let mut score = Score::default();
    for piece in Piece::ALL {
        for sq in board.pieces(Color::White, piece) {
            score += PIECE_VALUES[piece as usize];
            score += pst_value(Color::White, piece, sq);
        }
        for sq in board.pieces(Color::Black, piece) {
            score -= PIECE_VALUES[piece as usize];
            score -= pst_value(Color::Black, piece, sq);
        }
    }
    score
}

/// Side-to-move-relative score: positive means the side to move is ahead.
/// The convention negamax search wants.
pub fn evaluate(board: &Board) -> Score {
    match board.side_to_move() {
        Color::White => eval_white_pov(board),
        Color::Black => -eval_white_pov(board),
    }
}
