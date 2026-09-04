//! Formatting UCI protocol responses, free of I/O.
//!
//! `Response`'s `Display` impl produces exactly the line to write, the same way
//! `command::parse` stays pure on the input side. `super::session` is the only piece that
//! actually writes to stdout.

use crate::eval::Score;
use crate::search::tt::Tt;
use crate::search::MATE;
use crate::types::Move;
use std::fmt;
use std::time::Duration;

/// One line of UCI output.
///
/// `IdName`/`IdAuthor` carry no data: there's exactly one correct value for
/// each (this engine's own name and author), not something a caller
/// supplies per call, so they're baked into `Display` directly rather than
/// threaded through as fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// `id name turox`.
    IdName,
    /// `id author Carson Price`.
    IdAuthor,
    /// `option name Hash type spin default <n> min <n> max <n>`: advertises the
    /// transposition table size, in MB, a GUI can configure via `setoption`. The actual
    /// numbers come from `Tt::DEFAULT_HASH_MB`/`MIN_HASH_MB`/`MAX_HASH_MB`, not hardcoded
    /// here, so what this line advertises and what `uci::session::run` actually enforces
    /// on `setoption name Hash value <n>` can't drift apart. Carries no data, like
    /// `IdName`/`IdAuthor`: there's exactly one option to advertise right now, not
    /// something a caller supplies per call.
    OptionHash,
    /// `uciok`: done identifying, ready to receive commands.
    UciOk,
    /// `readyok`: reply to `isready`.
    ReadyOk,
    /// `bestmove <move>`, or the `0000` null-move convention when there's
    /// no move to make (the position `search` was given was already
    /// checkmate or stalemate).
    BestMove(Option<Move>),
    /// `info depth <d> score cp <s>|mate <n> nodes <n> [pv <move> ...]`.
    /// `pv` is whatever principal variation the caller has on hand, not
    /// something this type computes; an empty `pv` omits that field from
    /// the line entirely rather than emitting a trailing `pv` with nothing
    /// after it. `score` is side-to-move-relative, the same convention
    /// `search::SearchResult::score` already uses.
    Info {
        /// Search depth this result came from.
        depth: u8,
        /// Side-to-move-relative score, `SearchResult::score`'s own
        /// convention.
        score: Score,
        /// Total nodes searched.
        nodes: u64,
        /// Wall-clock time this search has taken. Emitted in milliseconds, and
        /// also what `nps` is derived from, so the two can never disagree the
        /// way two independently-passed fields could.
        time: Duration,
        /// Transposition table occupancy in permille, or `None` when the search
        /// had no table, in which case the field is omitted entirely rather
        /// than reported as zero.
        hashfull: Option<u16>,
        /// The principal variation, root move first.
        pv: Vec<Move>,
    },
}

/// Nodes per second, the `nps` info field.
///
/// Reports `nodes` unchanged when no measurable time has passed rather than
/// dividing by zero: the first iteration routinely completes inside a
/// millisecond, and `Duration::as_secs_f64` would make that an infinity that
/// formats as `inf` and confuses a GUI. Treating it as "this many nodes in
/// about a second" understates the real rate, which is the safe direction for
/// a diagnostic number.
fn nps(nodes: u64, time: Duration) -> u64 {
    let micros = u64::try_from(time.as_micros()).unwrap_or(u64::MAX);
    if micros == 0 {
        return nodes;
    }
    nodes.saturating_mul(1_000_000) / micros
}

/// UCI's two mutually exclusive ways to report a score: an ordinary
/// evaluation in centipawns, or a forced mate `moves` full moves away
/// (UCI counts *moves*, not plies: `mate 1` means the side to move mates on
/// their very next move, `mate 2` means it takes two of their own moves
/// with one forced reply in between). Positive when the side the score is
/// relative to delivers the mate, negative when they're the one being
/// mated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScoreKind {
    Cp(Score),
    Mate(Score),
}

/// Classifies a side-to-move-relative score (`SearchResult::score`'s own
/// convention) as an ordinary centipawn evaluation or a mate distance.
///
/// A genuine mate score is always within a few hundred of `MATE` (`search`'s
/// own `ply as Score - MATE`/`MATE - ply` formula, documented on `MATE`
/// itself), however deep the search went; an ordinary material/positional
/// evaluation never gets remotely close to that magnitude. `MATE / 2` as
/// the cutoff between the two is generous in both directions: no realistic
/// evaluation swing approaches it, and no realistic search depth produces a
/// mate distance anywhere near it either.
///
/// Converting a mate *score* into a mate *move count* reuses the same ply
/// arithmetic `MATE`'s own doc lays out, just run in reverse: recover the
/// ply distance from `MATE - score.abs()`, then convert plies to full moves
/// (UCI's unit, not this engine's own). Get the rounding direction right,
/// worth checking against a concrete case rather than trusting it by
/// inspection: `tests/uci_response.rs` pins specific mate scores
/// already confirmed correct by `search`'s own mate-puzzle tests
/// (`MATE - 1`, `MATE - 3`, ...) to their expected `mate N` output.
const fn classify_score(score: Score) -> ScoreKind {
    if score.abs() > MATE / 2 {
        let ply = MATE - score.abs();
        let moves = (ply + 1) / 2;
        ScoreKind::Mate(score.signum() * moves)
    } else {
        ScoreKind::Cp(score)
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdName => write!(f, "id name turox"),
            Self::IdAuthor => write!(f, "id author Carson Price"),
            Self::OptionHash => write!(
                f,
                "option name Hash type spin default {} min {} max {}",
                Tt::DEFAULT_HASH_MB,
                Tt::MIN_HASH_MB,
                Tt::MAX_HASH_MB
            ),
            Self::UciOk => write!(f, "uciok"),
            Self::ReadyOk => write!(f, "readyok"),
            Self::BestMove(m) => {
                write!(f, "bestmove ")?;
                match m {
                    None => write!(f, "0000"),
                    Some(m) => write!(f, "{}", m.to_uci()),
                }
            }
            Self::Info {
                depth,
                score,
                nodes,
                time,
                hashfull,
                pv,
            } => {
                write!(f, "info depth {depth} score ")?;
                let score_kind = classify_score(*score);
                match score_kind {
                    ScoreKind::Cp(score) => write!(f, "cp {score}"),
                    ScoreKind::Mate(moves) => write!(f, "mate {moves}"),
                }?;
                let millis = u64::try_from(time.as_millis()).unwrap_or(u64::MAX);
                write!(f, " nodes {nodes} nps {} ", nps(*nodes, *time))?;
                if let Some(permille) = hashfull {
                    write!(f, "hashfull {permille} ")?;
                }
                write!(f, "time {millis}")?;
                if !pv.is_empty() {
                    write!(f, " pv")?;
                    for m in pv {
                        write!(f, " {}", m.to_uci())?;
                    }
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MoveFlags, Square};

    #[test]
    fn ordinary_evaluations_classify_as_centipawns() {
        for cp in [0, 1, 150, -320, 900, -900] {
            assert_eq!(classify_score(cp), ScoreKind::Cp(cp), "score {cp}");
        }
    }

    /// One ply-distance table, both signs, checked against `MATE - ply`/
    /// `ply - MATE` directly (the exact formula `MATE`'s own doc and
    /// `search`'s mate-scoring both use) rather than a hand-picked score
    /// per case, so the *mapping* from ply to UCI's move count is what's
    /// under test, not just a handful of disconnected magic numbers.
    /// `(1, 1)`/`(3, 2)` are `search`'s own already-verified mate-in-1 and
    /// Philidor's Legacy mate-in-2 scores, not invented here.
    #[test]
    fn mate_scores_convert_ply_distance_to_uci_move_count() {
        let ply_to_moves = [
            (0, 0),
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
            (6, 3),
            (7, 4),
        ];
        for (ply, moves) in ply_to_moves {
            let delivering = MATE - ply;
            let suffering = -(MATE - ply);
            assert_eq!(
                classify_score(delivering),
                ScoreKind::Mate(moves),
                "ply {ply} (delivering)"
            );
            assert_eq!(
                classify_score(suffering),
                ScoreKind::Mate(-moves),
                "ply {ply} (suffering)"
            );
        }
    }

    fn quiet(from: Square, to: Square) -> Move {
        Move::new(from, to, MoveFlags::Quiet)
    }

    #[test]
    fn id_name_and_author() {
        assert_eq!(Response::IdName.to_string(), "id name turox");
        assert_eq!(Response::IdAuthor.to_string(), "id author Carson Price");
    }

    #[test]
    fn option_hash() {
        assert_eq!(
            Response::OptionHash.to_string(),
            "option name Hash type spin default 16 min 1 max 1024"
        );
    }

    #[test]
    fn uciok_and_readyok() {
        assert_eq!(Response::UciOk.to_string(), "uciok");
        assert_eq!(Response::ReadyOk.to_string(), "readyok");
    }

    #[test]
    fn bestmove_with_a_move() {
        let m = quiet(Square::E2, Square::E4);
        assert_eq!(Response::BestMove(Some(m)).to_string(), "bestmove e2e4");
    }

    #[test]
    fn bestmove_with_no_legal_move_is_the_null_move() {
        assert_eq!(Response::BestMove(None).to_string(), "bestmove 0000");
    }

    #[test]
    fn bestmove_with_a_promotion() {
        let m = Move::new(Square::E7, Square::E8, MoveFlags::PromoteQueen);
        assert_eq!(Response::BestMove(Some(m)).to_string(), "bestmove e7e8q");
    }

    /// All four corners get their own check rather than trusting symmetry.
    /// `to()` already being the king's real destination (not the rook's
    /// square) is confirmed directly against `legal_moves` elsewhere; these
    /// only need to confirm `bestmove`'s formatting doesn't reintroduce a
    /// special case that undoes that.
    #[test]
    fn bestmove_with_castling_all_four_corners() {
        let cases = [
            (
                Square::E1,
                Square::G1,
                MoveFlags::KingCastle,
                "bestmove e1g1",
            ),
            (
                Square::E1,
                Square::C1,
                MoveFlags::QueenCastle,
                "bestmove e1c1",
            ),
            (
                Square::E8,
                Square::G8,
                MoveFlags::KingCastle,
                "bestmove e8g8",
            ),
            (
                Square::E8,
                Square::C8,
                MoveFlags::QueenCastle,
                "bestmove e8c8",
            ),
        ];
        for (from, to, flags, expected) in cases {
            let m = Move::new(from, to, flags);
            assert_eq!(Response::BestMove(Some(m)).to_string(), expected);
        }
    }

    #[test]
    fn info_with_an_ordinary_score_and_no_pv() {
        let response = Response::Info {
            depth: 5,
            score: 150,
            nodes: 12345,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: Vec::new(),
        };
        assert_eq!(
            response.to_string(),
            "info depth 5 score cp 150 nodes 12345 nps 12345 time 0"
        );
    }

    #[test]
    fn info_with_a_negative_score() {
        let response = Response::Info {
            depth: 4,
            score: -320,
            nodes: 999,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: Vec::new(),
        };
        assert_eq!(
            response.to_string(),
            "info depth 4 score cp -320 nodes 999 nps 999 time 0"
        );
    }

    #[test]
    fn info_with_a_principal_variation() {
        let response = Response::Info {
            depth: 3,
            score: 20,
            nodes: 500,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: vec![quiet(Square::E2, Square::E4), quiet(Square::E7, Square::E5)],
        };
        assert_eq!(
            response.to_string(),
            "info depth 3 score cp 20 nodes 500 nps 500 time 0 pv e2e4 e7e5"
        );
    }

    /// Same mate-in-1 score `search`'s own `white_delivers_mate_in_one`
    /// test produces, formatted the way UCI expects: `mate 1`, not
    /// `cp 29999` or some other leftover of the internal `MATE`-relative
    /// representation leaking into the wire format.
    #[test]
    fn info_with_a_mate_score_delivering() {
        let response = Response::Info {
            depth: 1,
            score: MATE - 1,
            nodes: 15,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: vec![quiet(Square::A1, Square::A8)],
        };
        assert_eq!(
            response.to_string(),
            "info depth 1 score mate 1 nodes 15 nps 15 time 0 pv a1a8"
        );
    }

    /// Same shape, the side to move about to be mated: negative `mate N`.
    #[test]
    fn info_with_a_mate_score_suffering() {
        let response = Response::Info {
            depth: 2,
            score: -(MATE - 2),
            nodes: 40,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: Vec::new(),
        };
        assert_eq!(
            response.to_string(),
            "info depth 2 score mate -1 nodes 40 nps 40 time 0"
        );
    }

    /// Same mate-in-2 score `search`'s own `philidors_legacy_smothered_mate`
    /// test produces.
    #[test]
    fn info_with_a_deeper_mate_score() {
        let response = Response::Info {
            depth: 3,
            score: MATE - 3,
            nodes: 200,
            time: Duration::from_millis(0),
            hashfull: None,
            pv: vec![quiet(Square::E6, Square::G8)],
        };
        assert_eq!(
            response.to_string(),
            "info depth 3 score mate 2 nodes 200 nps 200 time 0 pv e6g8"
        );
    }

    /// `nps` is derived from `nodes` and `time` rather than passed in, so this
    /// pins the arithmetic rather than the plumbing: 2000 nodes in half a
    /// second is 4000 per second.
    #[test]
    fn info_derives_nps_from_nodes_and_elapsed_time() {
        let response = Response::Info {
            depth: 7,
            score: 30,
            nodes: 2000,
            time: Duration::from_millis(500),
            hashfull: None,
            pv: vec![],
        };
        assert_eq!(
            response.to_string(),
            "info depth 7 score cp 30 nodes 2000 nps 4000 time 500"
        );
    }

    /// The first iteration routinely finishes inside a microsecond, where a
    /// naive divide would produce an infinity that formats as `inf`. Reporting
    /// the raw node count understates the rate, which is the safe direction.
    #[test]
    fn info_does_not_divide_by_zero_when_no_time_has_elapsed() {
        assert_eq!(nps(41, Duration::ZERO), 41);
    }

    /// `None` omits the field entirely rather than emitting `hashfull 0`: a
    /// search with no table and a search with an empty one are different
    /// states, and only the latter is worth reporting as zero.
    #[test]
    fn info_omits_hashfull_when_the_search_had_no_table() {
        let response = Response::Info {
            depth: 2,
            score: 0,
            nodes: 10,
            time: Duration::from_millis(1),
            hashfull: None,
            pv: vec![],
        };
        assert!(
            !response.to_string().contains("hashfull"),
            "no table means the field is absent, got: {response}"
        );
    }

    #[test]
    fn info_emits_hashfull_in_permille_when_a_table_is_present() {
        let response = Response::Info {
            depth: 2,
            score: 0,
            nodes: 10,
            time: Duration::from_millis(1),
            hashfull: Some(37),
            pv: vec![],
        };
        assert_eq!(
            response.to_string(),
            "info depth 2 score cp 0 nodes 10 nps 10000 hashfull 37 time 1"
        );
    }
}
