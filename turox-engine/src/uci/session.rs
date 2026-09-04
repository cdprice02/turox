//! The stateful UCI session loop: reads commands from `R`, drives `board`
//! through them, and writes responses to `W`. The only stateful, I/O-doing
//! piece in the `uci` module; `command::parse` and `response::Response`
//! both stay pure.
//!
//! **Known simplification**: `Search::new`'s `history` (real-game position
//! hashes, for repetition detection that sees repeats from actual play, not
//! just within one search tree) is approximated here as one hash pushed per
//! `position` command received, not one per half-move actually played.
//! `Command::Position` resolves a whole `position ... moves ...` line
//! down to just the final `Board`, discarding the intermediate positions
//! each move in that list passed through, so there's currently no finer
//! granularity to work with. A GUI that resends the full move list on every
//! `position` command (the normal case) means this only sees one sample
//! point per command rather than the true position graph: good enough to
//! catch a repetition a GUI's own successive `position` commands span, not
//! guaranteed to catch every repetition within a single command's move
//! list. Fixing this properly means threading the per-move hash trail
//! through `Command::Position` itself, which is a deliberately separate,
//! later change, not something to fold into this loop silently.

use crate::board::Board;
use crate::search::time::allocate_time;
use crate::search::tt::Tt;
use crate::search::{Search, SearchResult};
use crate::types::Color;
use crate::uci::{self, Command, GoOptions, Response};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Iterative deepening starts at depth 1 and deepens until some budget
/// stops it first; this is only a ceiling on that loop; in any position
/// with a real time/node/`stop` budget, a search should never actually
/// reach it. `go infinite` and a bare `go` (no depth given at all) both
/// use it too, relying on `stop` alone to end the search.
const DEFAULT_MAX_DEPTH: u8 = 64;

/// Drives `board` from `reader`, writing UCI responses to `writer`, until
/// `quit` arrives or `reader` runs out of input. `reader` moves onto its
/// own thread (hence `Send + 'static`): a `stop`/`quit` sent while a search
/// is running has to reach it directly, since the thread that would
/// otherwise receive it is busy blocking inside `Search::search`. `writer`
/// stays on this thread; nothing here ever writes concurrently.
pub fn run<R, W>(board: &mut Board, reader: R, mut writer: W)
where
    R: BufRead + Send + 'static,
    W: Write,
{
    let (tx, rx) = mpsc::channel::<Command>();
    let active_stop: Arc<Mutex<Option<Arc<AtomicBool>>>> = Arc::new(Mutex::new(None));

    let reader_stop = Arc::clone(&active_stop);
    let reader_handle = thread::spawn(move || read_commands(reader, &tx, &reader_stop));

    let mut history: Vec<u64> = Vec::new();
    // Owned here, not by `Search`: `Search` is rebuilt fresh every `go`, so a table
    // living inside it would never see the transpositions that matter most in real
    // play, ones found across *separate* `go` calls in the same game. `history`
    // conceptually follows the same shape, though it's actually copied into `Search`
    // per call rather than borrowed.
    let mut tt = Tt::new(Tt::DEFAULT_HASH_MB);

    for command in rx {
        match command {
            Command::Uci => {
                send(&mut writer, &Response::IdName);
                send(&mut writer, &Response::IdAuthor);
                send(&mut writer, &Response::OptionHash);
                send(&mut writer, &Response::UciOk);
            }
            Command::IsReady => send(&mut writer, &Response::ReadyOk),
            Command::NewGame => {
                history.clear();
                tt.clear();
            }
            Command::Position(new_board) => {
                history.push(board.hash());
                *board = new_board;
            }
            Command::Go(options) => {
                let stop = Arc::new(AtomicBool::new(false));
                // A poisoned lock means another thread already panicked while
                // holding it; propagating that panic here, rather than silently
                // continuing with possibly-inconsistent shared state, is correct.
                *active_stop
                    .lock()
                    .expect("active_stop mutex should not be poisoned") = Some(Arc::clone(&stop));

                let (mut search, max_depth) =
                    build_search(board, history.clone(), &options, stop, &mut tt);
                // `search_with_info`, not plain `search`: streams an `info`
                // line after every completed depth, so a GUI watching a
                // long search sees progress instead of silence until
                // `bestmove`.
                let result = search.search_with_info(board, max_depth, |partial| {
                    send(&mut writer, &info_response(partial));
                });

                *active_stop
                    .lock()
                    .expect("active_stop mutex should not be poisoned") = None;

                // Zero iterations completed (max_depth == 0): the callback
                // above never fired, so send the one info line here instead.
                if result.depth == 0 {
                    send(&mut writer, &info_response(&result));
                }
                send(&mut writer, &Response::BestMove(result.best_move));
            }
            // Already handled by `read_commands` setting `active_stop`
            // directly: that's the only way to reach a search still
            // blocking this loop, so there's nothing left to do here.
            Command::Stop => {}
            // Only `Hash` has any effect right now; any other option name is a
            // recognized, well-formed command this engine just doesn't act on yet,
            // per UCI's own ignore-what-you-don't-support convention.
            Command::SetOption { name, value } => {
                if name == "Hash" {
                    // A value that doesn't parse as a number at all is ignored,
                    // leaving the table untouched, the same convention as an
                    // unrecognized option name; an in-range-or-not number is
                    // clamped rather than discarded, honoring the GUI's intent
                    // for an in-spirit-but-out-of-range request.
                    if let Some(mb) = value.as_deref().and_then(|v| v.parse::<usize>().ok()) {
                        tt.resize(mb.clamp(Tt::MIN_HASH_MB, Tt::MAX_HASH_MB));
                    }
                }
            }
            Command::Quit => break,
        }
    }

    // The loop above also ends this way if `reader` hit EOF without ever
    // sending `Quit` (dropping `tx` when `read_commands` returns closes
    // the channel, which ends `for command in rx` on its own); joining
    // either way just waits for that thread to have actually finished.
    let _ = reader_handle.join();
}

/// Builds the `Response::Info` line for one completed iteration's
/// `SearchResult`, shared between the per-depth streaming callback and the
/// zero-iterations fallback in `Command::Go`'s handling above, so both
/// paths format identically.
fn info_response(result: &SearchResult) -> Response {
    Response::Info {
        depth: result.depth,
        score: result.score,
        nodes: result.nodes,
        time: result.time,
        hashfull: result.hashfull,
        pv: result.best_move.into_iter().collect(),
    }
}

fn send(writer: &mut impl Write, response: &Response) {
    // A GUI is actively waiting on most of these (`uciok`, `readyok`,
    // `bestmove`); an unflushed line sitting in a buffer never reaching it
    // would look identical to the engine having hung. Errors here mean the
    // other end went away, nothing to do differently in response.
    let _ = writeln!(writer, "{response}");
    let _ = writer.flush();
}

/// Runs on its own thread: reads lines from `reader`, parses each into a
/// `Command`, and sends it to the main loop over `tx`. `Stop`/`Quit` also
/// set `active_stop` directly, bypassing `tx` entirely: the main loop can't
/// drain the channel while it's blocked inside `Search::search`, so this is
/// the only way those two commands can reach a search that's already
/// running.
fn read_commands<R: BufRead>(
    mut reader: R,
    tx: &mpsc::Sender<Command>,
    active_stop: &Arc<Mutex<Option<Arc<AtomicBool>>>>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return, // EOF, or a real I/O error: nothing more to read either way
            Ok(_) => {}
        }

        let Some(command) = uci::parse(line.trim_end()) else {
            continue; // unrecognized or malformed line: ignore, per UCI's own spec
        };

        if matches!(command, Command::Stop | Command::Quit) {
            if let Some(stop) = active_stop
                .lock()
                .expect("active_stop mutex should not be poisoned")
                .as_ref()
            {
                stop.store(true, Ordering::Relaxed);
            }
        }

        let is_quit = matches!(command, Command::Quit);
        if tx.send(command).is_err() || is_quit {
            return;
        }
    }
}

/// Turns `options` into a `Search` (seeded with `history`, `stop`, and `tt`) and
/// the `max_depth` to hand `Search::search`.
fn build_search<'a>(
    board: &Board,
    history: Vec<u64>,
    options: &GoOptions,
    stop: Arc<AtomicBool>,
    tt: &'a mut Tt,
) -> (Search<'a>, u8) {
    let mut search = Search::new(history)
        .with_stop_flag(stop)
        .with_tt(tt)
        .with_root_randomization(root_seed());

    if let Some(nodes) = options.nodes {
        search = search.with_max_nodes(nodes);
    }
    if let Some(deadline) = go_deadline(board, options) {
        search = search.with_deadline(deadline);
    }

    let max_depth = options.depth.unwrap_or(DEFAULT_MAX_DEPTH);
    (search, max_depth)
}

/// `infinite`, checked first, wins outright: per UCI, `go infinite` means
/// search until `stop` alone, with no depth/time budget at all, so it has
/// to bypass `movetime` and the clock fields entirely, not just fall
/// through to them being absent. A real GUI can and does send `go
/// infinite` alongside `wtime`/`btime` (both are simply always attached to
/// `go`, independent of whether `infinite` is also set), so this can't be
/// "no deadline" merely as a side effect of the clock fields happening to
/// be unset.
///
/// Otherwise, `movetime`, if given, wins outright. Failing that, with a
/// real game clock (`wtime`/`btime`), budgets from *whichever side's
/// clock is actually running* (`board.side_to_move()`, not always
/// White's) via `search::time::allocate_time`. None of the above (a bare
/// `go`, or `go depth N` with no clock fields) means no deadline at all:
/// depth, node count, and `stop` are what bound the search instead.
/// A per-search seed for root move randomization, taken from the wall clock.
///
/// Deliberately not a fixed seed: a fixed one would make every *game* identical
/// again, just along a different line than the unrandomized engine played, which
/// is the exact problem this exists to solve. Reproducibility lives at the
/// `Search` layer instead, where a caller passes its own seed.
///
/// Nanosecond resolution rather than seconds: two `go` commands inside the same
/// second are the normal case in a fast time control, and a second-resolution
/// clock would hand them the same seed.
fn root_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

fn go_deadline(board: &Board, options: &GoOptions) -> Option<Instant> {
    if options.infinite {
        return None;
    }
    if let Some(movetime) = options.movetime {
        return Some(Instant::now() + movetime);
    }

    let (time_left, increment) = match board.side_to_move() {
        Color::White => (options.wtime?, options.winc.unwrap_or(Duration::ZERO)),
        Color::Black => (options.btime?, options.binc.unwrap_or(Duration::ZERO)),
    };
    Some(Instant::now() + allocate_time(time_left, increment, options.movestogo))
}
