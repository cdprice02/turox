//! The `bookgen` binary: reads one or more PGN files, builds a
//! `turox_engine::book::Book` from them, and writes the result to a file
//! in `Book::to_bytes`'s format.

use bookgen::aggregate::BuildOptions;
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

/// Command-line arguments for a single `bookgen` run.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// One or more PGN files to read. All games from every file are
    /// aggregated together, so transpositions across files still merge.
    #[arg(long = "input", required = true)]
    inputs: Vec<PathBuf>,
    /// Where to write the resulting book file.
    #[arg(long = "output")]
    output: PathBuf,
    /// Both players must meet this rating for a game to count at all.
    #[arg(long = "min-rating", default_value_t = 2300)]
    min_rating: u32,
    /// A position must be reached by at least this many qualifying games
    /// to stay in the book.
    #[arg(long = "min-sample-size", default_value_t = 10)]
    min_sample_size: u32,
    /// The hard ply cap: no position past this many plies is ever
    /// recorded, regardless of how well-attested a longer line is.
    #[arg(long = "max-ply", default_value_t = 20)]
    max_ply: u32,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let mut pgn_text = String::new();
    for input in &args.inputs {
        match fs::read_to_string(input) {
            Ok(text) => pgn_text.push_str(&text),
            Err(err) => {
                eprintln!("bookgen: failed to read {}: {err}", input.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let options = BuildOptions {
        min_rating: args.min_rating,
        min_sample_size: args.min_sample_size,
        max_ply: args.max_ply,
    };
    let book = bookgen::build_book(&pgn_text, &options);

    if let Err(err) = fs::write(&args.output, book.to_bytes()) {
        eprintln!("bookgen: failed to write {}: {err}", args.output.display());
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
