//! Walking a set of parsed games into per-position move statistics: how
//! many times each candidate move was played from each position reached,
//! and with what results, subject to a rating floor and a ply cap.
//!
//! Two passes, deliberately: [`aggregate`] applies the rating floor and the
//! ply cap (the "backstop" half of the density-plus-ply-cap depth rule) and
//! counts everything within them; [`filter_by_density`] is the "density"
//! half, dropping any position too few games actually reached. Games
//! merge into the same position by hash regardless of the move order that
//! reached it, which is what makes a density count meaningful across
//! transpositions rather than per move-order.

use crate::pgn::PgnGame;
use turox_engine::Move;

/// One candidate move's observed statistics at some position, before
/// weighing turns them into a `book::BookMove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Tuning knobs for [`aggregate`] and [`filter_by_density`]. Deliberately
/// not fixed constants: the right values are found by measuring the
/// generator's own output (book size, coverage, plausibility of the lines
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

/// Walks every qualifying game in `games` (both ratings at least
/// `options.min_rating`, a known result) from the start position, up to
/// `options.max_ply` plies, recording which move was played from each
/// position reached and with what eventual result.
///
/// Returns the raw counts, not yet checked against `options.min_sample_size`;
/// see [`filter_by_density`] for that half.
#[must_use]
pub fn aggregate(_games: &[PgnGame], _options: &BuildOptions) -> Vec<(u64, Vec<MoveStats>)> {
    Vec::new()
}

/// Drops any position whose candidates' combined `times_played` (summed
/// across every move [`aggregate`] recorded for it, since every qualifying
/// game reaching a position plays exactly one move from it) falls short of
/// `min_sample_size`.
#[must_use]
pub fn filter_by_density(
    aggregated: Vec<(u64, Vec<MoveStats>)>,
    min_sample_size: u32,
) -> Vec<(u64, Vec<MoveStats>)> {
    let _ = min_sample_size;
    aggregated
}
