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
//! **Every puzzle is searched past where it is found as well as at it.**
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
    /// distance `MATE - n` encodes, so it is the score the assertion expects,
    /// whatever depth the search needed to reach it.
    mate_in_plies: u8,
    /// The shallowest depth at which the search actually reports the mate.
    ///
    /// Equal to `mate_in_plies` for a full-width search, and larger wherever a
    /// selective technique has given up horizon: a reduced or pruned line needs
    /// more budget to reach the same mate. Recorded per puzzle rather than
    /// allowed for globally, so the cost of each technique is a number that
    /// changes visibly in a diff. A feature that pushes a mate one ply out is
    /// doing its job; one that pushes it five, or loses it, is not, and only a
    /// recorded figure tells those apart.
    found_at: u8,
    /// The second depth to search at, past `mate_in_plies`. Only this one is
    /// free: the other depth a puzzle is worth searching at is its own mate
    /// distance, so naming it separately would only create a way for the two
    /// to disagree.
    ///
    /// One extra depth rather than every depth in between, because
    /// `Search::search` iterates from depth 1: a puzzle searched at 5, 6 and 7
    /// re-searches the shallow plies three times over for the same coverage.
    deep_depth: u8,
    /// A node budget for both of this puzzle's searches, roughly twice what the
    /// deeper of the two costs.
    ///
    /// Recorded per puzzle for the same reason `found_at` is: it is the cost of
    /// the deep search, so a technique that gives up cutoffs shows up here as a
    /// figure that has to be raised, rather than as a test that quietly got
    /// slower. The bound is worth having because losing a mate and losing
    /// pruning are different failures with the same symptom at these depths.
    /// Unbounded, a search that no longer cuts anything off keeps looking for
    /// the mate until something outside the test gives up, and a test that
    /// never finishes says less than one coming back with the wrong score.
    ///
    /// Twice rather than a wider margin, because a budget also costs what it
    /// allows: every mutant that trips one pays for the search up to it, and
    /// these puzzles run once per mutant. The counts are deterministic, so
    /// twice is headroom for retuning and not for a loaded machine.
    node_budget: u64,
}

/// Whether the mating side is the one to move is not incidental: a mate
/// distance formula crossed with a colour is the shape that has produced
/// scrambled bugs in this engine before, so both colours mate here.
const PUZZLES: &[MatePuzzle] = &[
    MatePuzzle {
        name: "back-rank mate, White mating",
        fen: "6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1",
        mate_in_plies: 1,
        found_at: 1,
        deep_depth: 7,
        node_budget: 60_000,
    },
    MatePuzzle {
        name: "back-rank mate, Black mating",
        fen: "r3k3/8/8/8/8/8/5PPP/6K1 b - - 0 1",
        mate_in_plies: 1,
        found_at: 1,
        deep_depth: 7,
        node_budget: 60_000,
    },
    MatePuzzle {
        name: "Philidor's Legacy, the smothered mate finish",
        fen: "5r1k/6pp/4Q2N/8/8/8/5PPP/6K1 w - - 4 3",
        mate_in_plies: 3,
        found_at: 3,
        deep_depth: 7,
        node_budget: 260_000,
    },
    MatePuzzle {
        name: "rook ladder, White mating",
        fen: "7k/8/8/8/8/8/R7/1R5K w - - 0 1",
        mate_in_plies: 3,
        found_at: 3,
        deep_depth: 7,
        node_budget: 210_000,
    },
    MatePuzzle {
        name: "rook ladder, Black mating",
        fen: "1r5k/r7/8/8/8/8/8/7K b - - 0 1",
        mate_in_plies: 3,
        found_at: 3,
        deep_depth: 7,
        node_budget: 210_000,
    },
    MatePuzzle {
        name: "crowded board, forced mate in five plies",
        fen: "7k/8/8/8/3NN3/1PPPPP2/R5P1/1R4K1 w - - 0 1",
        mate_in_plies: 5,
        // Late move reductions cost a ply here: the mating line's later moves
        // are quiet and ordered late among forty, so at a depth-5 budget the
        // reduced line no longer reaches the mate. Depth 6 finds it, and finds
        // it at the true distance rather than a wrong one.
        found_at: 6,
        deep_depth: 7,
        node_budget: 370_000,
    },
    // The three rook ladders below are the deep half of this set, and they are
    // here for the shape of their mating lines rather than their length. Each
    // one needs a quiet waiting move partway through (`Kh2` in both sevens,
    // and `Kh2` plus a second quiet rook move in the nine), because the ladder
    // runs out of checks and has to hand the move back to reach the mate. A
    // quiet move late in a long list is precisely what a reduction shortens and
    // what a futility margin discards, so a mate that depends on one is the
    // case a selective technique fails at. A line of nothing but checks would
    // not test that, since every move in it is forcing.
    //
    // Sparse on purpose, as the counterpart to the crowded mate above: few
    // pieces buy the depth these need at a runtime the pull request can afford,
    // while the crowded position buys a wide move list at shallow depth. One
    // position cannot be both.
    MatePuzzle {
        name: "rook ladder against a king on e7, White mating",
        fen: "8/4k3/8/8/8/8/R7/1R5K w - - 0 1",
        mate_in_plies: 7,
        found_at: 7,
        deep_depth: 10,
        node_budget: 2_800_000,
    },
    MatePuzzle {
        name: "rook ladder against a king on e7, Black mating",
        fen: "1r5k/r7/8/8/8/8/4K3/8 b - - 0 1",
        mate_in_plies: 7,
        found_at: 7,
        deep_depth: 10,
        node_budget: 3_000_000,
    },
    // Two plies longer than the sevens above for one file of king travel, and
    // the only one of the three whose mirror does not belong here: mated from
    // d7 the line is found at its own depth, and the rank-flipped position with
    // the colours swapped needs two plies more to find the same mate at the
    // same distance. Both report the true distance once they see it, so the
    // mate-score formula agrees across colours and it is the search that
    // differs, but a puzzle costs roughly twice as much per ply and the mirror
    // would be the slowest entry here by a wide margin.
    MatePuzzle {
        name: "rook ladder against a king on d7, White mating",
        fen: "8/3k4/8/8/8/8/R7/1R5K w - - 0 1",
        mate_in_plies: 9,
        found_at: 9,
        deep_depth: 11,
        node_budget: 7_100_000,
    },
];

#[test]
fn every_forced_mate_is_found_at_and_beyond_its_own_depth() {
    for puzzle in PUZZLES {
        let board = Board::try_from_fen(puzzle.fen).expect("valid FEN");
        let expected = MATE - i16::from(puzzle.mate_in_plies);
        assert!(
            puzzle.found_at >= puzzle.mate_in_plies,
            "{}: cannot find a mate at ply {} with only {} plies of budget",
            puzzle.name,
            puzzle.mate_in_plies,
            puzzle.found_at
        );
        assert!(
            puzzle.deep_depth > puzzle.found_at,
            "{}: a deep depth of {} is not past where the mate is found, {}",
            puzzle.name,
            puzzle.deep_depth,
            puzzle.found_at
        );
        for depth in [puzzle.found_at, puzzle.deep_depth] {
            let mut search = Search::new(Vec::new()).with_max_nodes(puzzle.node_budget);
            let result = search.search(&board, depth);
            assert_eq!(
                result.depth, depth,
                "{}: the search stopped at depth {} of {depth}, so it reached this puzzle's node_budget of {}: the tree has outgrown the budget",
                puzzle.name, result.depth, puzzle.node_budget
            );
            assert_eq!(
                result.score, expected,
                "{} at depth {depth}: expected mate in {} plies",
                puzzle.name, puzzle.mate_in_plies
            );
        }
    }
}
