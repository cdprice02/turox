//! Property tests for FEN parsing/formatting (`Board::try_from_fen`/`to_fen`).

mod common;

use common::{any_color, any_piece_with_king, any_square};
use proptest::prelude::*;
use turox_engine::board::Board;

/// A `Board` strategy: places between 2 and 24 random (color, piece, square)
/// triples, skipping squares already taken. Not guaranteed "legal" chess-wise
/// (may have no king, doubled kings, pawns on rank 1, etc.); that's fine, FEN
/// round-tripping doesn't care, and legality is `move_gen`'s job, not `board`'s.
/// `any_piece_with_king`, not `common::any_piece`: this file's whole point is
/// exercising the no-king/doubled-king cases `common::any_piece`'s king-free
/// distribution would silently stop generating.
fn any_board() -> impl Strategy<Value = Board> {
    use turox_engine::ColoredPiece;

    proptest::collection::vec((any_color(), any_piece_with_king(), any_square()), 2..24).prop_map(
        |placements| {
            let mut board = Board::default();
            for (color, piece, sq) in placements {
                if board.piece_at(sq).is_none() {
                    board.place(sq, ColoredPiece::new(color, piece));
                }
            }
            board
        },
    )
}

proptest! {
    #[test]
    fn fen_round_trips(board in any_board()) {
        let fen = board.to_fen();
        let parsed = Board::try_from_fen(&fen).expect("Board::to_fen output must parse");
        prop_assert_eq!(board, parsed);
    }

    #[test]
    fn try_from_fen_never_panics_on_arbitrary_input(s in ".{0,64}") {
        // The historical bug this guards: an unbounded `rank -= 1` on `/` that
        // underflowed `usize` and panicked instead of returning `Err`. Wrapping in
        // catch_unwind turns any future regression of that shape into a normal
        // assertion failure instead of aborting the whole proptest run.
        let result = std::panic::catch_unwind(|| Board::try_from_fen(&s));
        prop_assert!(result.is_ok(), "try_from_fen panicked on input {s:?}");
    }
}
