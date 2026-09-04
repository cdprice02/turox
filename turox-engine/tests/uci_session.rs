//! End-to-end UCI session tests, driven over in-memory buffers instead of
//! real stdin/stdout. This is what actually proves parsing, emission, and
//! search compose correctly together as one session, not just that each
//! works in isolation; a genuine integration test, so it lives here rather
//! than as a unit test alongside any one of those pieces.

use std::io::{BufReader, Cursor, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::Engine;

/// Feeds `input` to a fresh `Engine` over an in-memory buffer and returns
/// everything it wrote back, as a `String`.
fn run_session(input: &str) -> String {
    let mut engine = Engine::new();
    let reader = Cursor::new(input.as_bytes().to_vec());
    let mut output = Vec::new();
    engine.run_with_io(reader, &mut output);
    String::from_utf8(output).expect("UCI output must be valid UTF-8")
}

#[test]
fn uci_command_gets_identification_and_uciok() {
    let output = run_session("uci\nquit\n");
    assert!(output.contains("id name turox"), "output: {output:?}");
    assert!(
        output.contains("id author Carson Price"),
        "output: {output:?}"
    );
    assert!(
        output.contains("option name Hash type spin default 16 min 1 max 1024"),
        "output: {output:?}"
    );
    assert!(output.contains("uciok"), "output: {output:?}");
}

#[test]
fn isready_gets_readyok() {
    let output = run_session("isready\nquit\n");
    assert!(output.contains("readyok"), "output: {output:?}");
}

/// The issue's own acceptance test: `position ... moves ...` then `go`
/// produces a `bestmove` that's actually legal in the position those two
/// moves reach, not just some hardcoded fallback the loop already had a
/// board for. The expected position is rebuilt independently here via
/// `Board::make_move` directly, not by trusting the session's own
/// position-tracking to have gotten it right.
#[test]
fn position_and_go_returns_a_legal_bestmove() {
    let output = run_session("position startpos moves e2e4 e7e5\ngo depth 3\nquit\n");

    let bestmove_line = output
        .lines()
        .find(|line| line.starts_with("bestmove "))
        .unwrap_or_else(|| panic!("no bestmove line in output: {output:?}"));
    let uci_move = bestmove_line
        .strip_prefix("bestmove ")
        .expect("checked above")
        .trim();

    let start = Board::start_pos();
    let e2e4 = *legal_moves(&start)
        .as_slice()
        .iter()
        .find(|m| m.to_uci() == "e2e4")
        .expect("e2e4 is legal from startpos");
    let after_e4 = start.make_move(e2e4);
    let e7e5 = *legal_moves(&after_e4)
        .as_slice()
        .iter()
        .find(|m| m.to_uci() == "e7e5")
        .expect("e7e5 is legal after 1.e4");
    let reached = after_e4.make_move(e7e5);

    let legal = legal_moves(&reached);
    assert!(
        legal.as_slice().iter().any(|m| m.to_uci() == uci_move),
        "bestmove {uci_move:?} must be legal after 1.e4 e5, output: {output:?}"
    );
}

/// A root position that's already a draw still has to produce a real
/// `bestmove`, not the null move `0000`, whenever legal moves exist. Two
/// independent drawn positions, both reached the way a real GUI would:
///
/// - The fifty-move rule, via `position fen ...` with `halfmove_clock`
///   already at the 100-half-move threshold.
/// - A genuine threefold repetition, via three separate `position`
///   commands resending the growing move list, since `session::run` only
///   samples one history hash per `position` command (see its own doc), so
///   the repetition has to span commands to be visible here.
#[test]
fn drawn_root_position_still_returns_a_real_bestmove() {
    let fifty_move_output =
        run_session("position fen 4k3/8/8/8/8/8/6R1/R3K3 w - - 100 1\ngo depth 4\nquit\n");
    let fifty_move_bestmove = fifty_move_output
        .lines()
        .find(|line| line.starts_with("bestmove "))
        .unwrap_or_else(|| panic!("no bestmove line in output: {fifty_move_output:?}"));
    assert_ne!(
        fifty_move_bestmove, "bestmove 0000",
        "fifty-move draw with legal moves available must not report the null move, output: {fifty_move_output:?}"
    );

    let threefold_output = run_session(concat!(
        "position startpos\n",
        "position startpos moves g1f3 g8f6 f3g1 f6g8\n",
        "position startpos moves g1f3 g8f6 f3g1 f6g8 g1f3 g8f6 f3g1 f6g8\n",
        "go depth 4\n",
        "quit\n",
    ));
    let threefold_bestmove = threefold_output
        .lines()
        .find(|line| line.starts_with("bestmove "))
        .unwrap_or_else(|| panic!("no bestmove line in output: {threefold_output:?}"));
    assert_ne!(
        threefold_bestmove, "bestmove 0000",
        "threefold repetition with legal moves available must not report the null move, output: {threefold_output:?}"
    );
}

/// A root position where depth 1 itself doesn't finish inside the node
/// budget still has to produce a real `bestmove`, not `0000`: distinct from
/// `drawn_root_position_still_returns_a_real_bestmove` above, which starts
/// from a full move budget, this is any root with the budget expiring
/// before the first iteration completes. `go nodes 1000` on this position
/// is short of the 2376 nodes depth 1 actually costs here (confirmed via
/// `go nodes 3000` returning a real move), so the first iteration always
/// aborts partway through.
#[test]
fn interrupted_first_iteration_still_returns_a_real_bestmove() {
    let output = run_session(concat!(
        "position fen r1bqk2r/ppp2ppp/2n5/3np1N1/1bBP4/2P5/PP3PPP/RNBQK2R b KQkq - 0 1\n",
        "go nodes 1000\n",
        "quit\n",
    ));
    let bestmove_line = output
        .lines()
        .find(|line| line.starts_with("bestmove "))
        .unwrap_or_else(|| panic!("no bestmove line in output: {output:?}"));
    assert_ne!(
        bestmove_line, "bestmove 0000",
        "depth 1 aborting before it completes must not report the null move, output: {output:?}"
    );
}

/// `session::run` streams an `info depth` line after every completed
/// iteration, not just the final one. `go depth 3` with no deadline
/// completes deterministically, so this counts exactly three lines rather
/// than depending on timing to land a partial one.
///
/// No trailing `quit`, deliberately: the reader thread parses this whole
/// in-memory buffer well before the main thread finishes `go depth 3`'s
/// blocking search, so a `quit` right behind it races `Command::Go` for
/// who sets `active_stop` first, and can genuinely abort depth 3 partway
/// through depending who wins. A preexisting property of this reader
/// design (see `session_ends_cleanly_on_eof_with_no_explicit_quit` above),
/// not something this change introduced; ending on EOF instead sidesteps
/// the race entirely.
#[test]
fn go_depth_streams_an_info_line_per_completed_depth() {
    let output = run_session("position startpos\ngo depth 3\n");

    let info_depths: Vec<u32> = output
        .lines()
        .filter(|line| line.starts_with("info depth "))
        .map(|line| {
            line.strip_prefix("info depth ")
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|depth| depth.parse().ok())
                .unwrap_or_else(|| panic!("malformed info depth line: {line:?}"))
        })
        .collect();

    assert_eq!(
        info_depths,
        vec![1, 2, 3],
        "expected one info line per completed depth 1..=3, output: {output:?}"
    );
}

/// No trailing `quit`: the reader thread hits EOF, drops its `Sender`, and
/// the main loop's `for command in rx` ends on its own once the channel
/// closes. Proves the session doesn't hang waiting for a `quit` that never
/// comes, the real risk in a hand-rolled threaded loop like this one.
#[test]
fn session_ends_cleanly_on_eof_with_no_explicit_quit() {
    let output = run_session("isready\n");
    assert!(output.contains("readyok"), "output: {output:?}");
}

/// Garbage lines interleaved with real commands are ignored (per UCI's own
/// spec, and `command::parse`'s own contract), not something that derails
/// the rest of the session.
#[test]
fn malformed_lines_are_ignored_without_disrupting_the_session() {
    let output = run_session("not a uci command\n\x00\x00\nisready\ngarbage again\nquit\n");
    assert!(output.contains("readyok"), "output: {output:?}");
}

/// `setoption` isn't wired to anything yet (a transposition table for
/// `Hash` to resize doesn't exist), but it has to be a recognized command
/// the session accepts without derailing whatever comes after it, the same
/// way a genuinely malformed line is ignored above.
#[test]
fn setoption_is_accepted_without_disrupting_the_session() {
    let output = run_session("setoption name Hash value 64\nisready\nquit\n");
    assert!(output.contains("readyok"), "output: {output:?}");
}

/// An out-of-range `Hash` value (above the `max` `option name Hash` itself advertises)
/// clamps rather than getting rejected outright: the session, and a search run right
/// after, both still have to work normally.
#[test]
fn setoption_hash_above_the_max_is_clamped_not_rejected() {
    let output =
        run_session("setoption name Hash value 999999\nposition startpos\ngo depth 2\nquit\n");
    assert!(
        output.lines().any(|line| line.starts_with("bestmove ")),
        "output: {output:?}"
    );
}

/// A `Hash` value that doesn't parse as a number at all is ignored (the table is left
/// untouched), not a reason to derail the rest of the session.
#[test]
fn setoption_hash_with_a_non_numeric_value_is_ignored() {
    let output = run_session("setoption name Hash value banana\nisready\nquit\n");
    assert!(output.contains("readyok"), "output: {output:?}");
}

/// `ucinewgame` clears the transposition table alongside `history`; a search run right
/// after still has to produce a real move, not something a stale or freshly-cleared
/// table could derail.
#[test]
fn ucinewgame_still_allows_a_normal_search_afterward() {
    let output = run_session(
        "position startpos\ngo depth 2\nucinewgame\nposition startpos\ngo depth 2\nquit\n",
    );
    let bestmove_count = output
        .lines()
        .filter(|line| line.starts_with("bestmove "))
        .count();
    assert_eq!(
        bestmove_count, 2,
        "both go commands, before and after ucinewgame, must produce a bestmove, output: {output:?}"
    );
}

/// `go infinite`'s own doc says "search until `stop`, no depth/time budget
/// at all"; this is the regression test for the bug where `go_deadline`
/// parsed `infinite` but never consulted it, so a real GUI's `go infinite
/// wtime ... btime ...` (both clock fields are simply always attached to
/// `go`, independent of `infinite`) got a clock-derived deadline anyway.
///
/// Drives the session over a real OS pipe rather than `run_session`'s
/// in-memory `Cursor`: this test needs to control *when* `stop` arrives
/// relative to the search actually starting, which an upfront buffer can't
/// do (the reader thread would race the main thread to decide whether
/// `stop` lands before or after the search begins, `go_depth_streams_an_
/// info_line_per_completed_depth` above documents that exact race for a
/// bounded search). A real pipe blocks the reader thread on `read_line`
/// until bytes are actually written, so this test's own `sleep` reliably
/// happens *during* the search rather than racing its start.
///
/// `wtime`/`btime` are deliberately tiny (1 second): if the bug were still
/// present, `allocate_time` would hand back a deadline on that same order,
/// and the sleep below (comfortably shorter) would still catch a
/// `bestmove` that arrived on its own before `stop` was ever sent.
#[test]
fn go_infinite_ignores_the_clock_and_waits_for_stop() {
    let (reader, mut writer) = std::io::pipe().expect("creating an OS pipe should not fail");
    let output = Arc::new(Mutex::new(Vec::new()));

    let mut engine = Engine::new();
    let session_output = SharedOutput(Arc::clone(&output));
    let handle = thread::spawn(move || {
        engine.run_with_io(BufReader::new(reader), session_output);
    });

    writer
        .write_all(b"position startpos\ngo infinite wtime 1000 btime 1000\n")
        .expect("writing to the pipe should not fail");

    thread::sleep(Duration::from_millis(300));
    assert!(
        !contains_bestmove(&output),
        "go infinite must not return on its own before `stop`, even with a real \
         game clock attached, output so far: {:?}",
        String::from_utf8_lossy(&output.lock().expect("mutex not poisoned"))
    );

    writer
        .write_all(b"stop\nquit\n")
        .expect("writing to the pipe should not fail");
    drop(writer);

    handle.join().expect("session thread should not panic");
    assert!(
        contains_bestmove(&output),
        "go infinite must return a bestmove once `stop` is sent, output: {:?}",
        String::from_utf8_lossy(&output.lock().expect("mutex not poisoned"))
    );
}

fn contains_bestmove(output: &Arc<Mutex<Vec<u8>>>) -> bool {
    output
        .lock()
        .expect("mutex not poisoned")
        .windows(b"bestmove ".len())
        .any(|window| window == b"bestmove ")
}

/// `Engine::run_with_io` takes its writer by value; this shares one
/// `Vec<u8>` between the session thread (which writes) and the test thread
/// (which reads it mid-flight, before the session has finished) via a
/// `Mutex` rather than requiring the session to finish before its output
/// is readable at all.
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl Write for SharedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("mutex not poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The fields a GUI and a game record need in order to describe a search
/// without it having to be reconstructed offline. Asserted end to end through
/// a real session rather than only on `Response`'s `Display`, since the two
/// can drift: the formatting could be right while `info_response` forgets to
/// pass the values through.
///
/// `hashfull` is included because the session owns the transposition table,
/// so a real session is the only place it can be non-`None`.
#[test]
fn go_emits_time_nps_and_hashfull_on_every_info_line() {
    let output = run_session("position startpos\ngo depth 3\n");

    let info_lines: Vec<&str> = output
        .lines()
        .filter(|line| line.starts_with("info depth "))
        .collect();
    assert!(
        !info_lines.is_empty(),
        "expected at least one info line, output: {output:?}"
    );

    for line in info_lines {
        for field in ["nodes ", "nps ", "hashfull ", "time "] {
            assert!(
                line.contains(field),
                "info line is missing {field:?}: {line:?}"
            );
        }
    }
}

/// UCI's field order is conventional rather than enforced, but a GUI parsing
/// positionally is a real thing, and `pv` in particular must stay last: it is
/// the one variable-length field, so anything after it would be swallowed into
/// the move list.
#[test]
fn info_line_keeps_pv_last() {
    let output = run_session("position startpos\ngo depth 2\n");

    let with_pv = output
        .lines()
        .find(|line| line.starts_with("info depth ") && line.contains(" pv "))
        .unwrap_or_else(|| panic!("expected an info line carrying a pv, output: {output:?}"));

    let after_pv = with_pv
        .split_once(" pv ")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    for field in ["nodes", "nps", "hashfull", "time", "score", "depth"] {
        assert!(
            !after_pv.contains(field),
            "{field:?} appears after the pv in: {with_pv:?}"
        );
    }
}
