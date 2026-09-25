//! Late move reductions: how many plies to take off a move the ordering put
//! late.
//!
//! The policy lives here rather than inside `alpha_beta_loop` because the loop
//! is deliberately chess-ignorant: it owns the windowing, the re-search and the
//! guarantee that one move is always searched, and asks a caller what to do
//! with each move after the first. Keeping the answer a pure function is what
//! makes it testable without driving a whole search.

/// Shallowest depth worth reducing at.
///
/// Three because two cannot: a child of a depth-2 node is searched at depth 1,
/// and `alpha_beta_loop` clamps any reduction that would take a child below
/// that, so every request at depth 2 already comes back as nothing. Saying so
/// here rather than leaning on the clamp keeps the rule stated in one place a
/// reader can find.
const MIN_DEPTH: u8 = 3;

/// How many moves are searched at full depth before the rest are treated as
/// late.
///
/// The ordering has to earn the assumption that what it put late is bad, and
/// three is the conventional price. Too low and the reduction lands on moves
/// the ordering had no real opinion about; too high and most of the tree is
/// searched at full depth anyway.
const MIN_SEARCHED: usize = 3;

/// Width of [`REDUCTIONS`] on both axes. Reductions saturate well below this,
/// so depths and move counts past it clamp to the last row or column rather
/// than growing the table to hold values that no longer change.
const DIM: usize = 32;

/// `floor(0.75 + ln(depth) * ln(searched) / 2.25)`, the conventional
/// logarithmic reduction, indexed `[depth][searched]`.
///
/// A literal table rather than a computation, because `f64::ln` is not `const`
/// and the alternatives each cost something on the hottest path in the engine:
/// a lazily initialised static pays an atomic check per lookup, and an integer
/// approximation of `ln` trades a table nobody has to read for arithmetic
/// everybody does. `the_table_matches_the_formula_it_claims_to_hold` is what
/// keeps these numbers honest.
#[rustfmt::skip]
const REDUCTIONS: [[u8; DIM]; DIM] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
    [0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3],
    [0, 0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3],
    [0, 0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3],
    [0, 0, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3],
    [0, 0, 1, 1, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4],
    [0, 0, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
    [0, 0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5],
];

/// The table's value for this depth and move count, clamped into range.
fn table(depth: u8, searched: usize) -> u8 {
    let depth = usize::from(depth).min(DIM - 1);
    let searched = searched.min(DIM - 1);
    REDUCTIONS[depth][searched]
}

/// How many plies to take off the move at this point in the loop, `0` for none.
///
/// `searched` rather than the move's index: the two are equal until something
/// starts skipping moves, and a count of moves *reached* would begin reducing
/// before three had actually been searched at full depth.
///
/// The caller turns a non-zero answer into `Verdict::Reduce`. Clamping a
/// reduction so the child still has depth to search is the loop's job, not
/// this function's.
pub(in crate::search) fn reduction(depth: u8, searched: usize, is_quiet: bool, is_pv: bool) -> u8 {
    if depth < MIN_DEPTH || searched < MIN_SEARCHED || !is_quiet {
        return 0;
    }

    let plies = table(depth, searched);
    if is_pv {
        plies.saturating_sub(1)
    } else {
        plies
    }
}

#[cfg(test)]
mod tests {
    use super::{reduction, table, DIM, REDUCTIONS};

    /// `floor(0.75 + ln(d) * ln(i) / 2.25)`, asserted as bounds on the stored
    /// value rather than by converting the float, which keeps the check free of
    /// both a lossy cast and a float comparison.
    fn assert_matches_formula(d: usize, i: usize) {
        let stored = f64::from(REDUCTIONS[d][i]);
        if d < 1 || i < 1 {
            assert_eq!(REDUCTIONS[d][i], 0, "ln is undefined at {d}, {i}");
            return;
        }
        let d = f64::from(u32::try_from(d).expect("index fits"));
        let i = f64::from(u32::try_from(i).expect("index fits"));
        let want = 0.75 + d.ln() * i.ln() / 2.25;
        assert!(
            stored <= want && want < stored + 1.0,
            "table holds {stored} at depth {d}, searched {i}, but the formula gives {want}"
        );
    }

    /// The table is a literal, so nothing but this connects it to the formula
    /// its doc claims it holds. A transcription slip in any one of its 1024
    /// entries would otherwise be invisible.
    #[test]
    fn the_table_matches_the_formula_it_claims_to_hold() {
        for d in 0..DIM {
            for i in 0..DIM {
                assert_matches_formula(d, i);
            }
        }
    }

    #[test]
    fn depth_and_lateness_past_the_table_clamp_to_its_edge() {
        let edge = REDUCTIONS[DIM - 1][DIM - 1];
        assert_eq!(table(200, 500), edge, "both axes past the end");
        assert_eq!(
            table(200, 4),
            REDUCTIONS[DIM - 1][4],
            "depth past the end, lateness inside it"
        );
        assert_eq!(
            table(4, 500),
            REDUCTIONS[4][DIM - 1],
            "lateness past the end, depth inside it"
        );
    }

    // ---- The policy ----
    //
    // A move is reduced when it is quiet, the node has depth to spare, and
    // enough moves have already been searched at full depth. Everything else
    // answers zero, which the caller reads as "search it normally" rather than
    // as "reduce it by no plies".

    #[test]
    fn a_move_that_is_not_quiet_is_never_reduced() {
        for depth in [3, 8, 20, 31] {
            for searched in [3, 8, 20, 31] {
                assert_eq!(
                    reduction(depth, searched, false, false),
                    0,
                    "captures, promotions, killers and hash moves keep their full depth (depth {depth}, searched {searched})"
                );
                assert!(
                    reduction(depth, searched, true, false) > 0,
                    "otherwise this proves nothing: the same position reduces a quiet move (depth {depth}, searched {searched})"
                );
            }
        }
    }

    #[test]
    fn nothing_is_reduced_below_depth_three() {
        for depth in [0, 1, 2] {
            assert_eq!(
                reduction(depth, 20, true, false),
                0,
                "depth {depth} has nothing to give up"
            );
        }
        assert!(
            reduction(3, 20, true, false) > 0,
            "depth three is where reducing starts"
        );
    }

    #[test]
    fn nothing_is_reduced_until_three_moves_have_been_searched() {
        for searched in [0, 1, 2] {
            assert_eq!(
                reduction(20, searched, true, false),
                0,
                "only {searched} moves have been searched at full depth"
            );
        }
        assert!(
            reduction(20, 3, true, false) > 0,
            "the fourth move is the first late one"
        );
    }

    #[test]
    fn a_late_quiet_move_is_reduced_by_the_table() {
        for depth in [3, 8, 20, 31] {
            for searched in [3, 8, 20, 31] {
                assert_eq!(
                    reduction(depth, searched, true, false),
                    table(depth, searched),
                    "depth {depth}, searched {searched}"
                );
            }
        }
    }

    /// A principal variation node is the line the engine intends to play, so it
    /// gives up a ply of reduction to be less wrong about it.
    #[test]
    fn a_pv_node_reduces_one_ply_less() {
        for depth in [3, 8, 20, 31] {
            for searched in [3, 8, 20, 31] {
                let plain = reduction(depth, searched, true, false);
                assert!(
                    plain > 0,
                    "otherwise saturating_sub makes this vacuous (depth {depth}, searched {searched})"
                );
                assert_eq!(
                    reduction(depth, searched, true, true),
                    plain.saturating_sub(1),
                    "depth {depth}, searched {searched}"
                );
            }
        }
    }

    /// Deeper nodes and later moves can only ever be reduced more, never less.
    /// A table read with its axes swapped still satisfies every test above on
    /// the diagonal, and fails this one off it.
    #[test]
    fn reducing_never_eases_off_as_depth_or_lateness_grows() {
        for depth in 3..DIM {
            for searched in 3..DIM {
                let here = reduction(u8::try_from(depth).expect("fits"), searched, true, false);
                if depth + 1 < DIM {
                    let deeper = reduction(
                        u8::try_from(depth + 1).expect("fits"),
                        searched,
                        true,
                        false,
                    );
                    assert!(
                        deeper >= here,
                        "depth {depth} reduces {here}, depth {} reduces {deeper}",
                        depth + 1
                    );
                }
                if searched + 1 < DIM {
                    let later = reduction(
                        u8::try_from(depth).expect("fits"),
                        searched + 1,
                        true,
                        false,
                    );
                    assert!(
                        later >= here,
                        "searched {searched} reduces {here}, searched {} reduces {later}",
                        searched + 1
                    );
                }
            }
        }
    }
}
