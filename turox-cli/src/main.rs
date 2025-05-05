use clap::Parser;
use turox_engine::Engine;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {}

fn main() {
    let args = Args::parse();
    println!("{:?}", args);

    let engine = Engine::new();
    println!("{}", engine.fen());
}
