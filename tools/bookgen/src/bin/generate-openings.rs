//! Regenerates `tools/selfplay/openings.epd` from the lichess-org/chess-
//! openings data set.
//!
//! The suite exists because an SPRT match runs with `Book=false` by
//! default (per `tools/selfplay/sprt.sh`'s own `--book` flag), to isolate
//! the search under test from the opening book's own move choices: with
//! no book steering play and a fixed seed pinning root-move-order tie
//! breaking, a match seeded from `startpos` alone would replay one game
//! for as long as it ran and report a confident-looking verdict resting
//! on a single sample. Every round needs its own start position instead.
//!
//! A second binary in `bookgen` rather than a new crate: reuses
//! `pgn::parse_movetext` and `san::resolve_san` directly instead of a
//! second, independent SAN implementation, and this tool's `pgn`/`san`
//! modules are exactly what a "replay a SAN line into a position" job
//! needs. Replaces `tools/selfplay/generate-openings.py`, whose
//! `python-chess` dependency was the only non-stdlib dependency anywhere
//! under `tools/`.
//!
//! Source data: <https://github.com/lichess-org/chess-openings>, released
//! under CC0 1.0 (public domain dedication). Pinned to one commit below,
//! so a regeneration reproduces the checked-in suite rather than drifting
//! with upstream.

use bookgen::openings::{
    fetch, parse_tsv, replay, OpeningRow, ECO_VOLUMES, FETCH_TIMEOUT, UPSTREAM_COMMIT, UPSTREAM_URL,
};
use clap::Parser;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use turox_chess::board::Board;
use turox_chess::move_gen::legal::legal_moves;
use turox_chess::{Color, Piece};
use turox_engine::eval::weights::PIECE_VALUES;
use turox_notation::pgn::parse_movetext;

/// Ply bounds on a line to be usable as a start position. Below the lower
/// bound the positions are too generic to spread games out (there are
/// only twenty legal first moves, and a suite that keeps repeating `1.
/// e4` against a deterministic-given-a-budget engine repeats games).
/// Above the upper bound the book is doing more of the playing than the
/// engine is, and the deeper named lines skew toward sharp theory a
/// sub-1500 engine cannot handle sensibly.
const MIN_PLIES: usize = 4;
const MAX_PLIES: usize = 12;

/// How far one side's material can lead by before a line is dropped.
/// Named opening theory includes plenty of lines objectively lost for
/// someone; a pawn gambit is fine (matches are played with both colors
/// from every position, so the imbalance cancels), a piece is not.
/// Reuses `turox_engine`'s own material weights rather than a second,
/// independently maintained table: this tool is judging the same
/// material balance the engine itself evaluates.
const MAX_MATERIAL_IMBALANCE: i32 = 100;

/// A start position kept for the suite: its FEN, and the ECO/name pair
/// `openings.epd`'s EPD comment attributes it to.
struct Opening {
    fen: String,
    eco: String,
    name: String,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Where to write the suite. Defaults to `openings.epd` next to
    /// `tools/selfplay/sprt.sh`, which is where it expects to find it.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let output = args.output.unwrap_or_else(default_output_path);

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .into();

    let mut rows = Vec::new();
    for volume in ECO_VOLUMES {
        // `{commit}` and `{volume}` are placeholders in `UPSTREAM_URL`'s own
        // template, substituted here rather than format arguments; they are
        // spelled this way because that is what the upstream URL contains.
        #[expect(
            clippy::literal_string_with_formatting_args,
            reason = "placeholders in a URL template, not a format string"
        )]
        let url = UPSTREAM_URL
            .replace("{commit}", UPSTREAM_COMMIT)
            .replace("{volume}", &volume.to_string());
        match fetch(&agent, &url) {
            Ok(text) => rows.extend(parse_tsv(&text)),
            Err(err) => {
                eprintln!("generate-openings: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    let suite = build_suite(rows);

    let mut out = String::new();
    for opening in &suite {
        // `fastchess -openings format=epd` hands the whole line to its FEN
        // parser, so the comment has to be a legal EPD operation rather
        // than a trailing bare string.
        //
        // `write!` into the string rather than pushing a `format!`: the same
        // result without allocating a throwaway String per opening, and the
        // suite runs to thousands of them.
        let _ = writeln!(
            out,
            "{} c0 \"{} {}\";",
            opening.fen, opening.eco, opening.name
        );
    }
    if let Err(err) = fs::write(&output, out) {
        eprintln!(
            "generate-openings: failed to write {}: {err}",
            output.display()
        );
        return ExitCode::FAILURE;
    }

    println!("wrote {} positions to {}", suite.len(), output.display());
    ExitCode::SUCCESS
}

/// `tools/selfplay/openings.epd`, resolved relative to this crate's own
/// manifest rather than the current directory, so the default works the
/// same regardless of where the binary is invoked from.
fn default_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../selfplay/openings.epd")
}

/// The absolute material imbalance in `board`, by `turox_engine`'s own
/// piece values, king excluded.
fn material_imbalance(board: &Board) -> i32 {
    Piece::ALL
        .into_iter()
        .filter(|&piece| piece != Piece::King)
        .map(|piece| {
            let value = i32::from(PIECE_VALUES[piece.index()]);
            let white =
                i32::try_from(board.pieces(Color::White, piece).count()).unwrap_or(i32::MAX);
            let black =
                i32::try_from(board.pieces(Color::Black, piece).count()).unwrap_or(i32::MAX);
            value * (white - black)
        })
        .sum::<i32>()
        .abs()
}

/// Replays every row, applies the ply/material/game-over filters, drops
/// transpositions to a position already kept, and returns the result
/// sorted by ECO then name so the file's order is a property of the data,
/// not of network response order. Match ordering is
/// `fastchess -openings ... order=random`'s job, not the file's.
fn build_suite(rows: Vec<OpeningRow>) -> Vec<Opening> {
    let mut seen = HashSet::new();
    let mut suite: Vec<Opening> = rows
        .into_iter()
        .filter_map(|row| {
            let line = parse_movetext(&row.pgn).ok()?;
            if !(MIN_PLIES..=MAX_PLIES).contains(&line.moves.len()) {
                return None;
            }
            let board = replay(&line)?;
            if legal_moves(&board).is_empty() || material_imbalance(&board) > MAX_MATERIAL_IMBALANCE
            {
                return None;
            }
            // Transpositions: several named lines reach the same
            // position, and a duplicate start position is a duplicate
            // game. The zobrist hash is exactly "this position, ignoring
            // the halfmove/fullmove counters" already, which is the same
            // notion of "the same position" `TaintedScore`'s repetition
            // detection uses elsewhere in this repo.
            if !seen.insert(board.hash()) {
                return None;
            }
            Some(Opening {
                fen: board.to_fen(),
                eco: row.eco,
                name: row.name,
            })
        })
        .collect();
    suite.sort_by(|a, b| (&a.eco, &a.name).cmp(&(&b.eco, &b.name)));
    suite
}
