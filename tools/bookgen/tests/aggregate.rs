//! Concrete tests for `aggregate::{aggregate, filter_by_density}`.

use bookgen::aggregate::{aggregate, filter_by_density, BuildOptions, MoveStats};
use bookgen::pgn::{GameResult, PgnGame};
use turox_engine::board::Board;
use turox_engine::{Move, MoveFlags, Square};

fn game(white_elo: u32, black_elo: u32, result: GameResult, moves: &[&str]) -> PgnGame {
    PgnGame {
        white_elo: Some(white_elo),
        black_elo: Some(black_elo),
        result,
        moves: moves.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn e4() -> Move {
    Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush)
}

fn d4() -> Move {
    Move::new(Square::D2, Square::D4, MoveFlags::DoublePawnPush)
}

fn c4() -> Move {
    Move::new(Square::C2, Square::C4, MoveFlags::DoublePawnPush)
}

fn nf3() -> Move {
    Move::new(Square::G1, Square::F3, MoveFlags::Quiet)
}

fn e5() -> Move {
    Move::new(Square::E7, Square::E5, MoveFlags::DoublePawnPush)
}

fn nc6() -> Move {
    Move::new(Square::B8, Square::C6, MoveFlags::Quiet)
}

fn bb5() -> Move {
    Move::new(Square::F1, Square::B5, MoveFlags::Quiet)
}

const LOOSE_OPTIONS: BuildOptions = BuildOptions {
    min_rating: 2300,
    min_sample_size: 1,
    max_ply: 10,
};

#[test]
fn a_game_with_either_player_below_min_rating_is_excluded() {
    let strong = game(2400, 2400, GameResult::WhiteWins, &["e4"]);
    let weak = game(1200, 2400, GameResult::WhiteWins, &["d4"]);

    let result = aggregate(&[strong, weak], &LOOSE_OPTIONS);

    let start_hash = Board::start_pos().hash();
    let (_, stats) = result
        .iter()
        .find(|(h, _)| *h == start_hash)
        .expect("startpos must be recorded from the strong game");
    assert_eq!(
        stats.len(),
        1,
        "only the strong game's move should be recorded: {stats:?}"
    );
    assert_eq!(stats[0].mv, e4());
}

#[test]
fn a_game_missing_either_elo_tag_is_excluded() {
    let mut incomplete = game(2400, 2400, GameResult::WhiteWins, &["e4"]);
    incomplete.black_elo = None;

    let result = aggregate(&[incomplete], &LOOSE_OPTIONS);
    assert!(
        result.is_empty(),
        "a missing rating must exclude the game entirely: {result:?}"
    );
}

#[test]
fn max_ply_stops_recording_positions_past_the_cap() {
    let g = game(2400, 2400, GameResult::WhiteWins, &["e4", "e5", "Nf3"]);
    let options = BuildOptions {
        max_ply: 2,
        ..LOOSE_OPTIONS
    };

    let result = aggregate(&[g], &options);

    let start_hash = Board::start_pos().hash();
    let after_e4 = Board::start_pos().make_move(e4());
    let after_e4_hash = after_e4.hash();
    let after_e5_hash = after_e4.make_move(e5()).hash();

    assert!(
        result.iter().any(|(h, _)| *h == start_hash),
        "ply 0 (before e4, the game's 1st move) must be recorded: {result:?}"
    );
    assert!(
        result.iter().any(|(h, _)| *h == after_e4_hash),
        "ply 1 (before e5, the game's 2nd move) must be recorded: {result:?}"
    );
    assert!(
        !result.iter().any(|(h, _)| *h == after_e5_hash),
        "ply 2 (before Nf3, the game's 3rd move) is past a cap of 2 and must not be recorded: {result:?}"
    );
}

#[test]
fn the_same_move_from_two_games_merges_into_one_candidate_with_summed_counts() {
    let g1 = game(2400, 2400, GameResult::WhiteWins, &["e4"]);
    let g2 = game(2400, 2400, GameResult::WhiteWins, &["e4"]);

    let result = aggregate(&[g1, g2], &LOOSE_OPTIONS);

    let start_hash = Board::start_pos().hash();
    let (_, stats) = result
        .iter()
        .find(|(h, _)| *h == start_hash)
        .expect("startpos recorded");
    assert_eq!(
        stats.len(),
        1,
        "the same move from two games must merge into one candidate: {stats:?}"
    );
    assert_eq!(
        stats[0].times_played, 2,
        "counts must sum across games: {stats:?}"
    );
}

#[test]
fn games_reaching_the_same_position_by_different_move_orders_merge() {
    // 1.e4 e5 2.Nf3 Nc6 and 1.Nf3 e5 2.e4 Nc6 reach the same placement,
    // side to move, and castling rights after 4 plies, but NOT after 3:
    // whichever order plays e4 last still has its one-ply en passant
    // availability live, while the other order's already expired. So the
    // shared hash to check is after 4 plies (once Nc6 is common to both),
    // with a 5th, also-shared move as the candidate that should merge.
    let via_e4_first = game(
        2400,
        2400,
        GameResult::WhiteWins,
        &["e4", "e5", "Nf3", "Nc6", "Bb5"],
    );
    let via_nf3_first = game(
        2400,
        2400,
        GameResult::WhiteWins,
        &["Nf3", "e5", "e4", "Nc6", "Bb5"],
    );

    let result = aggregate(&[via_e4_first, via_nf3_first], &LOOSE_OPTIONS);

    let transposed_hash = Board::start_pos()
        .make_move(e4())
        .make_move(e5())
        .make_move(nf3())
        .make_move(nc6())
        .hash();
    let (_, stats) = result
        .iter()
        .find(|(h, _)| *h == transposed_hash)
        .expect("the transposed position must be recorded once, not once per move order");
    assert_eq!(
        stats.len(),
        1,
        "both games play the same 5th move: {stats:?}"
    );
    assert_eq!(stats[0].mv, bb5());
    assert_eq!(
        stats[0].times_played, 2,
        "a transposition must merge counts across move orders: {stats:?}"
    );
}

#[test]
fn wins_draws_and_losses_are_tallied_relative_to_whoever_moved() {
    let white_wins = game(2400, 2400, GameResult::WhiteWins, &["e4"]);
    let white_loses = game(2400, 2400, GameResult::BlackWins, &["d4"]);
    let drawn = game(2400, 2400, GameResult::Draw, &["c4"]);

    let result = aggregate(&[white_wins, white_loses, drawn], &LOOSE_OPTIONS);
    let start_hash = Board::start_pos().hash();
    let (_, stats) = result
        .iter()
        .find(|(h, _)| *h == start_hash)
        .expect("startpos recorded");

    let find = |mv: Move| {
        stats
            .iter()
            .find(|s| s.mv == mv)
            .unwrap_or_else(|| panic!("{mv:?} not recorded in {stats:?}"))
    };

    let e4_stats = find(e4());
    assert_eq!((e4_stats.wins, e4_stats.draws, e4_stats.losses), (1, 0, 0));

    let d4_stats = find(d4());
    assert_eq!((d4_stats.wins, d4_stats.draws, d4_stats.losses), (0, 0, 1));

    let c4_stats = find(c4());
    assert_eq!((c4_stats.wins, c4_stats.draws, c4_stats.losses), (0, 1, 0));
}

fn dummy_stats(mv: Move, times_played: u32) -> MoveStats {
    MoveStats {
        mv,
        times_played,
        wins: times_played,
        draws: 0,
        losses: 0,
    }
}

#[test]
fn filter_by_density_drops_positions_below_the_sample_threshold() {
    let below_threshold = vec![dummy_stats(e4(), 3)];
    let at_threshold = vec![dummy_stats(d4(), 5)];
    let aggregated = vec![(111u64, below_threshold), (222u64, at_threshold.clone())];

    let kept = filter_by_density(aggregated, 5);

    assert_eq!(
        kept,
        vec![(222u64, at_threshold)],
        "a position with fewer than min_sample_size games must be dropped, one at the threshold must survive"
    );
}

#[test]
fn filter_by_density_sums_across_every_candidate_at_a_position() {
    // 2 + 2 = 4 total games reached this position, split across two
    // different replies; neither reply alone clears a threshold of 4, but
    // the position as a whole does.
    let stats = vec![dummy_stats(e4(), 2), dummy_stats(d4(), 2)];
    let aggregated = vec![(111u64, stats.clone())];

    let kept = filter_by_density(aggregated, 4);

    assert_eq!(
        kept,
        vec![(111u64, stats)],
        "density is the position's total across all candidates, not any one candidate alone"
    );
}
