//! Concrete tests for `san::resolve_san`.

use bookgen::san::resolve_san;
use turox_engine::board::Board;
use turox_engine::{Move, MoveFlags, Square};

fn fen(s: &str) -> Board {
    Board::try_from_fen(s).unwrap_or_else(|e| panic!("invalid test FEN {s:?}: {e:?}"))
}

#[test]
fn resolves_a_simple_pawn_push() {
    let board = Board::start_pos();
    assert_eq!(
        resolve_san(&board, "e4"),
        Some(Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush))
    );
}

#[test]
fn resolves_a_simple_piece_move() {
    let board = Board::start_pos();
    assert_eq!(
        resolve_san(&board, "Nf3"),
        Some(Move::new(Square::G1, Square::F3, MoveFlags::Quiet))
    );
}

#[test]
fn resolves_a_pawn_capture() {
    let board = fen("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1");
    assert_eq!(
        resolve_san(&board, "exd5"),
        Some(Move::new(Square::E4, Square::D5, MoveFlags::Capture))
    );
}

#[test]
fn resolves_file_disambiguated_knight_moves() {
    // White knights on c3 and g3 can both reach e4.
    let board = fen("4k3/8/8/8/8/2N3N1/8/4K3 w - - 0 1");
    assert_eq!(
        resolve_san(&board, "Nce4"),
        Some(Move::new(Square::C3, Square::E4, MoveFlags::Quiet))
    );
    assert_eq!(
        resolve_san(&board, "Nge4"),
        Some(Move::new(Square::G3, Square::E4, MoveFlags::Quiet))
    );
}

#[test]
fn resolves_rank_disambiguated_knight_moves() {
    // White knights on d1 and d5 (same file) can both reach c3.
    let board = fen("4k3/8/8/3N4/8/8/8/3NK3 w - - 0 1");
    assert_eq!(
        resolve_san(&board, "N1c3"),
        Some(Move::new(Square::D1, Square::C3, MoveFlags::Quiet))
    );
    assert_eq!(
        resolve_san(&board, "N5c3"),
        Some(Move::new(Square::D5, Square::C3, MoveFlags::Quiet))
    );
}

#[test]
fn an_ambiguous_token_with_no_disambiguator_resolves_to_none() {
    // Same position as the rank-disambiguation test, but the bare token
    // doesn't say which knight; a real game's PGN would never emit this,
    // but resolution must refuse to guess rather than pick one silently.
    let board = fen("4k3/8/8/3N4/8/8/8/3NK3 w - - 0 1");
    assert_eq!(resolve_san(&board, "Nc3"), None);
}

#[test]
fn resolves_a_promotion() {
    let board = fen("k7/4P3/8/8/8/8/8/4K3 w - - 0 1");
    assert_eq!(
        resolve_san(&board, "e8=Q"),
        Some(Move::new(Square::E7, Square::E8, MoveFlags::PromoteQueen))
    );
}

#[test]
fn resolves_a_promotion_with_capture() {
    let board = fen("k2r4/4P3/8/8/8/8/8/4K3 w - - 0 1");
    assert_eq!(
        resolve_san(&board, "exd8=Q"),
        Some(Move::new(
            Square::E7,
            Square::D8,
            MoveFlags::PromoteCaptureQueen
        ))
    );
}

#[test]
fn resolves_both_castling_sides() {
    let board = fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1");
    assert_eq!(
        resolve_san(&board, "O-O"),
        Some(Move::new(Square::E1, Square::G1, MoveFlags::KingCastle))
    );
    assert_eq!(
        resolve_san(&board, "O-O-O"),
        Some(Move::new(Square::E1, Square::C1, MoveFlags::QueenCastle))
    );
}

#[test]
fn strips_check_and_mate_suffixes_without_changing_the_resolved_move() {
    let board = Board::start_pos();
    let plain = resolve_san(&board, "e4");
    assert_eq!(resolve_san(&board, "e4+"), plain);
    assert_eq!(resolve_san(&board, "e4#"), plain);
}

#[test]
fn a_malformed_token_resolves_to_none() {
    let board = Board::start_pos();
    assert_eq!(resolve_san(&board, ""), None);
    assert_eq!(resolve_san(&board, "e4e5"), None);
    assert_eq!(resolve_san(&board, "Z9"), None);
}

#[test]
fn a_syntactically_valid_but_illegal_move_resolves_to_none() {
    // Well-formed SAN naming a real piece and a real square, but neither
    // rook can reach a5 in one move from startpos (both are blocked by
    // their own pawns and not on a5's rank or file).
    let board = Board::start_pos();
    assert_eq!(resolve_san(&board, "Ra5"), None);
}
