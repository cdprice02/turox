//! Draw detection: the fifty-move rule and threefold repetition, both
//! checked before search recurses into a position, so a search line that
//! finds a draw stops there instead of searching arbitrarily deep into an
//! already-decided position.

use crate::board::Board;

/// Whether the fifty-move rule already applies to `board`: fifty full moves
/// (100 half-moves) since the last pawn move or capture, per FIDE Article
/// 9.3, without either side having to actually claim it. Reads
/// `Board::halfmove_clock()` directly; no separate state to track.
pub fn is_fifty_move_draw(board: &Board) -> bool {
    board.halfmove_clock() >= 100
}

/// Whether `current_hash` has already occurred at least twice in `history`,
/// meaning the position it belongs to is itself the third occurrence: a
/// threefold repetition, and a draw.
///
/// `history` holds the hashes of every position on the path leading up to
/// (but not including) `current_hash` itself, so a caller doesn't need to
/// push before checking (and can't get the push/check order backwards).
/// Search's own copy grows by one push per ply, seeded at the start of a
/// search with the real game history, not just the current search call's
/// own path, so repetitions that actually happened in the game are visible
/// too, not only ones the search tree itself revisits.
///
/// Called at every search node, so this stops scanning as soon as a second
/// match is found (`filter(...).nth(1)`) rather than walking the whole
/// slice the way `filter(...).count() >= 2` would.
pub fn is_threefold_repetition(history: &[u64], current_hash: u64) -> bool {
    history
        .iter()
        .filter(|&&h| h == current_hash)
        .nth(1)
        .is_some()
}

/// Either draw condition at once: the single check a search node makes
/// before recursing further. `current_hash` is `board.hash()`; taken
/// separately rather than recomputed here since a caller tracking `history`
/// already has it on hand.
pub fn is_draw(board: &Board, history: &[u64], current_hash: u64) -> bool {
    is_fifty_move_draw(board) || is_threefold_repetition(history, current_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifty_move_draw_triggers_at_exactly_100_half_moves() {
        let short = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 99 1").expect("valid FEN");
        assert!(!is_fifty_move_draw(&short));

        let exact = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 100 1").expect("valid FEN");
        assert!(is_fifty_move_draw(&exact));

        let over = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 150 1").expect("valid FEN");
        assert!(
            is_fifty_move_draw(&over),
            "still a draw well past the threshold, not just at it"
        );
    }

    #[test]
    fn no_repetition_on_an_empty_history() {
        assert!(!is_threefold_repetition(&[], 42));
    }

    #[test]
    fn one_prior_occurrence_is_not_yet_a_threefold() {
        // Combined with the current position, that's only two total.
        assert!(!is_threefold_repetition(&[42], 42));
    }

    #[test]
    fn two_prior_occurrences_make_the_current_position_the_third() {
        assert!(is_threefold_repetition(&[42, 42], 42));
    }

    #[test]
    fn two_prior_occurrences_interleaved_with_other_positions_still_count() {
        // Order and adjacency don't matter, only the total count: a
        // repetition doesn't require the same position on consecutive plies.
        assert!(is_threefold_repetition(&[42, 7, 42, 13], 42));
    }

    #[test]
    fn unrelated_repeated_hashes_do_not_trigger_a_different_position() {
        assert!(!is_threefold_repetition(&[7, 7, 7], 42));
    }

    #[test]
    fn is_draw_triggers_on_fifty_move_alone_with_no_repetition() {
        let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 100 1").expect("valid FEN");
        assert!(is_draw(&board, &[], board.hash()));
    }

    #[test]
    fn is_draw_triggers_on_repetition_alone_with_a_fresh_clock() {
        let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");
        let history = [board.hash(), board.hash()];
        assert!(is_draw(&board, &history, board.hash()));
    }

    #[test]
    fn is_draw_is_false_when_neither_condition_holds() {
        let board = Board::try_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 10 1").expect("valid FEN");
        assert!(!is_draw(&board, &[], board.hash()));
    }
}
