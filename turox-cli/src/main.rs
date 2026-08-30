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
    engine.run();
}
