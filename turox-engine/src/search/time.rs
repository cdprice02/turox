//! Time allocation for a real game clock: how long to budget for the
//! current move given the clock state UCI's `go` command reports
//! (`wtime`/`btime`/`winc`/`binc`/`movestogo`). Pure and stateless; the
//! caller combines the result with `Instant::now()` and hands it to
//! [`super::Search::with_deadline`].

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

/// The floor on a single move's budget, so search always gets *some* time
/// to complete at least a shallow iteration and return a legal move even
/// with very little left on the clock. Small enough to matter only when
/// `time_left` itself is already tiny; `allocate_time`'s final clamp still
/// keeps the result at or under `time_left`, so this floor can't budget
/// more time than actually exists.
const MIN_BUDGET: Duration = Duration::from_millis(30);

/// Budgets a [`Duration`] for the current move from the time left on the
/// clock (`time_left`), the per-move increment (`increment`), and how many
/// moves remain until the next time control (`moves_to_go`, `None` if UCI
/// didn't say).
///
/// The raw estimate is `time_left / moves_to_go + increment`
/// (`DEFAULT_MOVES_TO_GO` standing in when `moves_to_go` is unknown), then
/// clamped three ways in order:
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
}
