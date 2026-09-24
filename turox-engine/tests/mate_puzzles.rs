//! Forced-mate fixtures shared by every selective-search feature.
//!
//! Each of late move reductions, reverse futility, null-move, futility and
//! late move pruning can lose a forced mate, and each loses it the same way:
//! the line is still in the tree, but the node that would have found it was
//! reduced, skipped or cut short. A search that no longer sees a mate it used
//! to see is a correctness failure that otherwise surfaces only as unexplained
//! strength loss, so the guard lives in one place rather than being rebuilt
//! per feature.
//!
//! The property is absolute rather than comparative: each puzzle names the
//! exact mate distance, so nothing needs a feature switch to compare against.
//! Any change that stops the search reaching a mate fails here.
//!
//! **Every puzzle is searched past its own mate distance as well as at it.**
//! Searching a mate in three at depth three proves very little about pruning:
//! the mating line is most of the tree. Searching the same puzzle several
//! plies deeper puts the mate inside a tree full of ordinary moves, which is
//! where a reduction or a margin actually gets the chance to prune it away.
//!
//! `tests/search.rs` keeps its own mate tests, which assert the mating *move*
//! and carry the provenance of each FEN. These assert the score, and exist to
//! be re-run rather than read.
//!
//! The deep searches make this one of the slower tests in the suite, and it
//! stays out of `#[ignore]` anyway: a guard that only runs in the deep job
//! does not guard the pull request that breaks it.

use turox_chess::board::Board;
use turox_engine::search::{Search, MATE};

/// A position with a forced mate for the side to move.
struct MatePuzzle {
    name: &'static str,
    fen: &'static str,
    /// Plies, not moves: mate in one is 1, mate in two is 3. This is the
    /// distance `MATE - n` encodes, so it is the number the assertion needs.
    mate_in_plies: u8,
    /// The second depth to search at, past `mate_in_plies`. Only this one is
    /// free: the other depth a puzzle is worth searching at is its own mate
    /// distance, so naming it separately would only create a way for the two
    /// to disagree.
    ///
    /// One extra depth rather than every depth in between, because
    /// `Search::search` iterates from depth 1: a puzzle searched at 5, 6 and 7
    /// re-searches the shallow plies three times over for the same coverage.
    deep_depth: u8,
}

/// Whether the mating side is the one to move is not incidental: a mate
/// distance formula crossed with a colour is the shape that has produced
/// scrambled bugs in this engine before, so both colours mate here.
const PUZZLES: &[MatePuzzle] = &[
    MatePuzzle {
        name: "back-rank mate, White mating",
        fen: "6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1",
        mate_in_plies: 1,
        deep_depth: 7,
    },
    MatePuzzle {
        name: "back-rank mate, Black mating",
        fen: "r3k3/8/8/8/8/8/5PPP/6K1 b - - 0 1",
        mate_in_plies: 1,
        deep_depth: 7,
    },
    MatePuzzle {
        name: "Philidor's Legacy, the smothered mate finish",
        fen: "5r1k/6pp/4Q2N/8/8/8/5PPP/6K1 w - - 4 3",
        mate_in_plies: 3,
        deep_depth: 7,
    },
    MatePuzzle {
        name: "rook ladder, White mating",
        fen: "7k/8/8/8/8/8/R7/1R5K w - - 0 1",
        mate_in_plies: 3,
        deep_depth: 7,
    },
    MatePuzzle {
        name: "rook ladder, Black mating",
        fen: "1r5k/r7/8/8/8/8/8/7K b - - 0 1",
        mate_in_plies: 3,
        deep_depth: 7,
    },
    MatePuzzle {
        name: "crowded board, forced mate in five plies",
        fen: "7k/8/8/8/3NN3/1PPPPP2/R5P1/1R4K1 w - - 0 1",
        mate_in_plies: 5,
        deep_depth: 7,
    },
];

#[test]
fn every_forced_mate_is_found_at_and_beyond_its_own_depth() {
    for puzzle in PUZZLES {
        let board = Board::try_from_fen(puzzle.fen).expect("valid FEN");
        let expected = MATE - i16::from(puzzle.mate_in_plies);
        assert!(
            puzzle.deep_depth > puzzle.mate_in_plies,
            "{}: a deep depth of {} is not past a mate at ply {}",
            puzzle.name,
            puzzle.deep_depth,
            puzzle.mate_in_plies
        );
        for depth in [puzzle.mate_in_plies, puzzle.deep_depth] {
            let mut search = Search::new(Vec::new());
            let result = search.search(&board, depth);
            assert_eq!(
                result.score, expected,
                "{} at depth {depth}: expected mate in {} plies",
                puzzle.name, puzzle.mate_in_plies
            );
        }
    }
}
