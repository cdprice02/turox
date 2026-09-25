//! Deciding which move to try first, and learning from what worked.
//!
//! Alpha-beta prunes in proportion to how early the best move is tried, so
//! this is what makes the search cheap rather than what makes it correct.
//!
//! [`MoveOrdering`] owns every table that feeds that decision, because they
//! form a closed loop: [`MoveOrdering::order`] reads them, and a beta cutoff
//! writes them back through [`MoveOrdering::on_cutoff`] and
//! [`MoveOrdering::update_history`]. Nothing outside this module reads what
//! they hold. [`stats`] is how the loop is checked from outside.

pub mod history;
mod killers;
mod priority;
pub mod stats;

use crate::eval::{Score, PIECE_VALUES};
use crate::search::is_mate_score;
use history::CutoffHistory;
use killers::KillerTable;
use priority::MovePriority;
use stats::CutoffCause;
use std::cmp::Reverse;
use turox_chess::board::Board;
use turox_chess::move_gen::move_list::MoveList;
use turox_chess::types::Move;
use turox_chess::MoveFlags;
use turox_chess::Piece;

pub(in crate::search) use priority::PriorityRuns;

/// Ply bound for every per-ply side table this module keeps: the killers, the
/// mate killers, the hash moves and the previous iteration's principal
/// variation. A ply past it reads and writes the last slot rather than
/// panicking, which quiescence's uncapped in-check evasion recursion can
/// reach.
///
/// Here rather than beside any one of those tables, because it is the bound
/// they share; moving it next to one would make the others import their size
/// from an unrelated concept.
const MAX_TRACKED_PLY: usize = 512;

/// Every table that decides which move a node tries first.
///
/// Held by `Search` as a single field rather than as five, so that the tables
/// a cutoff writes and the pass that reads them cannot drift apart: a new
/// ordering technique adds a table here and a branch in
/// [`MoveOrdering::on_cutoff`], and `Search` does not change at all.
pub(in crate::search) struct MoveOrdering<'a> {
    /// Scratch for one ordering pass: every move paired with the key it sorts
    /// on, so the key is computed once per move instead of once per comparison.
    ///
    /// Lives here rather than in [`Self::order`]'s own frame because one
    /// buffer is enough, ordering never re-entering itself, and a two-kilobyte
    /// local inlined into `negamax` would ride on every frame of a deep
    /// recursion. Only the first `moves.len()` entries are ever live: the tail
    /// holds whatever a longer list left behind and is never read.
    prioritized_moves: [(Reverse<(MovePriority, Score)>, Move); MoveList::CAPACITY],
    /// This search tree's killer moves, per ply.
    ///
    /// Tree-scoped rather than session-scoped like `tt`: a killer is a
    /// refutation specific to this tree's shape, not a fact about a position
    /// that is still true the next time `go` runs. By the same reasoning it is
    /// not reset between iterative-deepening iterations, which re-explore one
    /// tree at increasing depth.
    killers: KillerTable,
    /// The move this ply's move loop ordered by, if any: the transposition
    /// table's move for the position being searched there.
    ///
    /// Per ply for the same reason `killers` is, and clamped the same way: one
    /// position is being searched at each ply at a time, so a ply's slot is
    /// only ever about its own node, and a child writing its own slot cannot
    /// disturb an ancestor's.
    ///
    /// It holds what the loop *ordered by*, not what the table knows. Those
    /// differ: quiescence never orders by a hash move even where the table has
    /// one, so it writes `None` and its cutoffs are never attributed to the
    /// table. Attribution has to follow the ordering, or it credits a technique
    /// that did not put the move first.
    hash_moves: [Option<Move>; MAX_TRACKED_PLY],
    /// The previous iterative-deepening iteration's completed best line, one move
    /// per ply, frozen once that iteration finishes and consulted (never mutated)
    /// by [`Self::move_priority`] for the whole of the next. `None` past however
    /// deep that line actually ran, the same convention `hash_moves`/`killers`
    /// carry for an untouched ply, and unconditionally `None` at ply 0 of the very
    /// first iteration, when there is no previous iteration yet.
    ///
    /// Deliberately flat, not the triangular structure PVS's own in-progress PV
    /// tracking needs: once an iteration completes, its whole best line collapses
    /// to exactly one move per ply, so this only ever needs to answer "what was
    /// the move at this ply," never "what is the rest of the line from here."
    /// Trusted ahead of `hash_moves` for that ply (see [`MovePriority`]'s own
    /// doc): a hash move can be a different, more recently searched, position's
    /// entry if the table's replacement scheme has since overwritten this one, but
    /// this line is this search's own, uncorrupted record of what actually played
    /// out.
    previous_pv_line: [Option<Move>; MAX_TRACKED_PLY],
    /// Quiet-move ordering scores independent of any one search tree, unlike `killers`:
    /// it accumulates over many more nodes than two killer slots ever see, so it is
    /// threaded in from `uci::session::run` rather than owned here, the same way `tt` is.
    /// `None` by default (`negamax` orders and records quiets exactly as if history
    /// didn't exist): set via `Search::with_cutoff_history`.
    cutoff_history: Option<&'a mut CutoffHistory>,
}

impl<'a> MoveOrdering<'a> {
    /// Empty tables and no history: the ordering a search starts from, which
    /// is MVV-LVA on captures and declaration order on everything else.
    pub(in crate::search) const fn new() -> Self {
        Self {
            prioritized_moves: [(Reverse((MovePriority::Quiet, 0)), Move::SENTINEL);
                MoveList::CAPACITY],
            killers: KillerTable::new(),
            hash_moves: [None; MAX_TRACKED_PLY],
            previous_pv_line: [None; MAX_TRACKED_PLY],
            cutoff_history: None,
        }
    }

    /// Threads in the session-scoped history table; see the field's own doc
    /// for why that one is borrowed where the rest are owned.
    pub(in crate::search) const fn set_cutoff_history(&mut self, history: &'a mut CutoffHistory) {
        self.cutoff_history = Some(history);
    }

    /// Records what this ply's move loop is ordering by, which every loop must
    /// do before running, including the loops that order by nothing.
    ///
    /// Passing `None` is not a formality: it is what keeps a ply's slot about
    /// its own node rather than whatever last occupied that depth.
    pub(in crate::search) fn set_hash_move(&mut self, ply: u8, m: Option<Move>) {
        self.hash_moves[usize::from(ply).min(MAX_TRACKED_PLY - 1)] = m;
    }

    /// This ply's hash move; see [`Self::set_hash_move`].
    fn hash_move(&self, ply: u8) -> Option<Move> {
        self.hash_moves[usize::from(ply).min(MAX_TRACKED_PLY - 1)]
    }

    /// This ply's move in the previous iteration's completed best line; see
    /// [`Self::previous_pv_line`]. No setter alongside this one the way
    /// `hash_move` has `set_hash_move`: populating this is PVS's own job, done
    /// once per completed iteration from whatever in-progress structure it
    /// builds the line in, not once per node the way a hash move is recorded.
    fn previous_pv_move(&self, ply: u8) -> Option<Move> {
        self.previous_pv_line[usize::from(ply).min(MAX_TRACKED_PLY - 1)]
    }

    /// Orders `moves` in place, most promising first, so alpha-beta prunes more
    /// of the tree: MVV-LVA (most valuable victim, least valuable attacker)
    /// among captures, promotions interleaved onto that same material scale,
    /// ahead of quiet moves, since a capture or promotion that wins the most
    /// material is most likely to hold up and cause a beta cutoff early.
    /// `ply`'s previous-iteration PV move and hash move rank ahead of all of
    /// that; see [`Self::move_priority`]. `ply`'s two killer slots rank below
    /// captures but above the remaining quiet moves; see [`Self::move_priority`].
    /// `self.cutoff_history`, when `Some`, breaks ties among the remaining
    /// quiets by their stored score; `None` orders them exactly as if it
    /// didn't exist (every untouched quiet ties at `0`).
    ///
    /// En passant's victim isn't actually on `m.to()`; scored as `0` (an
    /// equal-value trade) rather than looking up the true victim square, an
    /// accepted ordering approximation since it only affects search order, not
    /// legality or correctness.
    ///
    /// Decorate, sort, undecorate, by way of [`Self::prioritized_moves`].
    /// Sorting the move list directly has `sort_unstable_by_key` recompute
    /// [`Self::move_priority`] on every comparison rather than once per move,
    /// and that key does a killer lookup, a cutoff-history lookup and
    /// piece-value arithmetic each time it runs.
    ///
    /// The sort keys on the pair's priority alone and never on the pair, since
    /// the tuple's derived ordering would add the move itself as a final
    /// tiebreak and so break ties among equal priorities differently from a
    /// sort over the moves on their own.
    ///
    /// Returns where each priority's run falls in the ordered result, which is
    /// the half of the work a plain sort computes and throws away.
    pub(in crate::search) fn order(
        &mut self,
        board: &Board,
        moves: &mut MoveList,
        ply: u8,
    ) -> PriorityRuns {
        // Counted here rather than scanned off the sorted result, because
        // this loop already visits every move.
        let mut counts = [0u16; MovePriority::COUNT];
        for (i, &m) in moves.iter().enumerate() {
            let priority = self.move_priority(board, m, ply);
            self.prioritized_moves[i] = (Reverse(priority), m);
            counts[priority.0.rank()] += 1;
        }

        // The live prefix only: past it sit the leavings of a longer list.
        self.prioritized_moves[..moves.len()].sort_unstable_by_key(|entry| entry.0);

        for i in 0..moves.len() {
            moves[i] = self.prioritized_moves[i].1;
        }

        PriorityRuns::from_counts(&counts)
    }

    /// Captures and promotions interleave on one material-gain scale rather than
    /// promotions getting a fixed rank: a capturing promotion (e.g. a pawn
    /// taking a rook while queening) is worth strictly more than the same
    /// promotion alone, by the captured piece's value, and only a shared scale
    /// can express that. `PIECE_VALUES` gives every promotion piece strictly
    /// more value than a pawn (the smallest gain, to a knight, is still 220), so
    /// `gain` can never be zero or negative for a promotion: it always resolves
    /// to `WinningCapture`, never `EqualCapture` or `LosingCapture`, and because
    /// that check happens before any killer-table lookup, a promotion can never
    /// fall through to `Quiet`, `Killer`, or `MateKiller` either, with or
    /// without a capture attached.
    ///
    /// Returns `(MovePriority, Score)`, both halves naturally bigger-is-better and
    /// neither wrapped in `Reverse`: `MovePriority` is declared worst-first so its
    /// derived `Ord` already agrees with `Score`'s own ordering, and
    /// [`Self::order`] applies the one flip `sort_unstable_by_key`'s
    /// ascending sort needs at its own call site, not here. Tiers with no real
    /// number to break ties by (`PrincipalVariation`, `Hash`, `EqualCapture`,
    /// `Killer`) carry a placeholder `0` that never actually competes against
    /// anything: every move landing in one of those tiers already ties on the
    /// first element of the tuple by definition of "same tier."
    ///
    /// `ply` scopes every per-node lookup this makes (`self.previous_pv_move`,
    /// `self.hash_move`, `self.killers`); `self.cutoff_history` isn't
    /// ply-scoped, the same way it isn't ply-scoped on `self` itself.
    #[expect(
        clippy::expect_used,
        reason = "the flags decide which arm runs, so a promotion arm always has a promotion piece and a capture arm always has a victim; the from-square always holds the moving piece"
    )]
    fn move_priority(&self, board: &Board, m: Move, ply: u8) -> (MovePriority, Score) {
        if Some(m) == self.previous_pv_move(ply) {
            return (MovePriority::PrincipalVariation, 0);
        }
        if Some(m) == self.hash_move(ply) {
            return (MovePriority::Hash, 0);
        }

        let flags = m.flags();
        let promotion_delta = || {
            PIECE_VALUES[flags
                .promotion_piece()
                .expect("move is a promotion")
                .index()]
                - PIECE_VALUES[Piece::Pawn.index()]
        };
        let capture_delta = || {
            let attacker = board.piece_at(m.from()).expect("move has a piece").piece();
            let victim = board
                .piece_at(m.to())
                .expect("capture has a victim")
                .piece();
            PIECE_VALUES[victim.index()] - PIECE_VALUES[attacker.index()]
        };
        let score_delta = match flags {
            MoveFlags::Quiet
            | MoveFlags::DoublePawnPush
            | MoveFlags::KingCastle
            | MoveFlags::QueenCastle => {
                if self.killers.holds_mate(ply, m) {
                    return (MovePriority::MateKiller, 0);
                }
                if self.killers.holds(ply, m) {
                    return (MovePriority::Killer, 0);
                }
                let history_score = self.cutoff_history.as_deref().map_or(0, |history| {
                    let piece = board.piece_at(m.from()).expect("move has a piece").piece();
                    history.score(board.side_to_move(), piece, m.to())
                });
                return (MovePriority::Quiet, history_score);
            }
            MoveFlags::Capture => capture_delta(),
            MoveFlags::EnPassant => 0,
            MoveFlags::PromoteKnight
            | MoveFlags::PromoteBishop
            | MoveFlags::PromoteRook
            | MoveFlags::PromoteQueen => promotion_delta(),
            MoveFlags::PromoteCaptureKnight
            | MoveFlags::PromoteCaptureBishop
            | MoveFlags::PromoteCaptureRook
            | MoveFlags::PromoteCaptureQueen => capture_delta() + promotion_delta(),
        };
        match score_delta.cmp(&0) {
            std::cmp::Ordering::Greater => (MovePriority::WinningCapture, score_delta),
            std::cmp::Ordering::Equal => (MovePriority::EqualCapture, 0),
            std::cmp::Ordering::Less => (MovePriority::LosingCapture, score_delta),
        }
    }

    /// Called whenever a move causes a beta cutoff: updates the killer and
    /// mate-killer tables and reports which ordering technique put `m` early
    /// enough to cut off.
    ///
    /// Classification lives here rather than at the call site because this is
    /// already the one place that knows what each technique holds, and the
    /// techniques queued behind killers and mate killers (history,
    /// countermoves) each add their own table to this type and a branch here,
    /// not another return value threaded out to the loop.
    ///
    /// The hash move is read from [`Self::hash_move`] like the killers are
    /// read from their own table, so every ordering input this classifies
    /// against lives on `Self`. Checked first, because a stored move that is
    /// *also* a killer was tried early because the table named it, and
    /// crediting the killer table would overstate what it does. The mate
    /// killer is checked next, ahead of the ordinary killer table, matching
    /// [`MovePriority`]'s own ranking: a move can sit in both tables at once
    /// (mate killers are recorded into the ordinary table too, since neither
    /// recording is conditional on the other), and when it does, the mate
    /// killer is the more specific, higher-value fact about it.
    ///
    /// `score` is `m`'s own resulting score, the same value the caller's
    /// `max` already holds at the point it decides `alpha >= beta`: what
    /// decides whether this cutoff also populates the mate-killer table,
    /// via [`is_mate_score`].
    ///
    /// Recording is a no-op for anything that isn't a plain quiet move:
    /// captures and promotions are already ordered by MVV-LVA/material gain,
    /// so recording them as killers would duplicate that and waste a slot on
    /// information the ordering already has. Such a move can still *cause* a
    /// cutoff, and is still classified.
    pub(in crate::search) fn on_cutoff(&mut self, ply: u8, m: Move, score: Score) -> CutoffCause {
        // Recording happens whatever the cause turns out to be. Skipping it for
        // a hash move would quietly change which tables hold what, and so the
        // move ordering and the tree, which classification must not.
        let (was_already_a_killer, was_already_the_mate_killer) = if m.flags().is_material_neutral()
        {
            let held = self.killers.record(ply, m, is_mate_score(score));
            (held.killer, held.mate_killer)
        } else {
            (false, false)
        };

        if self.hash_move(ply) == Some(m) {
            CutoffCause::HashMove
        } else if was_already_the_mate_killer {
            CutoffCause::MateKiller
        } else if was_already_a_killer {
            CutoffCause::Killer
        } else {
            CutoffCause::Other
        }
    }

    /// Feeds every material-neutral move this node's loop tried into `self.cutoff_history`:
    /// a bonus for whichever one caused the cutoff (if any, and if it's material-neutral), a
    /// malus for every other material-neutral move searched before it. If the loop never
    /// found a cutoff at all (an all-node), every material-neutral move in `moves` gets the
    /// malus instead, since none of them caused one. A no-op when `self.cutoff_history` is
    /// `None`, the same convention an absent table carries everywhere else here.
    ///
    /// `moves` is the same already-ordered list the caller's own loop iterated
    /// (`search_root`'s, `negamax`'s, or quiescence's evasion loop's), so this needs no
    /// separate bookkeeping of which quiets were tried: everything up to `cutoff_index` (or
    /// everything, if there wasn't one) was tried by construction. `cutoff_index` is `None`
    /// when the loop ran to completion without a cutoff, `Some(i)` when it broke at index `i`.
    ///
    /// `depth` is the caller's own remaining-depth parameter for `search_root`/`negamax`;
    /// quiescence's evasion loop has no such quantity and passes a depth-equivalent derived
    /// from `qdepth`'s own countdown instead; see `quiescence`'s own doc for `qdepth`.
    /// Called independently of, and after, [`Self::on_cutoff`]: the two tables answer
    /// different questions over the same event stream and neither is scoped by the other's
    /// outcome.
    #[expect(
        clippy::expect_used,
        reason = "moves is the position's own already-ordered move list, so a move's from-square always holds the piece that made it"
    )]
    pub(in crate::search) fn update_history(
        &mut self,
        board: &Board,
        moves: &MoveList,
        cutoff_index: Option<usize>,
        depth: u8,
    ) {
        let Some(cutoff_history) = self.cutoff_history.as_deref_mut() else {
            return;
        };
        let side = board.side_to_move();
        let searched_end = cutoff_index.unwrap_or(moves.len());
        for &searched in &moves[..searched_end] {
            if searched.flags().is_material_neutral() {
                let piece = board
                    .piece_at(searched.from())
                    .expect("move has a piece")
                    .piece();
                cutoff_history.record_no_cutoff(side, piece, searched.to(), depth);
            }
        }
        if let Some(i) = cutoff_index {
            let cutoff_move = moves[i];
            if cutoff_move.flags().is_material_neutral() {
                let piece = board
                    .piece_at(cutoff_move.from())
                    .expect("move has a piece")
                    .piece();
                cutoff_history.record_cutoff(side, piece, cutoff_move.to(), depth);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testing::{capture_and_quiet_position, find_move};
    use crate::search::MATE;
    use turox_chess::board::Board;
    use turox_chess::move_gen::attacks::in_check;
    use turox_chess::move_gen::legal::legal_moves;
    use turox_chess::types::Square;

    // `move_priority` and `order` are private to this module, and both are
    // pure enough (no board mutation, no search recursion) to test directly
    // here rather than only through `Search::search` end to end, matching this
    // crate's convention of unit-testing a private pure function in-module and
    // reserving `tests/search.rs`/`search_props.rs` for `Search`'s own public
    // API.

    /// `find_move` alone is ambiguous for a promotion square: a pawn reaching the
    /// back rank has up to four legal moves sharing the same `from`/`to`, one per
    /// promotion piece, so the piece has to be part of the match.
    fn find_promotion_move(board: &Board, from: Square, to: Square, piece: Piece) -> Move {
        *legal_moves(board)
            .as_slice()
            .iter()
            .find(|m| {
                m.from() == from && m.to() == to && m.flags().promotion_piece() == Some(piece)
            })
            .unwrap_or_else(|| {
                panic!("{from:?}{to:?}={piece:?} must be a legal promotion in this position")
            })
    }

    /// A `MoveOrdering` seeded with exactly the ply-0 ordering inputs a test wants to
    /// exercise, so each `move_priority`/`order` test below can stay a
    /// short, direct call the way it was when these were free functions,
    /// rather than repeating this setup inline everywhere.
    fn priority_ordering(tt_move: Option<Move>, killers: &[Move]) -> MoveOrdering<'static> {
        let mut ordering = MoveOrdering::new();
        ordering.set_hash_move(0, tt_move);
        // Recorded oldest first, so the slice reads most-recent-first the way
        // the table orders its own slots.
        for &m in killers.iter().rev() {
            ordering.killers.record(0, m, false);
        }
        ordering
    }

    /// Every variant, in the exact order the intended hierarchy requires:
    /// `#[derive(Ord)]` on the enum compares by declaration order, so this list
    /// *is* the ranking, not a lookup table alongside it. A typo'd or reordered
    /// variant here wouldn't fail to compile, it would just silently misorder two
    /// tiers relative to each other. `WinningCapture`/`LosingCapture` carry a
    /// same-magnitude tiebreak value in this test since only cross-variant order
    /// is being checked; within-tier ordering has its own tests below.
    #[test]
    fn move_priority_rank_order_matches_the_intended_hierarchy() {
        use MovePriority::{
            EqualCapture, Hash, Killer, LosingCapture, MateKiller, PrincipalVariation, Quiet,
            WinningCapture,
        };
        let ranked_worst_to_best = [
            LosingCapture,
            Quiet,
            Killer,
            MateKiller,
            EqualCapture,
            WinningCapture,
            Hash,
            PrincipalVariation,
        ];
        for pair in ranked_worst_to_best.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} must rank strictly behind {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn move_priority_with_no_tt_hint_classifies_a_quiet_move_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0)
        );
    }

    // MVV-LVA's whole point: a small attacker taking a large victim (pawn
    // takes queen) is the most promising capture there is, tried before a
    // big attacker taking a small victim (queen takes pawn), which is
    // comparatively unpromising. `WinningCapture`/`LosingCapture` need to
    // land on the *correct side* of that distinction, not just land on two
    // different variants (a naming swap between the two would still produce
    // "two distinct variants" and could still pass a looser test). Pinning
    // the exact tiebreak value, not just the variant, also catches the
    // tiebreak's sign getting negated relative to the raw victim-minus-
    // attacker delta: it should carry that delta un-negated.
    #[test]
    fn move_priority_classifies_a_small_attacker_taking_a_big_victim_as_winning() {
        let board = Board::try_from_fen("4k3/8/8/8/3q4/4P3/8/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_queen = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, pawn_takes_queen, 0),
            (MovePriority::WinningCapture, 800),
            "a pawn capturing a queen is the textbook winning capture, gain = 900 - 100"
        );
    }

    #[test]
    fn move_priority_classifies_a_big_attacker_taking_a_small_victim_as_losing() {
        let board = Board::try_from_fen("4k3/8/8/8/3p4/4Q3/8/4K3 w - - 0 1").expect("valid FEN");
        let queen_takes_pawn = find_move(&board, Square::E3, Square::D4);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, queen_takes_pawn, 0),
            (MovePriority::LosingCapture, -800),
            "a queen capturing an undefended pawn is comparatively unpromising, \
             the opposite end of the scale from a pawn capturing a queen, gain = 100 - 900"
        );
    }

    #[test]
    fn move_priority_classifies_an_equal_value_capture_as_equal() {
        let board = Board::try_from_fen("4k3/8/8/8/3r4/8/8/3RK3 w - - 0 1").expect("valid FEN");
        let rook_takes_rook = find_move(&board, Square::D1, Square::D4);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, rook_takes_rook, 0),
            (MovePriority::EqualCapture, 0)
        );
    }

    /// `RxQ` and `PxN` are both winning captures, so a priority that carried no
    /// payload would rank them equal and throw away MVV-LVA's whole point. This
    /// pins that the material delta on `WinningCapture` actually breaks the tie,
    /// rather than both moves merely landing in the same broad tier.
    #[test]
    fn move_priority_does_not_conflate_two_different_winning_captures() {
        let rxq_board = Board::try_from_fen("q5k1/8/8/8/8/8/8/R3K3 w - - 0 1").expect("valid FEN");
        let rook_takes_queen = find_move(&rxq_board, Square::A1, Square::A8);

        let pxn_board =
            Board::try_from_fen("6k1/8/8/8/8/2n5/1P6/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_knight = find_move(&pxn_board, Square::B2, Square::C3);

        let rxq_priority =
            priority_ordering(None, &[]).move_priority(&rxq_board, rook_takes_queen, 0);
        let pxn_priority =
            priority_ordering(None, &[]).move_priority(&pxn_board, pawn_takes_knight, 0);

        assert_ne!(
            rxq_priority, pxn_priority,
            "RxQ (gain 400) and PxN (gain 220) must not compare equal"
        );
        assert!(
            rxq_priority > pxn_priority,
            "RxQ must outrank PxN: rook-for-queen is a bigger material swing \
             than pawn-for-knight, even though both are winning captures"
        );
        assert_eq!(rxq_priority, (MovePriority::WinningCapture, 400));
        assert_eq!(pxn_priority, (MovePriority::WinningCapture, 220));
    }

    /// A losing capture must sort behind a quiet move, not ahead of it: hanging
    /// a piece is worse than an untried ordinary move. This is what
    /// `LosingCapture`'s position in the declaration order buys, and the
    /// derived `Ord` makes that position load-bearing rather than cosmetic.
    #[test]
    fn move_priority_ranks_a_losing_capture_behind_a_quiet_move() {
        let board = capture_and_quiet_position();
        let losing_capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        assert!(
            priority_ordering(None, &[]).move_priority(&board, quiet, 0)
                > priority_ordering(None, &[]).move_priority(&board, losing_capture, 0),
            "a losing capture must sort behind a quiet move, not ahead of it"
        );
    }

    /// The direction, stated as its own property rather than two isolated
    /// classifications: whichever variants "winning" and "losing" end up
    /// being, a small-attacker/big-victim capture must outrank a
    /// big-attacker/small-victim one on the same squares. This is the
    /// property the two classification tests above only check indirectly
    /// (through whatever `MovePriority::Ord` says those variants are worth);
    /// this one checks the actual consequence.
    #[test]
    fn small_attacker_takes_big_victim_ranks_above_big_attacker_takes_small_victim() {
        let winning_board =
            Board::try_from_fen("4k3/8/8/8/3q4/4P3/8/4K3 w - - 0 1").expect("valid FEN");
        let pawn_takes_queen = find_move(&winning_board, Square::E3, Square::D4);

        let losing_board =
            Board::try_from_fen("4k3/8/8/8/3p4/4Q3/8/4K3 w - - 0 1").expect("valid FEN");
        let queen_takes_pawn = find_move(&losing_board, Square::E3, Square::D4);

        assert!(
            priority_ordering(None, &[]).move_priority(&winning_board, pawn_takes_queen, 0)
                > priority_ordering(None, &[]).move_priority(&losing_board, queen_takes_pawn, 0),
            "a pawn capturing a queen must be tried before a queen capturing a pawn"
        );
    }

    #[test]
    fn move_priority_with_matching_tt_hint_is_hash_regardless_of_move_shape() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        assert_eq!(
            priority_ordering(Some(capture), &[]).move_priority(&board, capture, 0),
            (MovePriority::Hash, 0)
        );
        assert_eq!(
            priority_ordering(Some(quiet), &[]).move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0)
        );
    }

    #[test]
    fn move_priority_with_non_matching_tt_hint_falls_back_to_normal_classification() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet = find_move(&board, Square::E1, Square::D1);

        // `quiet` is hinted, but `capture` is the move being scored: the
        // hint shouldn't affect a move it doesn't match.
        assert_eq!(
            priority_ordering(Some(quiet), &[]).move_priority(&board, capture, 0),
            priority_ordering(None, &[]).move_priority(&board, capture, 0),
            "a tt hint for a different move must not affect this move's own ordering"
        );
    }

    /// The direction check: a lazier stub that only satisfies the
    /// `move_priority`-level tests above (e.g. one that *deprioritizes* the
    /// hinted move instead of prioritizing it) could still pass every one of
    /// them if it never actually checked which way "outranks" needs to go.
    /// Ordering a real capture against a real, unrelated quiet hint is what
    /// would actually catch that: a backwards implementation sends the
    /// quiet hint to the back, not the front.
    #[test]
    fn order_ranks_an_unrelated_tt_hint_above_a_good_capture() {
        let board = capture_and_quiet_position();
        let mut moves = legal_moves(&board);

        let capture = find_move(&board, Square::E4, Square::D5);
        let quiet_hint = find_move(&board, Square::E1, Square::D1);

        priority_ordering(Some(quiet_hint), &[]).order(&board, &mut moves, 0);

        assert_eq!(
            moves.as_slice()[0],
            quiet_hint,
            "the tt-hinted quiet move must sort first, ahead of the available capture"
        );
        assert_ne!(
            moves.as_slice()[0],
            capture,
            "the capture must not outrank an unrelated tt hint"
        );
    }

    /// Whichever legal move is handed to `order` as the hint lands at
    /// index 0, whatever kind of move it is. Looping over every legal move in
    /// the position (the one capture and several quiet king moves) as the hint
    /// in turn checks that generically, rather than pinning it to one move's
    /// own kind and passing for the wrong reason.
    #[test]
    fn order_places_any_hinted_move_first() {
        let board = capture_and_quiet_position();
        let legal = legal_moves(&board);

        for &hint in legal.as_slice() {
            let mut moves = legal_moves(&board);
            priority_ordering(Some(hint), &[]).order(&board, &mut moves, 0);
            assert_eq!(
                moves.as_slice()[0],
                hint,
                "hint move {hint:?} must land at index 0 regardless of whether \
                 it's a capture or a quiet move"
            );
        }
    }

    /// A lone pawn one push from queening, nothing else on the board that
    /// could capture: the only loud move available is the promotion itself.
    fn promotion_no_capture_position() -> Board {
        Board::try_from_fen("7k/4P3/8/8/8/8/8/K7 w - - 0 1").expect("valid FEN")
    }

    /// Same pawn, same promotion square, but a rook sits on the capture
    /// diagonal so every promotion piece has a capturing and a non-capturing
    /// counterpart to compare against.
    fn promotion_with_capture_position() -> Board {
        Board::try_from_fen("5r1k/4P3/8/8/8/8/8/K7 w - - 0 1").expect("valid FEN")
    }

    #[test]
    fn move_priority_classifies_a_non_capture_promotion_as_a_winning_capture() {
        let board = promotion_no_capture_position();
        let queen_promo = find_promotion_move(&board, Square::E7, Square::E8, Piece::Queen);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, queen_promo, 0),
            (MovePriority::WinningCapture, 800),
            "a non-capture queen promotion must outrank quiet moves, not tie with \
             them: gain = 900 - 100, the same tier a good capture lands in"
        );
    }

    /// The reason to interleave promotions with captures on one material
    /// scale, rather than giving promotions their own fixed tier: a
    /// promotion that also captures a piece is worth strictly more than the
    /// same promotion alone, by exactly the captured piece's value.
    #[test]
    fn move_priority_ranks_a_capturing_promotion_above_a_plain_promotion_of_the_same_piece() {
        let plain_board = promotion_no_capture_position();
        let plain_promo = find_promotion_move(&plain_board, Square::E7, Square::E8, Piece::Queen);

        let capture_board = promotion_with_capture_position();
        let capturing_promo =
            find_promotion_move(&capture_board, Square::E7, Square::F8, Piece::Queen);

        let plain_priority =
            priority_ordering(None, &[]).move_priority(&plain_board, plain_promo, 0);
        let capturing_priority =
            priority_ordering(None, &[]).move_priority(&capture_board, capturing_promo, 0);

        assert!(
            capturing_priority > plain_priority,
            "capturing a rook while promoting must outrank promoting alone"
        );
        assert_eq!(plain_priority, (MovePriority::WinningCapture, 800));
        assert_eq!(
            capturing_priority,
            (MovePriority::WinningCapture, 1200),
            "gain = (900 - 100) promotion + (500 - 100) capture"
        );
    }

    /// `PIECE_VALUES` gives every promotion piece strictly more value than a
    /// pawn (the smallest promotion gain, to a knight, is still 220), so
    /// `move_priority`'s combined gain can never be zero or negative for a
    /// promotion. That's what lets the branch order check material gain
    /// before any killer-table lookup: a promotion can never fall through to
    /// `Quiet`, `Killer`, or `MateKiller`, with or without a capture attached,
    /// and this is a fact about the value table, not about any one FEN.
    #[test]
    fn every_promotion_piece_classifies_as_winning_never_quiet() {
        let plain_board = promotion_no_capture_position();
        let capture_board = promotion_with_capture_position();

        for piece in [Piece::Knight, Piece::Bishop, Piece::Rook, Piece::Queen] {
            let plain = find_promotion_move(&plain_board, Square::E7, Square::E8, piece);
            assert!(
                matches!(
                    priority_ordering(None, &[]).move_priority(&plain_board, plain, 0),
                    (MovePriority::WinningCapture, _)
                ),
                "{piece:?} promotion alone must classify as WinningCapture, never Quiet"
            );

            let capturing = find_promotion_move(&capture_board, Square::E7, Square::F8, piece);
            assert!(
                matches!(
                    priority_ordering(None, &[]).move_priority(&capture_board, capturing, 0),
                    (MovePriority::WinningCapture, _)
                ),
                "{piece:?} promotion with a capture must classify as WinningCapture, never Quiet"
            );
        }
    }
    // ---- Killer-move classification ----
    //
    // `MoveOrdering::move_priority`'s own `killer_slots(ply)` lookup carries the two
    // slots for the ply the move is being classified at. A killer only outranks
    // a quiet move, never a capture, so these check the boundary in both
    // directions rather than only that a match is recognised.

    #[test]
    fn move_priority_with_matching_first_killer_slot_classifies_as_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_ordering(None, &[quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Killer, 0)
        );
    }

    #[test]
    fn move_priority_with_matching_second_killer_slot_classifies_as_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let other_quiet = find_move(&board, Square::E1, Square::F1);
        assert_eq!(
            priority_ordering(None, &[other_quiet, quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Killer, 0),
            "both slots must be checked, not just the first"
        );
    }

    #[test]
    fn move_priority_with_no_matching_killer_slot_classifies_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let unrelated = find_move(&board, Square::E1, Square::F1);
        assert_eq!(
            priority_ordering(None, &[unrelated]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0),
            "a killer slot holding a different move must not affect this move's own classification"
        );
    }

    #[test]
    fn move_priority_with_empty_killer_slots_classifies_a_quiet_move_as_quiet() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_ordering(None, &[]).move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0)
        );
    }

    /// A killer slot is looked up at the ply `move_priority` is asked about, so
    /// a move recorded as a killer at one ply must not leak into a
    /// classification call for a different ply's slots. This is now a real
    /// property of `ply` itself, not (as when `killers` was a bare parameter)
    /// just a labeling convention on whatever array the caller happened to pass.
    #[test]
    fn move_priority_trusts_whichever_ply_it_is_asked_about() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        ordering.killers.record(5, quiet, false);
        assert_eq!(
            ordering.move_priority(&board, quiet, 5),
            (MovePriority::Killer, 0)
        );
        assert_eq!(
            ordering.move_priority(&board, quiet, 9),
            (MovePriority::Quiet, 0),
            "ply 9's own (empty) killer slots must be consulted, not ply 5's"
        );
    }

    #[test]
    fn move_priority_prefers_hash_over_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        assert_eq!(
            priority_ordering(Some(quiet), &[quiet]).move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0),
            "the tt hint must outrank a killer slot for the same move"
        );
    }

    // ---- Mate-killer classification ----
    //
    // Same shape as the ordinary killer tests above, seeding the killer table
    // rather than driving a real search to populate it: `move_priority`'s own
    // lookup is what is under test, not `on_cutoff`'s population logic,
    // which has its own section below.

    #[test]
    fn move_priority_with_matching_mate_killer_classifies_as_mate_killer() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        ordering.killers.record(0, quiet, true);
        assert_eq!(
            ordering.move_priority(&board, quiet, 0),
            (MovePriority::MateKiller, 0)
        );
    }

    #[test]
    fn move_priority_with_no_matching_mate_killer_falls_back_to_ordinary_classification() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let unrelated = find_move(&board, Square::E1, Square::F1);
        let mut ordering = MoveOrdering::new();
        ordering.killers.record(0, unrelated, true);
        assert_eq!(
            ordering.move_priority(&board, quiet, 0),
            (MovePriority::Quiet, 0),
            "a mate-killer slot holding a different move must not affect this move's own \
             classification"
        );
    }

    /// The whole reason `MateKiller` is a separate rank from `Killer`: a move
    /// sitting in both tables at once (recording one is never conditional on
    /// the other; see `on_cutoff`) must classify by the more specific,
    /// higher-value fact about it.
    #[test]
    fn move_priority_prefers_mate_killer_over_ordinary_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = priority_ordering(None, &[quiet]);
        ordering.killers.record(0, quiet, true);
        assert_eq!(
            ordering.move_priority(&board, quiet, 0),
            (MovePriority::MateKiller, 0),
            "a move sitting in both tables must classify as the mate killer, not the ordinary one"
        );
    }

    #[test]
    fn move_priority_prefers_hash_over_mate_killer_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = priority_ordering(Some(quiet), &[]);
        ordering.killers.record(0, quiet, true);
        assert_eq!(
            ordering.move_priority(&board, quiet, 0),
            (MovePriority::Hash, 0),
            "the tt hint must outrank a mate-killer slot for the same move"
        );
    }

    /// Ply-scoping, the same property `move_priority_trusts_whichever_ply_it_is_asked_about`
    /// pins for the ordinary killer table: a mate killer recorded at one ply
    /// must not leak into a classification call for a different ply.
    #[test]
    fn move_priority_trusts_whichever_ply_its_mate_killer_is_asked_about() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        ordering.killers.record(5, quiet, true);
        assert_eq!(
            ordering.move_priority(&board, quiet, 5),
            (MovePriority::MateKiller, 0)
        );
        assert_eq!(
            ordering.move_priority(&board, quiet, 9),
            (MovePriority::Quiet, 0),
            "ply 9's own (empty) mate-killer slot must be consulted, not ply 5's"
        );
    }

    // ---- Mate-killer recording (`on_cutoff`) ----
    //
    // `move_priority`'s own lookup is covered above; these instead drive
    // `on_cutoff` itself, the write side, directly. `on_cutoff`
    // is private and pure enough (no recursion, no board mutation beyond
    // `self`'s own tables) to test the same way `move_priority`/`order`
    // are, per this module's convention.

    #[test]
    fn on_cutoff_records_a_mate_killer_on_a_quiet_move_with_a_mate_score() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        let cause = ordering.on_cutoff(0, quiet, MATE - 1);
        assert!(
            ordering.killers.holds_mate(0, quiet),
            "a quiet move causing a cutoff with a mate score must populate this ply's mate killer"
        );
        assert_eq!(
            cause,
            CutoffCause::Other,
            "the first time a move is recorded, nothing had predicted it yet: crediting the \
             mate-killer technique here would overstate what it did, the same reasoning \
             `on_cutoff`'s own doc gives for the ordinary killer table"
        );
    }

    #[test]
    fn on_cutoff_credits_mate_killer_once_the_same_move_repeats_at_the_same_ply() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        ordering.on_cutoff(0, quiet, MATE - 1);
        let cause = ordering.on_cutoff(0, quiet, MATE - 1);
        assert_eq!(
            cause,
            CutoffCause::MateKiller,
            "a move already sitting in this ply's mate-killer slot must be credited to it \
             on its next cutoff"
        );
    }

    #[test]
    fn on_cutoff_does_not_record_a_mate_killer_for_an_ordinary_score() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = MoveOrdering::new();
        ordering.on_cutoff(0, quiet, 100);
        assert!(
            !ordering.killers.holds_mate(0, quiet),
            "an ordinary (non-mate) cutoff score must never populate the mate-killer slot"
        );
    }

    #[test]
    fn on_cutoff_never_records_a_mate_killer_for_a_capture() {
        let board = capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let mut ordering = MoveOrdering::new();
        ordering.on_cutoff(0, capture, MATE - 1);
        assert!(
            !ordering.killers.holds_mate(0, capture),
            "captures are already ordered by MVV-LVA; recording one as a mate killer would \
             waste the slot the same way recording it as an ordinary killer would"
        );
    }

    #[test]
    fn on_cutoff_mate_killer_slot_always_replaces() {
        let board = capture_and_quiet_position();
        let first = find_move(&board, Square::E1, Square::D1);
        let second = find_move(&board, Square::E1, Square::F1);
        let mut ordering = MoveOrdering::new();
        ordering.on_cutoff(0, first, MATE - 1);
        ordering.on_cutoff(0, second, MATE - 1);
        assert!(
            ordering.killers.holds_mate(0, second) && !ordering.killers.holds_mate(0, first),
            "one slot, always-replace: the most recent mate-scoring cutoff move wins and the \
             previous one is gone, with no promote/shift policy the way the two-slot ordinary \
             killer table has"
        );
    }

    // ---- Previous-iteration PV classification ----
    //
    // `PrincipalVariation` outranks `Hash` (see `MovePriority`'s own doc for
    // why): a hash-table entry can belong to a different, more recently
    // searched line once replacement has overwritten it, where
    // `MoveOrdering::previous_pv_line` is this search's own uncorrupted record.
    // `PVS`'s own recursion is what will ever populate that line for a real
    // search; these tests poke it directly, the same way the killer tests
    // above poke `killers` directly rather than running a real search to get
    // a slot filled.

    #[test]
    fn move_priority_prefers_previous_pv_move_over_hash_when_a_move_matches_both() {
        let board = capture_and_quiet_position();
        let quiet = find_move(&board, Square::E1, Square::D1);
        let mut ordering = priority_ordering(Some(quiet), &[]);
        ordering.previous_pv_line[0] = Some(quiet);
        assert_eq!(
            ordering.move_priority(&board, quiet, 0),
            (MovePriority::PrincipalVariation, 0),
            "the previous iteration's own pv move must outrank a same-move hash hint"
        );
    }

    /// The direction check, same shape as the tt-hint/killer versions above:
    /// ordering a real capture against a real, unrelated previous-pv hint is
    /// what would catch a backwards or no-op implementation, not just the
    /// `move_priority`-level classification test above.
    #[test]
    fn order_places_the_previous_pv_move_first() {
        let board = capture_and_quiet_position();
        let mut moves = legal_moves(&board);

        let capture = find_move(&board, Square::E4, Square::D5);
        let pv_hint = find_move(&board, Square::E1, Square::D1);

        let mut ordering = priority_ordering(None, &[]);
        ordering.previous_pv_line[0] = Some(pv_hint);
        ordering.order(&board, &mut moves, 0);

        assert_eq!(
            moves.as_slice()[0],
            pv_hint,
            "the previous iteration's pv move must sort first, ahead of the available capture"
        );
        assert_ne!(
            moves.as_slice()[0],
            capture,
            "the capture must not outrank an unrelated pv hint"
        );
    }

    /// Captures return from `move_priority` before the killer-slot lookup is
    /// ever reached, so a killer slot happening to hold the same `Move` value
    /// as a capture (impossible in practice: a capture and a quiet move to
    /// the same square carry different `MoveFlags`, so they're different
    /// `Move` values, but this pins the guarantee down directly rather than
    /// relying on that being true by construction elsewhere) still can't
    /// promote it out of the capture tiers.
    #[test]
    fn move_priority_never_classifies_a_capture_as_killer() {
        let board = capture_and_quiet_position();
        // Queen takes knight: a losing trade by this heuristic (attacker
        // outvalues victim), which is exactly why it's the right move to use
        // here. If a capture's own killer-slot lookup were ever reachable,
        // this is the tier that would most plausibly get displaced by it:
        // `LosingCapture` sits right next to `Killer` in the ranking (see
        // `move_priority_rank_order_matches_the_intended_hierarchy`), so a
        // bug that let a losing capture fall through to a killer check would
        // be invisible on a winning capture, which is nowhere near that
        // boundary.
        let capture = find_move(&board, Square::E4, Square::D5);
        assert!(matches!(
            priority_ordering(None, &[capture]).move_priority(&board, capture, 0),
            (MovePriority::LosingCapture, _)
        ));
    }

    /// A pawn one capture from a free knight, alongside several quiet king
    /// moves: unlike `capture_and_quiet_position`'s queen-takes-knight
    /// (a losing trade), `Pxd5` here is a genuine winning capture, which is
    /// what the killer-ordering test below needs to outrank a killer with.
    fn winning_capture_and_quiet_position() -> Board {
        Board::try_from_fen("4k3/8/8/3n4/4P3/8/8/4K3 w - - 0 1").expect("valid FEN")
    }

    /// The direction check, same shape as the tt-hint version above: a
    /// stub that only satisfies the `move_priority`-level tests could still
    /// pass them without ever actually reordering a real move list. Ordering
    /// a real capture against a real killer is what would catch a backwards
    /// implementation (one that sends the killer to the back instead of
    /// ahead of unrelated quiets).
    #[test]
    fn order_places_a_killer_ahead_of_an_unrelated_quiet_but_behind_a_capture() {
        let board = winning_capture_and_quiet_position();
        let capture = find_move(&board, Square::E4, Square::D5);
        let killer = find_move(&board, Square::E1, Square::D1);
        let other_quiet = find_move(&board, Square::E1, Square::F1);

        let mut moves = legal_moves(&board);
        priority_ordering(None, &[killer]).order(&board, &mut moves, 0);

        let killer_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == killer)
            .expect("killer is a legal move in this position");
        let capture_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == capture)
            .expect("capture is a legal move in this position");
        let other_quiet_index = moves
            .as_slice()
            .iter()
            .position(|&m| m == other_quiet)
            .expect("other_quiet is a legal move in this position");

        assert!(
            capture_index < killer_index,
            "the capture must still outrank the killer"
        );
        assert!(
            killer_index < other_quiet_index,
            "the killer must outrank an unrelated quiet move"
        );
    }

    /// Classifying a move by its index, which is how the per-move policy seam's
    /// callers ask whether a move is quiet, is sound only if each
    /// [`MovePriority`] occupies one unbroken run of the ordered list. That
    /// holds because [`MoveOrdering::move_priority`]'s tuple sorts on the tier first,
    /// so it is a property of the sort rather than of any position.
    ///
    /// Worth its own test because every way of breaking it is silent: a
    /// secondary sort criterion that crossed tiers, a reordered `MovePriority`
    /// variant, or a new tier whose membership is not a function of the tuple's
    /// first element would all leave the existing ordering tests green while
    /// making an index-derived classification lie.
    #[test]
    fn ordering_leaves_every_priority_tier_in_one_contiguous_run() {
        // Kiwipete, which offers captures across the whole value range at once,
        // alongside the start position and a lone-king endgame where almost
        // everything is quiet.
        let fens = [
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "4k3/8/8/3n4/4Q3/8/8/4K3 w - - 0 1",
        ];

        let mut richest = 0;
        for fen in fens {
            let board = Board::try_from_fen(fen).expect("valid FEN");
            let mut moves = legal_moves(&board);
            let quiets: Vec<Move> = moves
                .iter()
                .filter(|m| !m.flags().is_capture() && !m.flags().is_promotion())
                .copied()
                .collect();
            assert!(
                quiets.len() >= 3,
                "{fen}: need three quiets to seed a hash hint and two killers"
            );

            let mut ordering = MoveOrdering::new();
            ordering.set_hash_move(0, Some(quiets[0]));
            ordering.killers.record(0, quiets[1], false);
            ordering.killers.record(0, quiets[2], true);
            ordering.order(&board, &mut moves, 0);

            let mut runs: Vec<MovePriority> = Vec::new();
            for &m in &moves {
                let tier = ordering.move_priority(&board, m, 0).0;
                if runs.last() != Some(&tier) {
                    runs.push(tier);
                }
            }

            let mut distinct = runs.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(
                runs.len(),
                distinct.len(),
                "{fen}: a tier appears in more than one run: {runs:?}"
            );
            richest = richest.max(distinct.len());
        }

        assert!(
            richest >= 4,
            "no position produced enough tiers for the property to mean anything: {richest}"
        );
    }

    // ---- Move ordering's priority runs ----

    /// Positions paired with the exact order `order` puts them in, all
    /// four seeded identically by [`seeded_ordering`].
    ///
    /// The orders were captured from the implementation rather than derived by
    /// hand, so on their own they assert only that ordering has not changed.
    /// That is the point: a rewrite of how the sort carries its keys has to
    /// come out move-identical, and this catches a difference in the unit test
    /// suite rather than only in `tools/refactor-gate.sh`.
    ///
    /// Every position is legal, checked with `in_check` against the side not to
    /// move. `capture_and_quiet_position` is deliberately not among them: the
    /// side not to move is in check there, which makes it an illegal position
    /// whose move list contains a king capture.
    const ORDERING_FIXTURES: &[(&str, &str)] = &[
        (
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "a2a3 g2h3 d5e6 e2a6 b2b3 a2a4 f3f4 c3b1 c3d1 c3a4 c3b5 e5d3 e5c4 \
             e5g4 e5c6 e1d1 e1f1 d2c1 d2e3 d2f4 d2g5 d2h6 d5d6 g2g4 e2f1 e2d3 \
             e2c4 e2b5 a1b1 a1c1 a1d1 h1f1 h1g1 f3d3 f3e3 f3g3 g2g3 f3g4 f3f5 \
             f3h5 e1g1 e1c1 e2d1 e5d7 e5f7 e5g6 f3f6 f3h3",
        ),
        (
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "a2a3 b2b3 a2a4 b2b4 c2c3 c2c4 d2d3 d2d4 e2e3 e2e4 f2f3 f2f4 g2g3 \
             g2g4 h2h3 h2h4 b1a3 b1c3 g1f3 g1h3",
        ),
        (
            "3k4/8/8/3n4/4Q3/8/8/4K3 w - - 0 1",
            "e1d1 e1d2 e1f1 e1e2 e1f2 e4d3 e4b1 e4h1 e4c2 e4e2 e4g2 e4e3 e4f3 \
             e4a4 e4b4 e4c4 e4d4 e4f4 e4g4 e4h4 e4e5 e4f5 e4e6 e4g6 e4e7 e4h7 \
             e4e8 e4d5",
        ),
        (
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "e2e3 g2g3 e2e4 g2g4 a5a4 a5a6 b4b1 b4b2 b4b3 b4a4 b4c4 b4d4 b4e4 \
             b4f4",
        ),
    ];

    /// A `MoveOrdering` whose ply-0 hints are seeded from this position's own quiet
    /// moves, so a fixture's expected order is reproducible from the FEN alone
    /// rather than from a table of hand-picked hint moves.
    fn seeded_ordering(board: &Board) -> MoveOrdering<'static> {
        let quiets: Vec<Move> = legal_moves(board)
            .iter()
            .filter(|m| !m.flags().is_capture() && !m.flags().is_promotion())
            .copied()
            .collect();
        assert!(
            quiets.len() >= 3,
            "a fixture needs three quiet moves to seed a hash hint, a killer and a mate killer"
        );
        let mut ordering = MoveOrdering::new();
        ordering.set_hash_move(0, Some(quiets[0]));
        ordering.killers.record(0, quiets[1], false);
        ordering.killers.record(0, quiets[2], true);
        ordering
    }

    /// The fixture's position, its ordered moves, the runs over them, and the
    /// search that produced all three.
    fn order_fixture(fen: &str) -> (Board, MoveList, PriorityRuns, MoveOrdering<'static>) {
        let board = Board::try_from_fen(fen).expect("valid FEN");
        let mut ordering = seeded_ordering(&board);
        let mut moves = legal_moves(&board);
        let runs = ordering.order(&board, &mut moves, 0);
        (board, moves, runs, ordering)
    }

    fn rendered(moves: &MoveList) -> String {
        moves
            .iter()
            .map(|m| format!("{:?}{:?}", m.from(), m.to()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn every_ordering_fixture_is_a_legal_position() {
        for (fen, _) in ORDERING_FIXTURES {
            let board = Board::try_from_fen(fen).expect("valid FEN");
            assert!(
                !in_check(&board, board.side_to_move().flip()),
                "{fen}: the side not to move is in check, so this position cannot arise"
            );
        }
    }

    #[test]
    fn order_produces_the_order_it_always_has() {
        for (fen, expected) in ORDERING_FIXTURES {
            let (_, moves, _, _) = order_fixture(fen);
            let expected: Vec<&str> = expected.split_whitespace().collect();
            assert_eq!(
                rendered(&moves),
                expected.join(" "),
                "{fen}: the ordering changed"
            );
        }
    }

    #[test]
    fn the_runs_place_every_move_in_its_own_tier() {
        for (fen, _) in ORDERING_FIXTURES {
            let (board, moves, runs, ordering) = order_fixture(fen);
            for (i, &m) in moves.iter().enumerate() {
                let tier = ordering.move_priority(&board, m, 0).0;
                assert!(
                    runs.range(tier).contains(&i),
                    "{fen}: move {i} is {tier:?}, but the runs put that tier at {:?}",
                    runs.range(tier)
                );
            }
            let covered: usize = MovePriority::ALL
                .into_iter()
                .map(|t| runs.range(t).len())
                .sum();
            assert_eq!(
                covered,
                moves.len(),
                "{fen}: the runs must cover every move exactly once"
            );
        }
    }

    #[test]
    fn is_quiet_agrees_with_the_tier_move_priority_assigns() {
        for (fen, _) in ORDERING_FIXTURES {
            let (board, moves, runs, ordering) = order_fixture(fen);
            for (i, &m) in moves.iter().enumerate() {
                let tier = ordering.move_priority(&board, m, 0).0;
                assert_eq!(
                    runs.is_quiet(i),
                    tier == MovePriority::Quiet,
                    "{fen}: move {i} is {tier:?}"
                );
            }
        }
    }

    /// An absent tier has to answer as an empty range rather than as a missing
    /// one, because a caller indexes it unconditionally. The start position has
    /// no captures at all, so three tiers are absent at once.
    #[test]
    fn a_tier_no_move_belongs_to_is_an_empty_range() {
        let (_, _, runs, _) = order_fixture(ORDERING_FIXTURES[1].0);
        for tier in [
            MovePriority::WinningCapture,
            MovePriority::EqualCapture,
            MovePriority::LosingCapture,
        ] {
            assert!(
                runs.range(tier).is_empty(),
                "the start position offers no captures, so {tier:?} must be empty, got {:?}",
                runs.range(tier)
            );
        }
        assert!(
            !runs.range(MovePriority::Quiet).is_empty(),
            "the start position is almost all quiet moves"
        );
    }

    #[test]
    fn an_empty_move_list_leaves_every_run_empty() {
        let board = Board::try_from_fen(ORDERING_FIXTURES[1].0).expect("valid FEN");
        let mut ordering = MoveOrdering::new();
        let mut moves = MoveList::new();
        let runs = ordering.order(&board, &mut moves, 0);
        for tier in MovePriority::ALL {
            assert!(
                runs.range(tier).is_empty(),
                "no moves, so {tier:?} must be empty"
            );
        }
        assert!(
            !runs.is_quiet(0),
            "no move is quiet when there is no move at all"
        );
    }

    /// Ordering is about to grow a scratch buffer that outlives a single call.
    /// Anything left in it by one position must not reach the next, and the
    /// failure mode is a wrong classification rather than a crash, so it needs
    /// pinning before the buffer exists rather than after.
    #[test]
    fn ordering_one_position_does_not_leak_into_the_next() {
        let board = Board::try_from_fen(ORDERING_FIXTURES[3].0).expect("valid FEN");
        let mut alone = seeded_ordering(&board);
        let mut expected_moves = legal_moves(&board);
        let expected_runs = alone.order(&board, &mut expected_moves, 0);

        let other = Board::try_from_fen(ORDERING_FIXTURES[0].0).expect("valid FEN");
        let mut reused = seeded_ordering(&board);
        let mut other_moves = legal_moves(&other);
        let _ = reused.order(&other, &mut other_moves, 0);
        let mut moves = legal_moves(&board);
        let runs = reused.order(&board, &mut moves, 0);

        assert_eq!(
            rendered(&moves),
            rendered(&expected_moves),
            "ordering a larger position first changed the order of this one"
        );
        assert_eq!(
            runs, expected_runs,
            "ordering a larger position first changed this one's runs"
        );
    }
}
