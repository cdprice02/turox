//! Concrete tests for `weight::weigh`.

use bookgen::aggregate::MoveStats;
use bookgen::weight::weigh;
use turox_engine::{Move, MoveFlags, Square};

fn stats(times_played: u32, wins: u32, draws: u32, losses: u32) -> MoveStats {
    MoveStats {
        mv: Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush),
        times_played,
        wins,
        draws,
        losses,
    }
}

#[test]
fn every_played_move_gets_a_positive_weight() {
    let weighed = weigh(&[stats(1, 1, 0, 0)]);
    assert!(
        weighed[0].weight > 0,
        "a move with at least one recorded game must never weigh zero: {weighed:?}"
    );
}

#[test]
fn more_games_at_the_same_win_rate_weighs_strictly_more() {
    let fewer = weigh(&[stats(10, 5, 0, 5)])[0].weight; // 50% win rate, 10 games
    let more = weigh(&[stats(20, 10, 0, 10)])[0].weight; // 50% win rate, 20 games
    assert!(
        more > fewer,
        "more games at the same win rate must weigh more: {fewer} vs {more}"
    );
}

#[test]
fn a_better_win_rate_at_the_same_game_count_weighs_strictly_more() {
    let worse = weigh(&[stats(10, 2, 0, 8)])[0].weight; // 20% win rate
    let better = weigh(&[stats(10, 8, 0, 2)])[0].weight; // 80% win rate
    assert!(
        better > worse,
        "a better record at the same sample size must weigh more: {worse} vs {better}"
    );
}

#[test]
fn a_draw_counts_as_a_half_win_not_a_full_loss() {
    // Same 10 games either way: one side is all draws, the other is half
    // wins/half losses. Both average to a 50% "score", so they should
    // weigh the same; a formula that ignored draws entirely, or treated
    // them as losses, would weigh these differently.
    let all_draws = weigh(&[stats(10, 0, 10, 0)])[0].weight;
    let half_and_half = weigh(&[stats(10, 5, 0, 5)])[0].weight;
    assert_eq!(
        all_draws, half_and_half,
        "both average a 50% score across 10 games and must weigh the same"
    );
}
