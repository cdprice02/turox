//! `turox-cli`: the binary that drives `turox_engine::Engine` over UCI (see
//! `Engine::run`), so any UCI-speaking GUI (or `lichess-bot`) can play
//! against it.

use clap::Parser;
use turox_engine::Engine;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {}

fn main() {
    // No stray output before `Engine::run` takes over: a real UCI GUI reads
    // stdout expecting only valid UCI responses, and anything else written
    // ahead of those (a debug print of `Args`, say) would corrupt the
    // stream from its perspective. `Args::parse()` still runs, so `--help`/
    // `--version` (from `#[clap(author, version, about)]`) keep working.
    Args::parse();

    let mut engine = Engine::new();
    match turox_chess::book::default_book() {
        Ok(book) => engine = engine.with_book(book),
        // stderr, not stdout: a real UCI GUI only reads stdout as protocol,
        // so a diagnostic line here can't corrupt that stream the way one
        // ahead of Args::parse() above would. An engine that can still
        // search is more useful than one that refuses to run at all over
        // its opening book specifically.
        Err(err) => {
            eprintln!(
                "turox: embedded opening book failed to load ({err:?}); continuing without it"
            );
        }
    }
    engine.run();
}
