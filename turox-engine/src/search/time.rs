//! Time allocation for a real game clock.
//!
//! How long to budget for the current move given the clock state UCI's `go` command
//! reports (`wtime`/`btime`/`winc`/`binc`/`movestogo`). Pure and stateless; the caller
//! combines the result with `Instant::now()` and hands it to
//! [`super::Search::with_deadline`].
//!
//! [`should_skip_next_iteration`] is the other half: once a move's budget is
//! spent iteration by iteration, whether to start one more.

use std::time::Duration;

/// Assumed moves remaining when UCI doesn't say (`moves_to_go` is `None`,
/// or `Some(0)`, which some GUIs send to mean the same "unknown" rather
/// than literally zero). Mainly matters for sudden-death controls, where
/// no more time ever gets added: 30 is a plain middle-of-the-road guess
/// with no game-phase information to do better with (this module doesn't
/// see the move number), low enough not to starve an early move, high
/// enough not to burn through the clock assuming a much shorter game than
/// actually happens.
const DEFAULT_MOVES_TO_GO: u32 = 30;

/// The floor on a single move's budget, so search always gets *some* time to complete at
/// least a shallow iteration and return a legal move even with very little left on the
/// clock.
///
/// Small enough to matter only when `time_left` itself is already tiny; `allocate_time`'s
/// final clamp still keeps the result at or under `time_left`, so this floor can't budget
/// more time than actually exists.
const MIN_BUDGET: Duration = Duration::from_millis(30);

/// Reserve subtracted from the computed budget to cover the gap between search
/// finishing and the move actually reaching the clock: process scheduling, UCI I/O,
/// and (for lichess-bot specifically) network round-trip time. That overhead is
/// roughly constant no matter the time control, since it comes from the dispatch
/// path rather than from anything proportional to `time_left` or `increment`, so
/// it's a fixed constant here rather than scaled the way the rest of the formula
/// is. 100ms comfortably clears an 87ms overrun observed in real self-play testing,
/// with margin left for jitter. Treat it as a tunable starting point; real tuning
/// belongs to the self-play SPRT harness, not further reasoning in this comment.
const OVERHEAD_RESERVE: Duration = Duration::from_millis(100);

/// Budgets a [`Duration`] for the current move.
///
/// From the time left on the clock (`time_left`), the per-move increment (`increment`),
/// and how many moves remain until the next time control (`moves_to_go`, `None` if UCI
/// didn't say). The raw estimate is `time_left / moves_to_go + increment`
/// (`DEFAULT_MOVES_TO_GO` standing in when `moves_to_go` is unknown), then clamped four
/// ways in order:
/// 1. Capped at half of `time_left`: a low or defaulted `moves_to_go` (most
///    of all, `Some(1)`) could otherwise budget the *entire* remaining
///    clock for one move, and GUI/network overhead or an optimistic
///    `moves_to_go` guess eating into that margin would flag the whole
///    game rather than just cost one weaker move.
/// 2. Floored at `MIN_BUDGET`, so search is never handed `Duration::ZERO`
///    and has no time to even return a legal move.
/// 3. Re-capped at `time_left` itself, since step 2's floor can push the
///    result back above `time_left` when the clock is already down to only
///    a few milliseconds; there's never more time to give than what's
///    actually left.
/// 4. `OVERHEAD_RESERVE` subtracted (saturating, so it can't underflow past
///    zero), then re-floored at `MIN_BUDGET` and re-capped at `time_left`.
///    The floor wins over the reserve on purpose: under extreme time
///    pressure, guaranteeing search gets *some* time outweighs guaranteeing
///    the reserve, since a zero budget is a guaranteed forfeit. The cap
///    is reapplied for the same reason step 3 exists: `MIN_BUDGET` can
///    again exceed `time_left` once `time_left` itself is tiny.
///
///    This (and step 2-3's identical pattern above) is `.max(...).min(...)`
///    rather than a single `.clamp(MIN_BUDGET, time_left)`: `Ord::clamp`
///    panics when its lower bound exceeds its upper bound, and
///    `MIN_BUDGET > time_left` is exactly the tiny-clock case these steps
///    exist to handle gracefully, not reject.
#[must_use]
pub fn allocate_time(
    time_left: Duration,
    increment: Duration,
    moves_to_go: Option<u32>,
) -> Duration {
    let moves_to_go = match moves_to_go {
        None | Some(0) => DEFAULT_MOVES_TO_GO,
        Some(value) => value,
    };
    (time_left / moves_to_go + increment)
        .min(time_left / 2)
        .max(MIN_BUDGET)
        .min(time_left)
        .saturating_sub(OVERHEAD_RESERVE)
        .max(MIN_BUDGET)
        .min(time_left)
}

/// The safety-margin multiplier applied to the last completed iteration's
/// own elapsed time when there is not yet a node-count ratio to estimate
/// the next iteration's cost from: only one iteration has completed so far,
/// so there is nothing to measure a growth rate from.
///
/// `2`, not the wider margins real branching factors near the horizon can
/// reach: this is the one guarded decision (whether to start iteration 2)
/// that always runs blind, on every search, regardless of how well move
/// ordering is doing right now. A tight margin here costs at most one
/// iteration's worth of conservatism before real per-search node counts
/// take over; a loose one throws away reachable depth on every single move.
const FALLBACK_SAFETY_MARGIN: u32 = 2;

/// Whether iterative deepening should skip starting its next iteration.
///
/// Takes the last completed iteration's own elapsed time and node count,
/// the iteration before that one's own node count (`None` if fewer than two
/// iterations have completed yet), and how much time actually remains
/// before the deadline.
///
/// With no node-count ratio yet to estimate growth from, this falls back to
/// [`FALLBACK_SAFETY_MARGIN`] against `elapsed_last` alone. Once two
/// iterations have completed, the next iteration's cost is estimated from
/// this search's own measured growth (`nodes_last` over
/// `nodes_before_last`), tracking whatever this search's move ordering is
/// actually doing right now rather than a hand-picked constant that goes
/// stale the moment ordering changes.
#[must_use]
pub fn should_skip_next_iteration(
    elapsed_last: Duration,
    nodes_last: u64,
    nodes_before_last: Option<u64>,
    remaining: Duration,
) -> bool {
    let estimated_cost = match nodes_before_last {
        Some(before) if before > 0 => {
            #[expect(
                clippy::as_conversions,
                clippy::cast_precision_loss,
                reason = "node counts as a growth ratio: `f64` has no infallible `From<u64>` since a count past 2^52 would lose precision, but a real search is nowhere near that, and a ratio of two node counts is an estimate already, not a value this cast could meaningfully corrupt"
            )]
            let ratio = nodes_last as f64 / before as f64;
            elapsed_last.mul_f64(ratio)
        }
        _ => elapsed_last.saturating_mul(FALLBACK_SAFETY_MARGIN),
    };
    estimated_cost > remaining
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_never_zero() {
        let budget = allocate_time(Duration::from_millis(500), Duration::ZERO, Some(40));
        assert!(budget > Duration::ZERO, "budget: {budget:?}");
    }

    #[test]
    fn budget_never_reaches_the_full_time_left() {
        // `moves_to_go: Some(1)` is the case most likely to tempt a naive
        // formula into spending everything on one move.
        let time_left = Duration::from_secs(60);
        let budget = allocate_time(time_left, Duration::ZERO, Some(1));
        assert!(
            budget < time_left,
            "budget {budget:?} must leave a safety margin under time_left {time_left:?}"
        );
    }

    #[test]
    fn zero_moves_to_go_does_not_panic_or_divide_by_zero() {
        let budget = allocate_time(Duration::from_secs(30), Duration::ZERO, Some(0));
        assert!(budget > Duration::ZERO);
        assert!(budget <= Duration::from_secs(30));
    }

    #[test]
    fn unknown_moves_to_go_falls_back_to_a_sane_default() {
        let budget = allocate_time(Duration::from_secs(300), Duration::ZERO, None);
        assert!(budget > Duration::ZERO);
        assert!(budget < Duration::from_secs(300));
    }

    #[test]
    fn fewer_moves_to_go_means_more_time_per_move() {
        let time_left = Duration::from_secs(120);
        let with_many_moves_left = allocate_time(time_left, Duration::ZERO, Some(40));
        let with_few_moves_left = allocate_time(time_left, Duration::ZERO, Some(4));
        assert!(
            with_few_moves_left > with_many_moves_left,
            "fewer moves to go ({with_few_moves_left:?}) should budget more per move than many moves to go ({with_many_moves_left:?})"
        );
    }

    #[test]
    fn a_larger_increment_never_decreases_the_budget() {
        let time_left = Duration::from_secs(60);
        let moves_to_go = Some(30);
        let no_increment = allocate_time(time_left, Duration::ZERO, moves_to_go);
        let with_increment = allocate_time(time_left, Duration::from_secs(2), moves_to_go);
        assert!(with_increment >= no_increment);
    }

    #[test]
    fn a_tiny_time_left_still_returns_a_bounded_nonzero_budget() {
        // A fast time control near flagging: the budget must still respect
        // time_left as a hard ceiling, not just a rough target.
        let time_left = Duration::from_millis(50);
        let budget = allocate_time(time_left, Duration::ZERO, Some(20));
        assert!(budget > Duration::ZERO);
        assert!(budget <= time_left);
    }

    #[test]
    fn overhead_reserve_is_subtracted_from_the_clamped_budget() {
        // time_left generous enough that the pre-reserve three-step clamp
        // lands well above MIN_BUDGET + OVERHEAD_RESERVE, so the reserve's
        // effect is isolated rather than masked by the MIN_BUDGET floor.
        let time_left = Duration::from_secs(60);
        let increment = Duration::ZERO;
        let moves_to_go: u32 = 30;
        let pre_reserve_budget = (time_left / moves_to_go + increment)
            .min(time_left / 2)
            .max(MIN_BUDGET)
            .min(time_left);
        let budget = allocate_time(time_left, increment, Some(moves_to_go));
        assert_eq!(
            budget,
            pre_reserve_budget.saturating_sub(OVERHEAD_RESERVE),
            "budget {budget:?} should be exactly OVERHEAD_RESERVE less than the pre-reserve clamp {pre_reserve_budget:?}"
        );
    }

    #[test]
    fn min_budget_still_wins_when_the_reserve_would_push_below_it() {
        // time_left small enough that the pre-reserve clamp already sits at
        // MIN_BUDGET; subtracting OVERHEAD_RESERVE from it would go negative
        // if not for the saturating subtract and the re-floor after it.
        let time_left = Duration::from_millis(40);
        let budget = allocate_time(time_left, Duration::ZERO, Some(20));
        assert!(
            budget >= MIN_BUDGET,
            "budget {budget:?} must never drop below MIN_BUDGET, even with the reserve applied"
        );
        assert!(budget > Duration::ZERO, "budget: {budget:?}");
    }

    #[test]
    fn fallback_margin_skips_when_no_ratio_data_and_double_elapsed_exceeds_remaining() {
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(150);
        assert!(
            should_skip_next_iteration(elapsed, 1000, None, remaining),
            "with no ratio data, 2x elapsed ({:?}) exceeds remaining {remaining:?}, so the next iteration must be skipped",
            elapsed.saturating_mul(FALLBACK_SAFETY_MARGIN)
        );
    }

    #[test]
    fn fallback_margin_does_not_skip_when_remaining_covers_double_elapsed() {
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(250);
        assert!(
            !should_skip_next_iteration(elapsed, 1000, None, remaining),
            "remaining {remaining:?} comfortably covers 2x elapsed {elapsed:?}, so the next iteration must not be skipped"
        );
    }

    #[test]
    fn a_zero_previous_node_count_falls_back_to_the_margin_rather_than_dividing_by_zero() {
        // `Some(0)` is a degenerate case (an iteration that searched zero
        // nodes), not the "no data yet" `None`, but a real ratio against it
        // is meaningless; this must behave exactly like `None` rather than
        // divide by zero.
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(150);
        assert!(should_skip_next_iteration(
            elapsed,
            1000,
            Some(0),
            remaining
        ));
    }

    #[test]
    fn ratio_estimate_skips_when_measured_growth_exceeds_remaining() {
        // A measured 9x growth (1_000 -> 9_000 nodes) projects a 900ms next
        // iteration, which does not fit in the 500ms left.
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(500);
        assert!(should_skip_next_iteration(
            elapsed,
            9_000,
            Some(1_000),
            remaining
        ));
    }

    #[test]
    fn ratio_estimate_does_not_skip_when_measured_growth_fits_remaining() {
        // A measured 1.2x growth (1_000 -> 1_200 nodes) projects a 120ms
        // next iteration, which comfortably fits in the 200ms left.
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(200);
        assert!(!should_skip_next_iteration(
            elapsed,
            1_200,
            Some(1_000),
            remaining
        ));
    }

    /// The point of measuring a real ratio at all: a search whose own move
    /// ordering is producing much less growth than the blind fallback
    /// margin assumes must not be penalized for it. `FALLBACK_SAFETY_MARGIN`
    /// alone would skip here (2x elapsed is 200ms, past the 180ms left);
    /// the measured 1.5x growth (120ms) must let it through instead.
    #[test]
    fn ratio_estimate_permits_an_iteration_the_fallback_margin_would_have_skipped() {
        let elapsed = Duration::from_millis(100);
        let remaining = Duration::from_millis(180);

        assert!(
            elapsed.saturating_mul(FALLBACK_SAFETY_MARGIN) > remaining,
            "test setup: the fallback margin must actually be the tighter bound here"
        );
        assert!(!should_skip_next_iteration(
            elapsed,
            1_500,
            Some(1_000),
            remaining
        ));
    }

    #[test]
    fn an_estimated_cost_exactly_equal_to_remaining_does_not_skip() {
        // Matches the fail-soft convention used everywhere else a cutoff or
        // a limit is checked in this crate: strictly greater trips the
        // limit, equal does not.
        let elapsed = Duration::from_millis(100);
        let remaining = elapsed.saturating_mul(FALLBACK_SAFETY_MARGIN);
        assert!(!should_skip_next_iteration(elapsed, 1000, None, remaining));
    }

    #[test]
    fn zero_elapsed_never_skips() {
        assert!(!should_skip_next_iteration(
            Duration::ZERO,
            1_000,
            Some(1_000),
            Duration::ZERO
        ));
    }
}
