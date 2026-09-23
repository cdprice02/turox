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
//! `pgn::tokenize_movetext` and `san::resolve_san` directly instead of a
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

use bookgen::pgn::tokenize_movetext;
use bookgen::san::resolve_san;
use clap::Parser;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;
use turox_engine::board::Board;
use turox_engine::eval::weights::PIECE_VALUES;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::{Color, Piece};

/// Pinned upstream revision, so regenerating reproduces the checked-in
/// suite.
const UPSTREAM_COMMIT: &str = "4b8622759e7ae6f93f011cc6c83a3823401ab45e";
const UPSTREAM_URL: &str =
    "https://raw.githubusercontent.com/lichess-org/chess-openings/{commit}/{volume}.tsv";
const ECO_VOLUMES: [char; 5] = ['a', 'b', 'c', 'd', 'e'];

/// A regeneration is five requests run by hand, so these bounds are not
/// about sustained load: they are about never leaving an unattended retry
/// running. A request with no timeout waits on a dead socket indefinitely,
/// and a retry loop with no ceiling turns one unreachable host into an
/// endless stream of connection attempts. That combination is what got
/// this machine's address null-routed by lichess once already, from a
/// different script, so nothing here talks to the network without both a
/// timeout and a bounded, spaced retry.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_ATTEMPTS: u32 = 3;
const FETCH_BACKOFF: Duration = Duration::from_secs(5);

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

/// One row of the upstream TSV: an ECO code, a human-readable name, and a
/// SAN movetext line, not yet replayed into a position.
struct OpeningRow {
    eco: String,
    name: String,
    pgn: String,
}

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

/// Returns the body of `url`, retrying only what a retry could fix.
fn fetch(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut last_err = String::new();
    for attempt in 1..=FETCH_ATTEMPTS {
        match agent.get(url).call() {
            Ok(mut response) => {
                return response
                    .body_mut()
                    .read_to_string()
                    .map_err(|err| format!("{url}: reading body: {err}"));
            }
            // A 4xx will not become a 2xx by asking again, and the
            // revision above is pinned, so a missing file means the pin
            // is wrong rather than that upstream is having a bad minute.
            Err(ureq::Error::StatusCode(code)) if code < 500 => {
                return Err(format!("{url}: HTTP {code}"));
            }
            Err(err) => {
                last_err = err.to_string();
                if attempt < FETCH_ATTEMPTS {
                    thread::sleep(FETCH_BACKOFF * attempt);
                }
            }
        }
    }
    Err(format!(
        "{url}: {last_err} (after {FETCH_ATTEMPTS} attempts)"
    ))
}

/// Parses a `chess-openings` TSV (`eco`, `name`, `pgn` columns, in
/// whatever order the header row gives them) into its rows. A row this
/// crate's own tab-splitting can't make sense of is dropped rather than
/// aborting the whole volume: these are hand-maintained TSVs upstream,
/// not adversarial input, but treating a stray malformed line as fatal
/// would make one upstream typo break every regeneration.
fn parse_tsv(text: &str) -> Vec<OpeningRow> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let columns: Vec<&str> = header.split('\t').collect();
    let Some(eco_col) = columns.iter().position(|&c| c == "eco") else {
        return Vec::new();
    };
    let Some(name_col) = columns.iter().position(|&c| c == "name") else {
        return Vec::new();
    };
    let Some(pgn_col) = columns.iter().position(|&c| c == "pgn") else {
        return Vec::new();
    };

    lines
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let eco = fields.get(eco_col)?;
            let name = fields.get(name_col)?;
            let pgn = fields.get(pgn_col)?;
            Some(OpeningRow {
                eco: (*eco).to_string(),
                name: (*name).to_string(),
                pgn: (*pgn).to_string(),
            })
        })
        .collect()
}

/// Replays `moves` into the position it reaches, or `None` if any token
/// along the way fails to resolve against the position at that point (a
/// malformed or ambiguous line, discarded whole rather than truncated).
fn replay(moves: &[String]) -> Option<Board> {
    let mut board = Board::start_pos();
    for token in moves {
        let mv = resolve_san(&board, token)?;
        board = board.make_move(mv);
    }
    Some(board)
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
            let moves = tokenize_movetext(&row.pgn);
            if !(MIN_PLIES..=MAX_PLIES).contains(&moves.len()) {
                return None;
            }
            let board = replay(&moves)?;
            if legal_moves(&board).as_slice().is_empty()
                || material_imbalance(&board) > MAX_MATERIAL_IMBALANCE
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
