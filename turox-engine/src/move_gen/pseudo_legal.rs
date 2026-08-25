//! Pseudolegal move generation: every move a piece's basic movement rule
//! allows, for `board.side_to_move()`, without checking whether it leaves the
//! mover's own king in check. That filter is `legal`'s job — see this module's
//! doc note on why the split exists.
//!
//! # Public surface
//!
//! - `fn pseudo_legal_moves(board: &Board, list: &mut MoveList)` — calls all
//!   five functions below, in any order (their outputs don't overlap).
//! - `fn pawn_moves(board: &Board, list: &mut MoveList)`
//! - `fn knight_moves(board: &Board, list: &mut MoveList)`
//! - `fn king_moves(board: &Board, list: &mut MoveList)`
//! - `fn slider_moves(board: &Board, list: &mut MoveList)` — bishop, rook, queen
//! - `fn castling_moves(board: &Board, list: &mut MoveList)`
//!
//! Each is `pub`, not just an internal helper of `pseudo_legal_moves`, so each
//! gets its own reference-oracle proptest and a bug in, say, pawn generation
//! fails in isolation rather than inside a 40-move diff against the whole list.
//!
//! # Why king moves aren't filtered here
//!
//! `king_moves` generates every king step onto a non-friendly square, including
//! ones an enemy piece attacks — that's `legal`'s job, once it exists, via
//! copy-make (`board.make_move(m)` then check the resulting king isn't
//! attacked). Pre-filtering with `attacks::attacked_by` here would be a
//! plausible-looking optimization that's actually wrong on its own: it doesn't
//! know about pins, discovered checks, or (see below) castling-through-check,
//! so it would need `legal`'s machinery anyway. Keep the concerns separate.
//!
//! # Knights, king, sliders
//!
//! Mechanical, and all three should go through `attacks::piece_attacks` rather
//! than reaching into `tables`/`magic` directly, so the `Piece` -> attack-fn
//! dispatch stays in the one place that already owns it. `Bitboard` implements
//! `IntoIterator<Item = Square>`, so `for from in board.pieces(us, piece)`
//! works directly. Per source square:
//!
//! ```text
//! targets  = piece_attacks(piece, us, from, board.occupied()) & !board[us]
//! captures = targets & board[them]   -> MoveFlags::Capture
//! quiets   = targets & board.empty() -> MoveFlags::Quiet
//! ```
//!
//! # Castling
//!
//! The {Color}x{kingside,queenside} four-way mapping `CLAUDE.md` flags as a
//! repeat offender. Worth encoding as *data* — one small table of 4 rows, read
//! side by side — rather than four hand-written branches:
//!
//! | color | side | king from -> to | must be empty | must be unattacked | flag          |
//! |-------|------|-----------------|----------------|---------------------|---------------|
//! | White | K    | e1 -> g1        | f1 g1          | e1 f1 g1            | `KingCastle`  |
//! | White | Q    | e1 -> c1        | b1 c1 d1       | e1 d1 c1            | `QueenCastle` |
//! | Black | K    | e8 -> g8        | f8 g8          | e8 f8 g8            | `KingCastle`  |
//! | Black | Q    | e8 -> c8        | b8 c8 d8       | e8 d8 c8            | `QueenCastle` |
//!
//! The queenside b-file square must be **empty but need not be unattacked** —
//! the king never crosses it, only the rook does. That the empty-set and the
//! must-be-safe set differ for exactly two of the four rows is exactly what a
//! hand-written branch per case tends to get wrong.
//!
//! Compute `enemy = attacks::attacked_by(board, them, board.occupied())`
//! **once** per call and test each row with `(safe_mask & enemy).is_empty()`,
//! rather than three separate `attacks::is_attacked` calls per row. Using the
//! board's actual occupancy (not a king-removed one) is correct here: the only
//! way our own king on e1/e8 could shadow one of these transit squares from an
//! enemy slider is via the a1-e1/a8-e8 ray, and a slider on that ray is already
//! attacking e1/e8 itself, which fails the check regardless.
//!
//! This is why `pseudo_legal` depends on `attacks` (merged in the prior PR):
//! `legal`'s copy-make filter only inspects the *resulting* position, so on its
//! own it can't catch castling *through* check — only landing in it. Rook
//! presence on the corner is implied by the castling right and is already
//! assumed by `Board::make_move`.
//!
//! # Pawns
//!
//! The hairy one. Set-wise, using primitives already on `Bitboard`:
//!
//! - `pushes = pawns.pawn_pushes(us, board.empty())`.
//! - `doubles = pushes.pawn_pushes(us, board.empty()) & double_rank`, where
//!   `double_rank` is `Rank::R4.bitboard()` for White, `Rank::R5.bitboard()`
//!   for Black. Pushing twice *through* `empty` is what makes a blocker on the
//!   intermediate square stop the double push; a single shift-by-16 would skip
//!   right over it — a classic silent bug.
//! - `caps_east = pawns.pawn_attacks_east(us) & board[them]`, same for `_west`.
//! - Recovering `from` from a target square: pick one sign convention once
//!   (`let forward: i8 = match us { White => 1, Black => -1 };`) and derive
//!   every source with `to.offset(-df, -forward * n)` rather than writing out
//!   four separate sign literals across push / double / capture-east /
//!   capture-west — one sign in one place is much harder to get backward for
//!   just one of the four than four sign literals scattered across the
//!   function.
//! - Promotion: split each target set by `Rank::R8.bitboard()` (White) /
//!   `Rank::R1.bitboard()` (Black). The `& promo` half fans one target into
//!   four moves (`PromoteKnight`/`Bishop`/`Rook`/`Queen`, or the
//!   `PromoteCapture*` variants if it came from a capture set); the `& !promo`
//!   half makes one `Quiet`/`Capture` move. Crossing the two halves — giving a
//!   promotion move a non-promoting capture flag, or vice versa — is the
//!   second classic bug here.
//! - En passant: `board.en_passant()` gives the target square, if any. Sources
//!   are `tables::pawn_attacks(them, ep_sq) & board.pieces(us, Pawn)` — the
//!   same reverse-the-color reasoning `attacks::attackers_of` uses (stand the
//!   *opposing* color's pawn on the target square to find which of *our* pawns
//!   could have captured onto it), and the second and last place in the crate
//!   that trick is needed. Flag is `MoveFlags::EnPassant`, **not** `Capture` —
//!   `MoveFlags::is_capture()` already covers both, so nothing downstream
//!   should need to tell them apart by checking `en_passant()` again.

use crate::board::Board;
use crate::move_gen::move_list::MoveList;

/// Generates every pseudolegal move for `board.side_to_move()` into `list`.
/// Calls the five functions below; their outputs never overlap (each covers a
/// disjoint set of piece types / move shapes), so order between them doesn't
/// matter.
#[allow(unused_variables)]
pub fn pseudo_legal_moves(board: &Board, list: &mut MoveList) {
    todo!()
}

/// Pushes, double pushes, captures (including en passant), and all four
/// promotion variants (quiet and capturing) for every pawn of
/// `board.side_to_move()`.
#[allow(unused_variables)]
pub fn pawn_moves(board: &Board, list: &mut MoveList) {
    todo!()
}

/// Every quiet move and capture for every knight of `board.side_to_move()`.
#[allow(unused_variables)]
pub fn knight_moves(board: &Board, list: &mut MoveList) {
    todo!()
}

/// Every quiet move and capture for `board.side_to_move()`'s king — deliberately
/// including moves onto attacked squares; see the module doc for why that
/// filter belongs in `legal`, not here.
#[allow(unused_variables)]
pub fn king_moves(board: &Board, list: &mut MoveList) {
    todo!()
}

/// Every quiet move and capture for every bishop, rook, and queen of
/// `board.side_to_move()`.
#[allow(unused_variables)]
pub fn slider_moves(board: &Board, list: &mut MoveList) {
    todo!()
}

/// Kingside and queenside castling for `board.side_to_move()`, where the
/// relevant `CastlingRights` bit is set, the squares between king and rook are
/// empty, and the king's start/transit/landing squares are all unattacked. See
/// the module doc for why this needs `attacks::attacked_by` rather than being
/// deferred to `legal`'s filter.
#[allow(unused_variables)]
pub fn castling_moves(board: &Board, list: &mut MoveList) {
    todo!()
}
