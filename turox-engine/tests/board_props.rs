//! Property tests for `Board`'s own `PartialEq`.
//!
//! Every other test file that compares boards leans on this impl, so it is the
//! one place where being wrong is invisible rather than loud: mutation testing
//! found that replacing the whole comparison with `true` survived the entire
//! suite, as did flipping each of its seven `&&` to `||`. Nothing anywhere
//! asserted that two boards were *unequal*, so `fen_props.rs`'s round-trip
//! property was only confirming that the comparison did not crash.
//!
//! The shape below fixes that by construction: take a board, change exactly one
//! field, and require the result to differ. One such pair is enough to catch
//! every `&&` mutant, since `&&` binds tighter than `||` and whichever side of
//! the mutated operator holds the changed field, the other side is entirely
//! unchanged and reports `true`. Each field still gets its own test rather than
//! relying on that argument, because a reader should be able to see the
//! coverage without reconstructing the operator-precedence reasoning.

mod common;

use common::{any_board, any_square};
use proptest::prelude::*;
use turox_engine::board::Board;
use turox_engine::{CastlingRights, ColoredPiece, Piece, Square};

/// Rebuilds `board` with its non-placement state replaced wholesale. The
/// placement half is carried over by copying `board` and letting `from_parts`
/// overwrite the rest, which is the same path `try_from_fen` takes.
const fn with_state(
    board: &Board,
    side_flipped: bool,
    castling: CastlingRights,
    en_passant: Option<Square>,
    halfmove_clock: u8,
    fullmove_number: u16,
) -> Board {
    let side = if side_flipped {
        board.side_to_move().flip()
    } else {
        board.side_to_move()
    };
    Board::from_parts(
        *board,
        side,
        castling,
        en_passant,
        halfmove_clock,
        fullmove_number,
    )
}

/// A board paired with one of its own occupied squares, chosen uniformly.
///
/// Deliberately not `any_board()` plus `prop_assume!(piece_at(sq).is_some())`:
/// the boards here are sparse, so a random square is usually empty and that
/// shape rejects far more often than proptest allows, failing on "too many
/// global rejects" rather than on anything about the code.
fn any_board_and_occupied_square() -> impl Strategy<Value = (Board, Square)> {
    any_board().prop_flat_map(|board| {
        let occupied: Vec<Square> = Square::ALL
            .into_iter()
            .filter(|&sq| board.piece_at(sq).is_some())
            .collect();
        (Just(board), prop::sample::select(occupied))
    })
}

/// `board` with everything left alone: the control for every test below, and
/// the thing that would break first if `from_parts` itself stopped preserving
/// the fields it is handed.
const fn unchanged(board: &Board) -> Board {
    with_state(
        board,
        false,
        board.castling_rights(),
        board.en_passant(),
        board.halfmove_clock(),
        board.fullmove_number(),
    )
}

proptest! {
    /// The control. Rebuilding a board without changing anything must produce
    /// an equal board, or every inequality test below would pass for the wrong
    /// reason: a rebuild that silently perturbed some field would make them all
    /// trivially true.
    #[test]
    fn rebuilding_a_board_unchanged_leaves_it_equal(board in any_board()) {
        prop_assert_eq!(&board, &unchanged(&board));
    }

    #[test]
    fn flipping_side_to_move_makes_a_board_unequal(board in any_board()) {
        let flipped = with_state(
            &board,
            true,
            board.castling_rights(),
            board.en_passant(),
            board.halfmove_clock(),
            board.fullmove_number(),
        );
        prop_assert_ne!(&board, &flipped);
    }

    /// Compares against `CastlingRights::NONE` or, when the board already has
    /// no rights, against all four. Perturbing toward a fixed value would
    /// silently become a no-op on every board that already held it.
    #[test]
    fn changing_castling_rights_makes_a_board_unequal(board in any_board()) {
        let all = CastlingRights::NONE
            .with(CastlingRights::WHITE_KINGSIDE)
            .with(CastlingRights::WHITE_QUEENSIDE)
            .with(CastlingRights::BLACK_KINGSIDE)
            .with(CastlingRights::BLACK_QUEENSIDE);
        let different = if board.castling_rights() == CastlingRights::NONE {
            all
        } else {
            CastlingRights::NONE
        };
        let changed = with_state(
            &board,
            false,
            different,
            board.en_passant(),
            board.halfmove_clock(),
            board.fullmove_number(),
        );
        prop_assert_ne!(&board, &changed);
    }

    /// `any_board` always produces `None` here, so this sets a square; the
    /// `Some`-to-`None` direction is covered by the concrete test below, which
    /// does not depend on the strategy's current behaviour staying that way.
    #[test]
    fn setting_an_en_passant_square_makes_a_board_unequal(
        board in any_board(),
        sq in any_square(),
    ) {
        prop_assume!(board.en_passant() != Some(sq));
        let changed = with_state(
            &board,
            false,
            board.castling_rights(),
            Some(sq),
            board.halfmove_clock(),
            board.fullmove_number(),
        );
        prop_assert_ne!(&board, &changed);
    }

    /// `wrapping_add` rather than `+ 1`: `any_board` fixes the clock at 0 today,
    /// but a future strategy generating 255 should not make this test panic
    /// instead of fail.
    #[test]
    fn changing_the_halfmove_clock_makes_a_board_unequal(board in any_board()) {
        let changed = with_state(
            &board,
            false,
            board.castling_rights(),
            board.en_passant(),
            board.halfmove_clock().wrapping_add(1),
            board.fullmove_number(),
        );
        prop_assert_ne!(&board, &changed);
    }

    #[test]
    fn changing_the_fullmove_number_makes_a_board_unequal(board in any_board()) {
        let changed = with_state(
            &board,
            false,
            board.castling_rights(),
            board.en_passant(),
            board.halfmove_clock(),
            board.fullmove_number().wrapping_add(1),
        );
        prop_assert_ne!(&board, &changed);
    }

    /// Recolouring a piece in place: `by_piece` and the occupancy are identical
    /// on both sides, so only `by_color` and `mailbox` differ. Without this,
    /// a comparison that dropped `by_color` entirely would still pass
    /// everything else here.
    #[test]
    fn recolouring_one_piece_makes_a_board_unequal(
        (board, sq) in any_board_and_occupied_square(),
    ) {
        let cp = board.piece_at(sq).expect("the strategy only yields occupied squares");

        let mut changed = board;
        changed.remove(sq);
        changed.place(sq, ColoredPiece::new(cp.color().flip(), cp.piece()));
        prop_assert_ne!(&board, &changed);
    }

    /// The mirror of the above: same colour, different piece type, so
    /// `by_color` and the occupancy match and only `by_piece` and `mailbox`
    /// differ.
    #[test]
    fn changing_one_piece_type_makes_a_board_unequal(
        (board, sq) in any_board_and_occupied_square(),
    ) {
        let cp = board.piece_at(sq).expect("the strategy only yields occupied squares");
        let other = if cp.piece() == Piece::Knight {
            Piece::Bishop
        } else {
            Piece::Knight
        };

        let mut changed = board;
        changed.remove(sq);
        changed.place(sq, ColoredPiece::new(cp.color(), other));
        prop_assert_ne!(&board, &changed);
    }

    /// Removing a piece outright, which changes occupancy as well as the
    /// per-colour and per-piece boards. The coarsest of the placement cases and
    /// the only one that would survive a comparison keyed on occupancy alone.
    #[test]
    fn removing_a_piece_makes_a_board_unequal(
        (board, sq) in any_board_and_occupied_square(),
    ) {
        let mut changed = board;
        changed.remove(sq);
        prop_assert_ne!(&board, &changed);
    }
}

/// The `Some`-to-`None` en passant direction, pinned concretely because
/// `any_board` never generates a board that already has an en passant square,
/// so the property above can only ever test `None` to `Some`.
#[test]
fn clearing_an_en_passant_square_makes_a_board_unequal() {
    let with_ep =
        Board::try_from_fen("rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 2")
            .expect("valid FEN");
    let without_ep =
        Board::try_from_fen("rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
            .expect("valid FEN");

    assert_eq!(
        with_ep.en_passant(),
        Some(Square::C6),
        "the two FENs must differ in exactly the en passant field for this to test anything"
    );
    assert_eq!(
        without_ep.en_passant(),
        None,
        "sanity check on the other side"
    );
    assert_ne!(
        with_ep, without_ep,
        "boards differing only in en passant must not compare equal"
    );
}
