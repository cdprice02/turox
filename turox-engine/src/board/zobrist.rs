//! Incremental Zobrist hashing of a `Board`.
//!
//! One independent key per hashable fact about a position (piece-square, side to move,
//! each castling right, each en passant file), `XORed` together into a single `u64`. XOR
//! is its own inverse, so the same operation that mixes a fact in also mixes it back out,
//! which is what makes incremental maintenance possible at all: `place`/`remove`
//! (`board::mod`) XOR the piece-square contribution as part of the single path every
//! placement already goes through; `Board::from_parts` XORs in the
//! side-to-move/castling/en-passant contribution once, for any position built directly
//! rather than incrementally (FEN parsing, the standard starting position, test helpers);
//! and `Board::make_move` keeps that same contribution correct move to move by `XORing`
//! the old and new value of each fact together in one combined update, rather than
//! patching the several direct field writes (five for castling rights, two for en
//! passant) that actually change them.
//! `side_to_move_hash`/`castling_hash`/`en_passant_hash` below are the shared tool behind
//! all three call sites.
//!
//! En passant refinement, deliberately deferred: strictly, the en passant key should only
//! be mixed in when an en passant capture is actually available in the resulting
//! position, not just whenever `Board::en_passant()` is `Some`. Two positions that are
//! otherwise identical but differ only in an all-but-unusable ep target currently hash
//! differently; that's a known, accepted gap for the first pass, not a silent oversight.

use super::Board;
use crate::{
    rng::xorshift64star,
    types::{CastlingRights, Color, ColoredPiece, Square},
};

/// One independent key per hashable fact, `XORed` together for a position's
/// hash. Kept private to this module; every other file reaches these
/// through the `*_hash` functions below rather than indexing the table
/// directly, so the mapping from "fact" to "which key" has exactly one
/// place to get right.
struct Keys {
    /// One key per `(ColoredPiece, Square)`, indexed by `ColoredPiece as usize`.
    piece_square: [[u64; 64]; 12],
    /// Mixed in whenever it's Black to move; White-to-move positions don't
    /// touch this at all. That asymmetry is deliberate: it means
    /// `make_move` can XOR this key in unconditionally on every move
    /// (it always toggles) rather than needing an old/new comparison the
    /// way castling and en passant do.
    side_to_move: u64,
    /// One key per individual castling right (not one key per 16-way
    /// combination), in `CASTLING_BITS` order.
    castling: [u64; 4],
    /// One key per file, mixed in whenever `Board::en_passant()` is `Some`
    /// on that file. See the module doc for the coarser-than-strictly-legal
    /// en passant behavior this implies.
    en_passant_file: [u64; 8],
}

/// The four individual castling rights, in the order `Keys::castling`
/// indexes them. Shared between `castling_hash` and anything else that
/// needs to walk "each right, in a fixed order" (matches the discipline
/// `CastlingRights::rook_squares`'s own tests already use: every
/// `{Color}x{side}` combination checked explicitly, not just one).
const CASTLING_BITS: [CastlingRights; 4] = [
    CastlingRights::WHITE_KINGSIDE,
    CastlingRights::WHITE_QUEENSIDE,
    CastlingRights::BLACK_KINGSIDE,
    CastlingRights::BLACK_QUEENSIDE,
];

/// The fixed seed the whole key table is generated from: the 64-bit
/// golden-ratio constant, a standard choice for PRNG/hash mixing and, not
/// incidentally, nonzero (`rng::xorshift64star`'s "zero is a fixed point"
/// gotcha applies here exactly as much as it does to the magic-number
/// search, which is why this isn't `0` or some other "obvious" value).
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Generates the full key table by walking `rng::xorshift64star` forward
/// from `SEED`, one call per key, in a fixed order: all 768 piece-square
/// keys, then side to move, then the 4 castling keys, then the 8
/// en-passant-file keys. `state` is always advanced *before* a value is
/// read off it, so `SEED` itself is never handed out as a key.
///
/// `const fn` rather than a plain `fn` run once at startup: the whole table
/// folds to a compile-time constant this way, the same reason
/// `Board::START_POS` is a `const` rather than 32 `place` calls repeated on
/// every call. That's also *why* this has to be `xorshift64star` and not,
/// say, a runtime-seeded RNG: nothing outside a hand-rolled, deterministic,
/// `const`-callable generator can run at compile time at all.
const fn generate_keys() -> Keys {
    let mut piece_square = [[0u64; 64]; 12];
    let mut state = SEED;
    let mut cp = 0;
    while cp < 12 {
        let mut sq = 0;
        while sq < 64 {
            state = xorshift64star(state);
            piece_square[cp][sq] = state;
            sq += 1;
        }
        cp += 1;
    }

    state = xorshift64star(state);
    let side_to_move = state;

    let mut castling = [0u64; 4];
    let mut i = 0;
    while i < 4 {
        state = xorshift64star(state);
        castling[i] = state;
        i += 1;
    }

    let mut en_passant_file = [0u64; 8];
    let mut f = 0;
    while f < 8 {
        state = xorshift64star(state);
        en_passant_file[f] = state;
        f += 1;
    }

    Keys {
        piece_square,
        side_to_move,
        castling,
        en_passant_file,
    }
}

const KEYS: Keys = generate_keys();

/// The hash contribution of a single `(ColoredPiece, Square)` fact. What
/// `place`/`remove` XOR in and back out as pieces come and go.
pub(crate) const fn piece_square_hash(cp: ColoredPiece, sq: Square) -> u64 {
    KEYS.piece_square[cp as usize][sq.index() as usize]
}

/// The hash contribution of `color` being to move: `0` for White, one
/// fixed key for Black. Since this is the *only* fact here that toggles
/// unconditionally on every move (side to move always flips), a caller
/// wiring this into `make_move` doesn't need this function's old/new form
/// at all: XOR `KEYS.side_to_move` in directly, every move, unconditionally.
pub(crate) const fn side_to_move_hash(color: Color) -> u64 {
    match color {
        Color::White => 0,
        Color::Black => KEYS.side_to_move,
    }
}

/// The combined hash contribution of every right set in `rights`.
///
/// Two uses. `Board::from_parts` folds in `castling_hash(rights)` once, for
/// a position built from scratch. `Board::make_move` uses it differently,
/// for an *incremental* update after rights change:
/// `castling_hash(old_rights) ^ castling_hash(new_rights)` `XORed` into the
/// position hash has the identical effect as `XORing` out exactly the bits
/// that changed, since any bit that *didn't* change gets `XORed` by its own
/// key twice and cancels back to zero. `make_move` calls this once, after
/// all five of its castling-rights writes are done, rather than `XORing`
/// next to each site individually.
pub(crate) const fn castling_hash(rights: CastlingRights) -> u64 {
    let mut hash = 0u64;
    let mut i = 0;
    while i < 4 {
        if rights.contains(CASTLING_BITS[i]) {
            hash ^= KEYS.castling[i];
        }
        i += 1;
    }
    hash
}

/// The hash contribution of `ep`: `0` for `None`, one key per file for
/// `Some`. Same old/new-XOR technique as `castling_hash` applies here for
/// an incremental update, and for the same reason: `make_move` only ever
/// has at most one en passant square live at a time, so
/// `en_passant_hash(old) ^ en_passant_hash(new)` is exactly the delta.
pub(crate) const fn en_passant_hash(ep: Option<Square>) -> u64 {
    match ep {
        Some(sq) => KEYS.en_passant_file[sq.file().index() as usize],
        None => 0,
    }
}

/// An independent, from-scratch fold over `board`'s full state.
///
/// Every occupied square via `piece_at` (not the bitboards `place`/`remove` maintain),
/// plus side to move, castling rights, and en passant. Used only as
/// `tests/zobrist_props.rs`'s test oracle, checked against the incrementally-maintained
/// `Board::hash()`; production code should always read `Board::hash()` instead;
/// recomputing this on a hot path defeats the entire point of maintaining the hash
/// incrementally.
#[must_use]
pub fn compute_hash(board: &Board) -> u64 {
    let mut hash = 0u64;
    for sq in Square::ALL {
        if let Some(cp) = board.piece_at(sq) {
            hash ^= piece_square_hash(cp, sq);
        }
    }
    hash ^= side_to_move_hash(board.side_to_move());
    hash ^= castling_hash(board.castling_rights());
    hash ^= en_passant_hash(board.en_passant());
    hash
}
