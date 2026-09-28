//! Fills an opening book's name table from the `lichess-org/chess-openings`
//! data set.
//!
//! The book is keyed by position hash and stores no paths, so a name cannot
//! simply be looked up per entry: the data set names a *line*, and what it
//! names is the position that line reaches. This walks the book forward from
//! the start position instead, which reconstructs each position's board (what
//! applying a move needs, and what a hash alone cannot give) and carries the
//! name in force down every line.
//!
//! A position the data set names directly takes that name. Any other position
//! inherits the deepest name on the way to it, so a move past the end of named
//! theory still reports the line it came from. Deepest, not first seen: two
//! paths into one position can carry different names, and the more specific of
//! them is the better answer rather than whichever arrived first.
//!
//! Source data: <https://github.com/lichess-org/chess-openings>, CC0 1.0. The
//! upstream revision is pinned in `bookgen::openings`, so a regeneration
//! reproduces the checked-in book rather than drifting.

use bookgen::openings::{
    fetch, parse_tsv, replay, ECO_VOLUMES, FETCH_TIMEOUT, UPSTREAM_COMMIT, UPSTREAM_URL,
};
use clap::Parser;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use turox_chess::board::Board;
use turox_chess::book::Book;
use turox_notation::pgn::tokenize_movetext;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The book to read.
    #[arg(long = "input", required = true)]
    input: PathBuf,
    /// Where to write the named book. Defaults to overwriting `--input`.
    #[arg(long = "output")]
    output: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("name-book: {message}");
            ExitCode::FAILURE
        }
    }
}

/// One named position from the data set: the name, and how many plies its
/// line runs, which is what makes one name more specific than another.
struct Named {
    name: String,
    plies: usize,
}

fn run(args: &Args) -> Result<String, String> {
    let bytes =
        fs::read(&args.input).map_err(|e| format!("reading {}: {e}", args.input.display()))?;
    let book =
        Book::from_bytes(&bytes).map_err(|e| format!("loading {}: {e:?}", args.input.display()))?;

    let named = load_named()?;
    let (table, report) = walk(&book, &named);

    let entries: Vec<(u64, Vec<_>)> = book
        .positions()
        .map(|(hash, mvs)| (hash, mvs.to_vec()))
        .collect();
    let out = args.output.clone().unwrap_or_else(|| args.input.clone());
    let named_book = Book::with_names(entries, table);
    fs::write(&out, named_book.to_bytes())
        .map_err(|e| format!("writing {}: {e}", out.display()))?;

    Ok(report)
}

/// Every named position the data set describes, by the hash of the position
/// its line reaches.
fn load_named() -> Result<HashMap<u64, Named>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .new_agent();

    let mut named: HashMap<u64, Named> = HashMap::new();
    for volume in ECO_VOLUMES {
        #[expect(
            clippy::literal_string_with_formatting_args,
            reason = "placeholders in a URL template, not a format string"
        )]
        let url = UPSTREAM_URL
            .replace("{commit}", UPSTREAM_COMMIT)
            .replace("{volume}", &volume.to_string());
        for row in parse_tsv(&fetch(&agent, &url)?) {
            let moves = tokenize_movetext(&row.pgn);
            let Some(board) = replay(&moves) else {
                continue;
            };
            let entry = Named {
                name: row.name,
                plies: moves.len(),
            };
            // A position two rows both reach keeps the more specific naming,
            // the same rule the walk below applies to inheritance.
            named
                .entry(board.hash())
                .and_modify(|held| {
                    if entry.plies > held.plies {
                        held.name.clone_from(&entry.name);
                        held.plies = entry.plies;
                    }
                })
                .or_insert(entry);
        }
    }

    if named.is_empty() {
        return Err("the data set produced no named positions".to_string());
    }
    Ok(named)
}

/// Walks the book from the start position, naming every position it reaches.
fn walk(book: &Book, named: &HashMap<u64, Named>) -> (Vec<(u64, String)>, String) {
    let moves_by_hash: HashMap<u64, Vec<_>> = book
        .positions()
        .map(|(hash, mvs)| (hash, mvs.iter().map(|bm| bm.mv).collect()))
        .collect();

    // The best name known for a position, and how deep the line naming it
    // ran. A position is re-expanded only when a better name reaches it, so
    // this doubles as the visited set.
    let mut best: HashMap<u64, Named> = HashMap::new();
    // Positions reached before any named line. They have no name to compare,
    // so `best` cannot bound their re-expansion and a transposition back into
    // one would loop.
    let mut seen_unnamed: HashSet<u64> = HashSet::new();
    // The shortest distance from the start at which each position is reached.
    // The queue is breadth-first, so the first arrival is that distance, and
    // a later one that transposed around is not.
    let mut shortest: HashMap<u64, usize> = HashMap::new();
    // Depth travels with the position because a line can transpose back to
    // one it already passed: 1.Nf3 Nf6 2.Ng1 Ng8 returns to the start, and
    // without the depth the start position would inherit the name of an
    // opening it precedes.
    let mut queue: VecDeque<(Board, Option<String>, usize, usize)> = VecDeque::new();
    queue.push_back((Board::start_pos(), None, 0, 0));

    let mut reached = 0usize;
    while let Some((board, carried, carried_plies, depth)) = queue.pop_front() {
        let hash = board.hash();
        // Only a position with book moves of its own can ever be looked up:
        // a book hit reports the position it played *from*. Positions past
        // the last book move are reached by the walk and named by nothing.
        if !moves_by_hash.contains_key(&hash) {
            continue;
        }
        let shortest_depth = *shortest.entry(hash).or_insert(depth);
        let own = named.get(&hash);
        let Some((name, plies)) = own
            .map(|own| (own.name.clone(), own.plies))
            // A name from a k-ply line describes the position that line
            // reaches, so it cannot describe a position closer than k plies
            // to the start however the walk arrived at it.
            .or_else(|| {
                carried
                    .filter(|_| carried_plies <= shortest_depth)
                    .map(|name| (name, carried_plies))
            })
        else {
            // Before any named line: nothing to inherit yet.
            if seen_unnamed.insert(hash) {
                for mv in moves_by_hash.get(&hash).into_iter().flatten() {
                    queue.push_back((board.make_move(*mv), None, 0, depth + 1));
                }
            }
            continue;
        };

        if let Some(held) = best.get(&hash) {
            if plies <= held.plies {
                continue;
            }
        } else {
            reached += 1;
        }
        best.insert(
            hash,
            Named {
                name: name.clone(),
                plies,
            },
        );

        for mv in moves_by_hash.get(&hash).into_iter().flatten() {
            queue.push_back((board.make_move(*mv), Some(name.clone()), plies, depth + 1));
        }
    }

    let total = moves_by_hash.len();
    // Percent in integer arithmetic: a float here would need a cast the lint
    // table denies, for a number that is only ever printed.
    let percent = (reached * 100).checked_div(total).unwrap_or(0);
    let report = format!(
        "data set names:   {}\nbook positions:   {total}\nnamed positions:  {reached} ({percent}%)\n",
        named.len(),
    );

    let table = best
        .into_iter()
        .map(|(hash, held)| (hash, held.name))
        .collect();
    (table, report)
}
