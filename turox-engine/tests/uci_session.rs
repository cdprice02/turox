//! End-to-end UCI session tests, driven over in-memory buffers instead of
//! real stdin/stdout. This is what actually proves parsing, emission, and
//! search compose correctly together as one session, not just that each
//! works in isolation; a genuine integration test, so it lives here rather
//! than as a unit test alongside any one of those pieces.

use std::io::Cursor;
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
