//! The `lichess-org/chess-openings` data set: where to get it, how to read
//! it, and how to turn one of its lines into a position.
//!
//! Shared by the two tools that consume it, the SPRT opening suite and the
//! book's opening names, so that a change to the upstream pin or the row
//! format lands in one place rather than in whichever tool was edited last.
//!
//! Released under CC0 1.0 (public domain dedication).

#[cfg(feature = "fetch")]
use std::thread;
#[cfg(feature = "fetch")]
use std::time::Duration;
use turox_chess::board::Board;
use turox_notation::pgn::Line;
use turox_notation::san::resolve_san;

/// Pinned upstream revision, so regenerating reproduces the checked-in
/// suite.
pub const UPSTREAM_COMMIT: &str = "4b8622759e7ae6f93f011cc6c83a3823401ab45e";
/// Where a volume's TSV lives, with `{commit}` and `{volume}` substituted.
pub const UPSTREAM_URL: &str =
    "https://raw.githubusercontent.com/lichess-org/chess-openings/{commit}/{volume}.tsv";
/// The five ECO volumes, one TSV each.
pub const ECO_VOLUMES: [char; 5] = ['a', 'b', 'c', 'd', 'e'];

/// A regeneration is five requests run by hand.
///
/// So these bounds are not about sustained load: they are about never
/// leaving an unattended retry running. A request with no timeout waits on a dead socket indefinitely,
/// and a retry loop with no ceiling turns one unreachable host into an
/// endless stream of connection attempts. That combination is what got
/// this machine's address null-routed by lichess once already, from a
/// different script, so nothing here talks to the network without both a
/// timeout and a bounded, spaced retry.
/// How long one request may wait before it is a failure rather than a wait.
#[cfg(feature = "fetch")]
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How many times a request is retried before giving up.
#[cfg(feature = "fetch")]
pub const FETCH_ATTEMPTS: u32 = 3;
/// Base spacing between retries, multiplied by the attempt number.
#[cfg(feature = "fetch")]
pub const FETCH_BACKOFF: Duration = Duration::from_secs(5);

/// One row of an upstream TSV: an ECO code, the opening's name, and the
/// line that reaches it in PGN movetext.
pub struct OpeningRow {
    /// The ECO classification code, for example `C65`.
    pub eco: String,
    /// The opening's name in English.
    pub name: String,
    /// The line reaching it, as PGN movetext.
    pub pgn: String,
}

/// Parses a `chess-openings` TSV into its rows.
///
/// Reads the `eco`, `name` and `pgn` columns in whatever order the header
/// row gives them. A row this crate's own tab-splitting can't make sense of
/// is dropped rather than aborting the whole volume: these are hand-maintained TSVs upstream,
/// not adversarial input, but treating a stray malformed line as fatal
/// would make one upstream typo break every regeneration.
#[must_use]
pub fn parse_tsv(text: &str) -> Vec<OpeningRow> {
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

/// Replays `line`'s mainline into the position it reaches.
///
/// `None` if any token along the way fails to resolve against the position
/// at that point: a malformed or ambiguous line, discarded whole rather than
/// truncated.
#[must_use]
pub fn replay(line: &Line) -> Option<Board> {
    let mut board = Board::start_pos();
    for token in line.mainline() {
        let mv = resolve_san(&board, token)?;
        board = board.make_move(mv);
    }
    Some(board)
}

/// Returns the body of `url`, retrying only what a retry could fix.
///
/// Behind `fetch` with `ureq`, so a default build compiles no TLS stack.
///
/// # Errors
///
/// The last transport or status error seen, once the attempt budget is
/// spent. A non-retryable status fails immediately rather than waiting it
/// out.
#[cfg(feature = "fetch")]
pub fn fetch(agent: &ureq::Agent, url: &str) -> Result<String, String> {
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
