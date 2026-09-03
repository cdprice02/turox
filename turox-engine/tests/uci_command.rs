//! Concrete scenario tests for `uci::command::parse`.
//!
//! `tests/uci_command_props.rs` has the round-trip property (a
//! `position fen ... moves ...` line built from `Board::to_fen`/`Move::to_uci`
//! recovers the right board); these are one test per command, from its exact
//! spec string, plus the malformed-input cases.

use std::time::Duration;
use turox_engine::board::Board;
use turox_engine::move_gen::legal::legal_moves;
use turox_engine::uci::{parse, Command, GoOptions};

// ---- Concrete: one command per type, from its exact spec string ----

#[test]
fn parses_uci() {
    assert_eq!(parse("uci"), Some(Command::Uci));
}

#[test]
fn parses_isready() {
    assert_eq!(parse("isready"), Some(Command::IsReady));
}

#[test]
fn parses_ucinewgame() {
    assert_eq!(parse("ucinewgame"), Some(Command::NewGame));
}

#[test]
fn parses_stop() {
    assert_eq!(parse("stop"), Some(Command::Stop));
}

#[test]
fn parses_quit() {
    assert_eq!(parse("quit"), Some(Command::Quit));
}

#[test]
fn parses_position_startpos_with_no_moves() {
    assert_eq!(
        parse("position startpos"),
        Some(Command::Position(Board::start_pos()))
    );
}

#[test]
fn parses_position_startpos_with_a_move_list() {
    let board = Board::start_pos();
    let e2e4 = *legal_moves(&board)
        .as_slice()
        .iter()
        .find(|m| m.to_uci() == "e2e4")
        .expect("e2e4 is legal from startpos");
    let after_e4 = board.make_move(e2e4);
    let e7e5 = *legal_moves(&after_e4)
        .as_slice()
        .iter()
        .find(|m| m.to_uci() == "e7e5")
        .expect("e7e5 is legal after 1.e4");
    let expected = after_e4.make_move(e7e5);

    assert_eq!(
        parse("position startpos moves e2e4 e7e5"),
        Some(Command::Position(expected))
    );
}

#[test]
fn parses_position_fen_with_no_moves() {
    let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    let expected = Board::try_from_fen(fen).expect("valid FEN");
    assert_eq!(
        parse(&format!("position fen {fen}")),
        Some(Command::Position(expected))
    );
}

#[test]
fn parses_position_fen_with_a_move_list() {
    let fen = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
    let board = Board::try_from_fen(fen).expect("valid FEN");
    let m = *legal_moves(&board)
        .as_slice()
        .iter()
        .find(|m| m.to_uci() == "e1d1")
        .expect("Ke1-d1 is legal here");
    let expected = board.make_move(m);

    assert_eq!(
        parse(&format!("position fen {fen} moves e1d1")),
        Some(Command::Position(expected))
    );
}

#[test]
fn parses_bare_go_as_all_default_options() {
    assert_eq!(parse("go"), Some(Command::Go(GoOptions::default())));
}

#[test]
fn parses_go_infinite() {
    assert_eq!(
        parse("go infinite"),
        Some(Command::Go(GoOptions {
            infinite: true,
            ..GoOptions::default()
        }))
    );
}

#[test]
fn parses_go_depth() {
    assert_eq!(
        parse("go depth 5"),
        Some(Command::Go(GoOptions {
            depth: Some(5),
            ..GoOptions::default()
        }))
    );
}

#[test]
fn parses_go_nodes() {
    assert_eq!(
        parse("go nodes 1000"),
        Some(Command::Go(GoOptions {
            nodes: Some(1000),
            ..GoOptions::default()
        }))
    );
}

#[test]
fn parses_go_movetime() {
    assert_eq!(
        parse("go movetime 100"),
        Some(Command::Go(GoOptions {
            movetime: Some(Duration::from_millis(100)),
            ..GoOptions::default()
        }))
    );
}

#[test]
fn parses_go_with_a_real_game_clock() {
    assert_eq!(
        parse("go wtime 60000 btime 55000 winc 1000 binc 2000 movestogo 30"),
        Some(Command::Go(GoOptions {
            wtime: Some(Duration::from_secs(60)),
            btime: Some(Duration::from_secs(55)),
            winc: Some(Duration::from_millis(1000)),
            binc: Some(Duration::from_millis(2000)),
            movestogo: Some(30),
            ..GoOptions::default()
        }))
    );
}

/// Order shouldn't matter: `go`'s own sub-options are independent
/// keyword/value pairs per the UCI spec, not fixed-position arguments.
#[test]
fn go_options_parse_the_same_regardless_of_order() {
    let forward = parse("go depth 5 nodes 1000");
    let reversed = parse("go nodes 1000 depth 5");
    assert_eq!(forward, reversed);
    assert_eq!(
        forward,
        Some(Command::Go(GoOptions {
            depth: Some(5),
            nodes: Some(1000),
            ..GoOptions::default()
        }))
    );
}

/// A real UCI keyword this engine doesn't act on (`ponder`, no pondering
/// support) shouldn't cost the rest of the line: `go`'s own budget fields
/// still parse normally around it, unlike `position`, where an unresolvable
/// token fails the whole command.
#[test]
fn go_ignores_an_unrecognized_token_without_losing_the_rest_of_the_line() {
    assert_eq!(
        parse("go ponder depth 5"),
        Some(Command::Go(GoOptions {
            depth: Some(5),
            ..GoOptions::default()
        }))
    );
}

#[test]
fn parses_setoption_with_a_value() {
    assert_eq!(
        parse("setoption name Hash value 64"),
        Some(Command::SetOption {
            name: "Hash".to_string(),
            value: Some("64".to_string()),
        })
    );
}

#[test]
fn parses_setoption_with_no_value() {
    assert_eq!(
        parse("setoption name Ponder"),
        Some(Command::SetOption {
            name: "Ponder".to_string(),
            value: None,
        })
    );
}

/// Both `name` and `value` are real multi-word UCI options in practice
/// (`Debug Log File`, a `string`-type value containing spaces); this
/// checks both sides of the `value` keyword collect every token they're
/// given, not just the first one.
#[test]
fn parses_setoption_with_a_multi_word_name_and_value() {
    assert_eq!(
        parse("setoption name Debug Log File value C:\\temp log file.txt"),
        Some(Command::SetOption {
            name: "Debug Log File".to_string(),
            value: Some("C:\\temp log file.txt".to_string()),
        })
    );
}

/// An option this engine doesn't recognize still parses to a real
/// `Command::SetOption`: whether to act on it is downstream of parsing,
/// not parsing's own job (`parse`'s module doc makes the same point about
/// unrecognized `go` sub-options).
#[test]
fn parses_setoption_for_an_unrecognized_option_name() {
    assert_eq!(
        parse("setoption name UCI_Chess960 value true"),
        Some(Command::SetOption {
            name: "UCI_Chess960".to_string(),
            value: Some("true".to_string()),
        })
    );
}

// ---- Malformed input: never panics, and never guesses ----

#[test]
fn rejects_garbage_without_panicking() {
    for bad in [
        "",
        "   ",
        "notacommand",
        "position",
        "position fen",
        "position fen 8/8/8/8/8/8/8 w - - 0 1", // only 7 ranks
        "position fen not a real fen at all moves e2e4",
        "position startpos moves e2e4 zzzz", // unresolvable move
        "position startpos moves e2e4 e7e5 e2e4", // e2 is empty by move 3
        "go depth",                          // keyword with no value
        "go depth abc",                      // not a number
        "go movetime abc",
        "setoption",          // missing `name`
        "setoption value 64", // `value` with no `name`
        "€ from_uci garbage €",
    ] {
        assert_eq!(parse(bad), None, "expected None for {bad:?}");
    }
}
