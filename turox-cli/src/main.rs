//! `turox-cli`: the binary that drives `turox_engine::Engine`, eventually over
//! UCI (see `Engine::run`). Not yet a UCI frontend: the engine's own
//! `search`/`uci` modules are what it will call once those land.

use clap::Parser;
use turox_engine::Engine;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {}

fn main() {
    let args = Args::parse();
    println!("{:?}", args);

    let engine = Engine::new();
    engine.run();
}
