//! Walking a set of parsed games into per-position move statistics.
//!
//! How many times each candidate move was played from each position
//! reached, and with what results, subject to a rating floor and a ply cap.
//!
//! Two passes, deliberately: [`aggregate`] applies the rating floor and the
//! ply cap (the "backstop" half of the density-plus-ply-cap depth rule) and
//! counts everything within them; [`filter_by_density`] is the "density"
//! half, dropping any position too few games actually reached. Games
//! merge into the same position by hash regardless of the move order that
//! reached it, which is what makes a density count meaningful across
//! transpositions rather than per move-order.

use std::collections::HashMap;

use turox_chess::board::Board;
use turox_chess::{Color, Move};
use turox_notation::pgn::{GameResult, PgnGame};
use turox_notation::san::resolve_san;

/// One candidate move's observed statistics at some position, before
/// weighing turns them into a `book::BookMove`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveStats {
    /// The move itself.
    pub mv: Move,
    /// How many qualifying games played this move from this position.
    pub times_played: u32,
    /// Of those, how many were eventually won by whoever played it.
    pub wins: u32,
    /// Of those, how many were eventually drawn.
    pub draws: u32,
    /// Of those, how many were eventually lost by whoever played it.
    pub losses: u32,
    /// The opening name of the first qualifying game recorded to play this
    /// move from this position, or `None` if that game's own
    /// `PgnGame::opening_name` was itself `None`. First-seen, not
    /// necessarily the most common name among every game that played this
    /// move: still names the move, just not guaranteed to pick the
    /// majority's name when sources disagree.
    pub opening_name: Option<String>,
}

/// Tuning knobs for [`aggregate`] and [`filter_by_density`].
///
/// Deliberately not fixed constants: the right values are found by measuring
/// the generator's own output (book size, coverage, plausibility of the lines
/// it keeps), not decided in the abstract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildOptions {
    /// Both players must meet this rating for a game to count at all. A
    /// game missing either player's rating is excluded, not assumed to
    /// pass.
    pub min_rating: u32,
    /// A position survives [`filter_by_density`] only if at least this
    /// many qualifying games reached it.
    pub min_sample_size: u32,
    /// The hard ply cap: at most `max_ply` positions are recorded from any
    /// one game (the position before its 1st move through the position
    /// before its `max_ply`-th move), regardless of how well-attested a
    /// longer line is.
    pub max_ply: u32,
}

/// Walks every qualifying game in `games` from the start position.
///
/// Qualifying means both ratings at least `options.min_rating` and a known
/// result; the walk runs up to `options.max_ply` plies, recording which move
/// was played from each position reached and with what eventual result.
///
/// Takes an owned-item iterator, not a slice: `games` is consumed one game
/// at a time (each dropped once its moves are walked) rather than held as
/// a pre-collected list, so a caller feeding this from `PgnReader` never
/// holds more than one game's worth of parsed data alongside the running
/// accumulator, regardless of how large the underlying source is.
///
/// Returns the raw counts, not yet checked against `options.min_sample_size`;
/// see [`filter_by_density`] for that half.
#[must_use]
pub fn aggregate(
    games: impl IntoIterator<Item = PgnGame>,
    options: &BuildOptions,
) -> Vec<(u64, Vec<MoveStats>)> {
    let mut results: HashMap<u64, Vec<MoveStats>> = HashMap::new();

    for game in games.into_iter().filter(|g| {
        g.result != GameResult::Unknown
            && g.white_elo.is_some_and(|elo| elo >= options.min_rating)
            && g.black_elo.is_some_and(|elo| elo >= options.min_rating)
    }) {
        let mut board = Board::start_pos();

        for (ply, san) in game.moves.iter().enumerate() {
            let ply = u32::try_from(ply).unwrap_or(u32::MAX);
            if ply >= options.max_ply {
                break;
            }
            let Some(mv) = resolve_san(&board, san) else {
                break;
            };

            let (win_delta, draw_delta, loss_delta) =
                match (board.side_to_move(), game.result) {
                    // Grouped by whether the side to move is the side that won,
                    // which is the only thing being asked. Spelling the four
                    // combinations out separately invited reading them as four
                    // independent facts rather than one.
                    (Color::White, GameResult::WhiteWins)
                    | (Color::Black, GameResult::BlackWins) => (1, 0, 0),
                    (Color::Black, GameResult::WhiteWins)
                    | (Color::White, GameResult::BlackWins) => (0, 0, 1),
                    (_, GameResult::Draw) => (0, 1, 0),
                    (_, GameResult::Unknown) => (0, 0, 0), // excluded above; never reached
                };

            let entry = results.entry(board.hash()).or_default();
            if let Some(existing) = entry.iter_mut().find(|ms| ms.mv == mv) {
                existing.times_played += 1;
                existing.wins += win_delta;
                existing.draws += draw_delta;
                existing.losses += loss_delta;
            } else {
                entry.push(MoveStats {
                    mv,
                    times_played: 1,
                    wins: win_delta,
                    draws: draw_delta,
                    losses: loss_delta,
                    opening_name: game.opening_name.clone(),
                });
            }

            board = board.make_move(mv);
        }
    }

    results.into_iter().collect()
}

/// Drops any position whose candidates' combined `times_played` falls short
/// of `min_sample_size`.
///
/// Combined means summed across every move [`aggregate`] recorded for it,
/// since every qualifying game reaching a position plays exactly one move
/// from it.
#[must_use]
pub fn filter_by_density(
    aggregated: Vec<(u64, Vec<MoveStats>)>,
    min_sample_size: u32,
) -> Vec<(u64, Vec<MoveStats>)> {
    aggregated
        .into_iter()
        .filter(|(_, stats)| {
            let total: u32 = stats.iter().map(|s| s.times_played).sum();
            total >= min_sample_size
        })
        .collect()
}
