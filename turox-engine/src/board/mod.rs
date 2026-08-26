//! The board: piece placement plus the rest of a chess position's state (side to
//! move, castling rights, en passant target, move clocks).

use crate::types::{
    Bitboard, CastlingRights, Color, ColoredPiece, File, Move, MoveFlags, Piece, Rank, Square,
};
use crate::Direction;
use std::fmt;
use std::ops::Index;

pub mod error;
pub mod fen;
pub mod zobrist;

pub use error::InvalidFenError;

/// A chess position.
///
/// `Copy` and ~136 bytes (8 bitboards + a 64-byte mailbox + game state), built for
/// copy-make: `make_move` takes `&self` and returns a new `Board` rather than
/// mutating in place plus an undo stack. No undo-state bugs, and it stays trivially
/// parallel if lazy SMP search happens later.
///
/// The mailbox (`piece_at` in O(1)) exists alongside the bitboards specifically
/// because captures, SEE, and eval all want "what's on this square" far more often
/// than "where are all the knights" — paying 64 bytes to make that O(1) instead of a
/// 6-bitboard scan is worth it. `ColoredPiece` being a single `repr(u8)` enum rather
/// than a `{ color, piece }` struct is what keeps `Option<ColoredPiece>` to 1 byte
/// (a niche in the 12..=255 range) instead of 2, halving this array from 128 bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    by_color: [Bitboard; 2],
    by_piece: [Bitboard; 6],
    mailbox: [Option<ColoredPiece>; 64],
    side_to_move: Color,
    castling: CastlingRights,
    en_passant: Option<Square>,
    halfmove_clock: u8,
    fullmove_number: u16,
}

impl Default for Board {
    /// An empty board, White to move, no castling rights, move 1. Not a legal
    /// position on its own — use `start_pos` or `try_from_fen` for one.
    fn default() -> Self {
        Self {
            by_color: [Bitboard::EMPTY; 2],
            by_piece: [Bitboard::EMPTY; 6],
            mailbox: [None; 64],
            side_to_move: Color::White,
            castling: CastlingRights::NONE,
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
        }
    }
}

impl Board {
    /// The standard chess starting position.
    pub fn start_pos() -> Self {
        let mut board = Self::default();
        const BACK_RANK: [Piece; 8] = [
            Piece::Rook,
            Piece::Knight,
            Piece::Bishop,
            Piece::Queen,
            Piece::King,
            Piece::Bishop,
            Piece::Knight,
            Piece::Rook,
        ];
        for (file, &piece) in File::ALL.iter().zip(BACK_RANK.iter()) {
            board.place(
                Square::new(*file, Rank::R1),
                ColoredPiece::new(Color::White, piece),
            );
            board.place(
                Square::new(*file, Rank::R2),
                ColoredPiece::new(Color::White, Piece::Pawn),
            );
            board.place(
                Square::new(*file, Rank::R7),
                ColoredPiece::new(Color::Black, Piece::Pawn),
            );
            board.place(
                Square::new(*file, Rank::R8),
                ColoredPiece::new(Color::Black, piece),
            );
        }
        board.castling = CastlingRights::ALL;
        board
    }

    /// Places `cp` on `sq`, keeping the mailbox and the bitboards in sync. Does not
    /// check whether `sq` was already occupied — callers that care (e.g. FEN
    /// parsing rejecting overlapping pieces) should check `piece_at` first.
    ///
    /// This is the single path every piece placement should go through, so the
    /// mailbox/bitboard consistency invariant (checked by the `board_consistency`
    /// property test) can't be broken by construction.
    pub fn place(&mut self, sq: Square, cp: ColoredPiece) {
        self.mailbox[sq.index() as usize] = Some(cp);
        self.by_color[cp.color() as usize] = self.by_color[cp.color() as usize].or(sq.bitboard());
        self.by_piece[cp.piece() as usize] = self.by_piece[cp.piece() as usize].or(sq.bitboard());
    }

    /// Builds a full `Board` from a placement-only board (assembled via
    /// `Board::default()` plus repeated `place` calls) and the remaining
    /// position state. The single path every non-placement field goes through:
    /// `try_from_fen` builds its result this way, and so can anything else that
    /// wants a specific side-to-move/castling/en-passant/clock combination
    /// without hand-assembling a FEN string first (test helpers, in particular).
    pub fn from_parts(
        placement: Board,
        side_to_move: Color,
        castling: CastlingRights,
        en_passant: Option<Square>,
        halfmove_clock: u8,
        fullmove_number: u16,
    ) -> Board {
        Board {
            side_to_move,
            castling,
            en_passant,
            halfmove_clock,
            fullmove_number,
            ..placement
        }
    }

    /// Removes and returns whatever was on `sq`, or `None` if it was already empty.
    pub fn remove(&mut self, sq: Square) -> Option<ColoredPiece> {
        let cp = self.mailbox[sq.index() as usize].take()?;
        self.by_color[cp.color() as usize] =
            self.by_color[cp.color() as usize].and_not(sq.bitboard());
        self.by_piece[cp.piece() as usize] =
            self.by_piece[cp.piece() as usize].and_not(sq.bitboard());
        Some(cp)
    }

    /// O(1) lookup of whatever is on `sq`, via the mailbox.
    pub fn piece_at(&self, sq: Square) -> Option<ColoredPiece> {
        self.mailbox[sq.index() as usize]
    }

    /// All pieces of a given color and kind.
    pub fn pieces(&self, color: Color, piece: Piece) -> Bitboard {
        self[piece].and(self[color])
    }

    /// Every occupied square, regardless of color or piece kind.
    pub fn occupied(&self) -> Bitboard {
        self[Color::White].or(self[Color::Black])
    }

    /// Every unoccupied square.
    pub fn empty(&self) -> Bitboard {
        !self.occupied()
    }

    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn castling_rights(&self) -> CastlingRights {
        self.castling
    }

    pub fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    pub fn halfmove_clock(&self) -> u8 {
        self.halfmove_clock
    }

    pub fn fullmove_number(&self) -> u16 {
        self.fullmove_number
    }

    /// Applies `m` to a copy of this position and returns the result (copy-make;
    /// see the struct docs). Stubbed pending move generation: this needs
    /// `Bitboard`'s scanning/shift primitives to be implemented first, and its own
    /// logic (captures, en passant, castling rook movement, promotion, clock
    /// resets) is the subject of the move-generation change.
    pub fn make_move(&self, m: Move) -> Board {
        let mut board = *self;
        let color = board.side_to_move();

        let moved_piece = board
            .remove(m.from())
            .expect("Move must have a piece on from square");
        let piece = moved_piece.piece();

        if piece == Piece::Pawn || m.flags().is_capture() {
            board.halfmove_clock = 0;
        } else {
            board.halfmove_clock += 1;
        }

        if piece == Piece::King {
            board.castling = board.castling_rights().without_color(color);
        }
        if m.from() == Square::A1 || m.to() == Square::A1 {
            board.castling = board
                .castling_rights()
                .without(CastlingRights::WHITE_QUEENSIDE);
        }
        if m.from() == Square::H1 || m.to() == Square::H1 {
            board.castling = board
                .castling_rights()
                .without(CastlingRights::WHITE_KINGSIDE);
        }
        if m.from() == Square::A8 || m.to() == Square::A8 {
            board.castling = board
                .castling_rights()
                .without(CastlingRights::BLACK_QUEENSIDE);
        }
        if m.from() == Square::H8 || m.to() == Square::H8 {
            board.castling = board
                .castling_rights()
                .without(CastlingRights::BLACK_KINGSIDE);
        }

        if m.flags() == MoveFlags::DoublePawnPush {
            let forward_dir = match color {
                Color::Black => Direction::South,
                Color::White => Direction::North,
            };
            let en_passant = m
                .from()
                .bitboard()
                .shift(forward_dir)
                .pop_lsb()
                .expect("one bit is set after the shift");
            board.en_passant = Some(en_passant);
        } else {
            board.en_passant = None;
        }

        match m.flags() {
            MoveFlags::Quiet => {
                board.place(m.to(), moved_piece);
            }
            MoveFlags::Capture => {
                board.remove(m.to());
                board.place(m.to(), moved_piece);
            }
            MoveFlags::PromoteBishop
            | MoveFlags::PromoteKnight
            | MoveFlags::PromoteRook
            | MoveFlags::PromoteQueen => {
                board.place(
                    m.to(),
                    ColoredPiece::new(
                        color,
                        m.flags().promotion_piece().expect("matched on promote"),
                    ),
                );
            }
            MoveFlags::PromoteCaptureBishop
            | MoveFlags::PromoteCaptureKnight
            | MoveFlags::PromoteCaptureRook
            | MoveFlags::PromoteCaptureQueen => {
                board.remove(m.to());
                board.place(
                    m.to(),
                    ColoredPiece::new(
                        color,
                        m.flags().promotion_piece().expect("matched on promote"),
                    ),
                );
            }
            MoveFlags::DoublePawnPush => {
                board.place(m.to(), moved_piece);
            }
            MoveFlags::KingCastle => {
                board.place(m.to(), moved_piece);
                match color {
                    Color::Black => {
                        let br = board.remove(Square::H8).expect("KingCastle must have rook");
                        board.place(Square::F8, br);
                    }
                    Color::White => {
                        let wr = board.remove(Square::H1).expect("KingCastle must have rook");
                        board.place(Square::F1, wr);
                    }
                }
            }
            MoveFlags::QueenCastle => {
                board.place(m.to(), moved_piece);
                match color {
                    Color::Black => {
                        let br = board
                            .remove(Square::A8)
                            .expect("QueenCastle must have a rook");
                        board.place(Square::D8, br);
                    }
                    Color::White => {
                        let wr = board
                            .remove(Square::A1)
                            .expect("QueenCastle must have a rook");
                        board.place(Square::D1, wr);
                    }
                }
            }
            MoveFlags::EnPassant => {
                let opp_forward_dir = match color.flip() {
                    Color::Black => Direction::South,
                    Color::White => Direction::North,
                };
                let en_passant_target = m
                    .to()
                    .bitboard()
                    .shift(opp_forward_dir)
                    .pop_lsb()
                    .expect("one bit is set after the shift");
                board.place(m.to(), moved_piece);
                board.remove(en_passant_target);
            }
        }
        if color == Color::Black {
            board.fullmove_number += 1;
        }
        board.side_to_move = color.flip();
        board
    }
}

impl Index<Color> for Board {
    type Output = Bitboard;
    fn index(&self, color: Color) -> &Bitboard {
        &self.by_color[color as usize]
    }
}

impl Index<Piece> for Board {
    type Output = Bitboard;
    fn index(&self, piece: Piece) -> &Bitboard {
        &self.by_piece[piece as usize]
    }
}

impl fmt::Debug for Board {
    /// A single 8x8 grid built from the mailbox. Deliberately not `#[derive(Debug)]`
    /// dumping all 8 bitboard fields separately — this is the representation
    /// actually useful for debugging a position, and (unlike `Bitboard`'s own
    /// `Debug`) doesn't depend on any of the still-unimplemented `Bitboard` methods,
    /// so it works today.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let sq = Square::ALL[rank * 8 + file];
                let ch = self.piece_at(sq).map_or('.', ColoredPiece::to_fen);
                write!(f, "{ch} ")?;
            }
            writeln!(f)?;
        }
        write!(f, "  a b c d e f g h")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MoveFlags;

    #[test]
    fn start_pos_places_expected_piece_counts() {
        let board = Board::start_pos();
        for sq in Square::ALL {
            let _ = board.piece_at(sq); // mailbox lookup never panics
        }
        let mut counts = [0u32; 12];
        for sq in Square::ALL {
            if let Some(cp) = board.piece_at(sq) {
                counts[cp as usize] += 1;
            }
        }
        assert_eq!(counts[ColoredPiece::WhitePawn as usize], 8);
        assert_eq!(counts[ColoredPiece::BlackPawn as usize], 8);
        assert_eq!(counts[ColoredPiece::WhiteKing as usize], 1);
        assert_eq!(counts[ColoredPiece::BlackKing as usize], 1);
    }

    #[test]
    fn start_pos_side_to_move_and_castling() {
        let board = Board::start_pos();
        assert_eq!(board.side_to_move(), Color::White);
        assert_eq!(board.castling_rights(), CastlingRights::ALL);
        assert_eq!(board.en_passant(), None);
        assert_eq!(board.fullmove_number(), 1);
    }

    #[test]
    fn place_then_piece_at_agrees() {
        let mut board = Board::default();
        board.place(Square::E4, ColoredPiece::WhiteKnight);
        assert_eq!(board.piece_at(Square::E4), Some(ColoredPiece::WhiteKnight));
        assert_eq!(board.piece_at(Square::E5), None);
    }

    #[test]
    fn remove_clears_mailbox() {
        let mut board = Board::default();
        board.place(Square::E4, ColoredPiece::WhiteKnight);
        assert_eq!(board.remove(Square::E4), Some(ColoredPiece::WhiteKnight));
        assert_eq!(board.piece_at(Square::E4), None);
        assert_eq!(board.remove(Square::E4), None);
    }

    #[test]
    fn start_pos_occupied_and_empty() {
        let board = Board::start_pos();
        assert_eq!(board.occupied().count(), 32);
        assert_eq!(board.occupied().and(board.empty()), Bitboard::EMPTY);
    }

    #[test]
    fn start_pos_one_king_per_side() {
        let board = Board::start_pos();
        for color in Color::ALL {
            assert_eq!(board.pieces(color, Piece::King).count(), 1);
        }
    }

    #[test]
    fn every_color_piece_pair_is_disjoint_and_covers_occupied() {
        assert_board_is_internally_consistent(&Board::start_pos());
    }

    /// Every (color, piece) bitboard pair is disjoint and their union is exactly
    /// `occupied()`, and the mailbox agrees with the bitboards at every square.
    /// Shared by the start-position check above and, more importantly, by
    /// `make_move_outputs_stay_internally_consistent` below — `place`/`remove`
    /// are unit-tested to keep this invariant individually, but that's a claim
    /// about them in isolation, not a guarantee that `make_move`'s specific
    /// sequence of place/remove calls across all 14 move types preserves it too.
    fn assert_board_is_internally_consistent(board: &Board) {
        let mut union = Bitboard::EMPTY;
        for color in Color::ALL {
            for piece in Piece::ALL {
                let bb = board.pieces(color, piece);
                assert_eq!(
                    bb.and(union),
                    Bitboard::EMPTY,
                    "overlap for {color:?}/{piece:?}"
                );
                union = union.or(bb);
            }
        }
        assert_eq!(union, board.occupied(), "bitboards don't cover occupied()");

        for sq in Square::ALL {
            let via_mailbox = board.piece_at(sq);
            let via_bitboards = Color::ALL.iter().find_map(|&color| {
                Piece::ALL
                    .iter()
                    .find(|&&piece| board.pieces(color, piece).contains(sq))
                    .map(|&piece| ColoredPiece::new(color, piece))
            });
            assert_eq!(
                via_mailbox, via_bitboards,
                "mailbox/bitboard mismatch at {sq:?}"
            );
        }
    }

    #[test]
    fn make_move_outputs_stay_internally_consistent() {
        // One representative move per structurally distinct code path (plain
        // placement, capture, castling's extra rook move, en passant's
        // off-destination capture, promotion), not all 14 flags — the invariant
        // is checked once per *shape* of place/remove sequence, which is what
        // could plausibly desync it.
        let cases = [
            (
                Board::start_pos(),
                Move::new(Square::G1, Square::F3, MoveFlags::Quiet),
            ),
            (
                Board::try_from_fen("8/8/8/8/8/5n2/8/6N1 w - - 0 1").expect("valid FEN"),
                Move::new(Square::G1, Square::F3, MoveFlags::Capture),
            ),
            (
                Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN"),
                Move::new(Square::E1, Square::G1, MoveFlags::KingCastle),
            ),
            (
                Board::try_from_fen("8/8/8/3pP3/8/8/8/8 w - d6 0 1").expect("valid FEN"),
                Move::new(Square::E5, Square::D6, MoveFlags::EnPassant),
            ),
            (
                Board::try_from_fen("3n4/4P3/8/8/8/8/8/8 w - - 0 1").expect("valid FEN"),
                Move::new(Square::E7, Square::D8, MoveFlags::PromoteCaptureRook),
            ),
        ];
        for (board, m) in cases {
            assert_board_is_internally_consistent(&board.make_move(m));
        }
    }

    // ---- make_move ----
    //
    // FEN-based scenario tests rather than proptest: without legal move
    // generation yet, there's no way to generate an arbitrary (position, legal
    // move) pair to check against an independent oracle, which is what proptest
    // needs to be worth it. Concrete positions per rule are the right tool here.

    #[test]
    fn quiet_move_relocates_piece_and_flips_side_to_move() {
        let board = Board::start_pos();
        let next = board.make_move(Move::new(Square::G1, Square::F3, MoveFlags::Quiet));

        assert_eq!(next.piece_at(Square::G1), None);
        assert_eq!(next.piece_at(Square::F3), Some(ColoredPiece::WhiteKnight));
        assert_eq!(next.side_to_move(), Color::Black);
        assert_eq!(next.halfmove_clock(), 1);
        assert_eq!(next.fullmove_number(), 1);
        assert_eq!(next.en_passant(), None);
        assert_eq!(next.castling_rights(), CastlingRights::ALL);
    }

    #[test]
    fn make_move_does_not_mutate_the_original_board() {
        let board = Board::start_pos();
        let _ = board.make_move(Move::new(Square::G1, Square::F3, MoveFlags::Quiet));

        assert_eq!(board.piece_at(Square::G1), Some(ColoredPiece::WhiteKnight));
        assert_eq!(board.piece_at(Square::F3), None);
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn double_pawn_push_sets_en_passant_target_and_resets_halfmove_clock() {
        let board = Board::start_pos();
        let next = board.make_move(Move::new(Square::E2, Square::E4, MoveFlags::DoublePawnPush));

        assert_eq!(next.piece_at(Square::E2), None);
        assert_eq!(next.piece_at(Square::E4), Some(ColoredPiece::WhitePawn));
        assert_eq!(next.en_passant(), Some(Square::E3));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn quiet_pawn_push_also_resets_halfmove_clock() {
        let board = Board::try_from_fen("8/8/8/8/4P3/8/8/8 w - - 12 7").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E4, Square::E5, MoveFlags::Quiet));

        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn unrelated_move_clears_a_previously_set_en_passant_target() {
        // Starts with an en passant target already set (as if the previous move
        // was a double push); this move is unrelated and not itself a double
        // push, so the target must be cleared, not carried over.
        let board = Board::try_from_fen("8/8/8/3pP3/8/8/8/4K3 w - d6 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::E2, MoveFlags::Quiet));

        assert_eq!(next.en_passant(), None);
    }

    #[test]
    fn pawn_capture_removes_captured_piece_and_resets_halfmove_clock() {
        let board =
            Board::try_from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 5 3")
                .expect("valid FEN");
        let next = board.make_move(Move::new(Square::E4, Square::D5, MoveFlags::Capture));

        assert_eq!(next.piece_at(Square::E4), None);
        assert_eq!(next.piece_at(Square::D5), Some(ColoredPiece::WhitePawn));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn non_pawn_capture_also_resets_halfmove_clock() {
        let board = Board::try_from_fen("8/8/8/8/8/5n2/8/6N1 w - - 7 5").expect("valid FEN");
        let next = board.make_move(Move::new(Square::G1, Square::F3, MoveFlags::Capture));

        assert_eq!(next.piece_at(Square::G1), None);
        assert_eq!(next.piece_at(Square::F3), Some(ColoredPiece::WhiteKnight));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn en_passant_capture_removes_the_pawn_behind_the_target_square() {
        // White pawn e5, black pawn d5 (just double-pushed), ep target d6.
        let board = Board::try_from_fen("8/8/8/3pP3/8/8/8/8 w - d6 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E5, Square::D6, MoveFlags::EnPassant));

        assert_eq!(next.piece_at(Square::E5), None, "mover's origin square");
        assert_eq!(
            next.piece_at(Square::D6),
            Some(ColoredPiece::WhitePawn),
            "mover lands on the ep target square"
        );
        assert_eq!(
            next.piece_at(Square::D5),
            None,
            "captured pawn removed from behind the target, not from the target itself"
        );
        assert_eq!(next.halfmove_clock(), 0);
        assert_eq!(
            next.en_passant(),
            None,
            "ep target only ever valid for one reply"
        );
    }

    #[test]
    fn castling_increments_halfmove_clock_like_any_other_non_capture_non_pawn_move() {
        // Castling is neither a capture nor a pawn move, so — same rule as any
        // other quiet non-pawn move — the clock increments, it doesn't reset.
        // The reset/increment logic is shared, generic code, already checked
        // elsewhere, but the castling-rights corner mapping looked shared and
        // obviously-correct too, right up until it wasn't — so this gets its own
        // direct check instead of assuming the shared path transfers cleanly.
        let board =
            Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 11 6").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::G1, MoveFlags::KingCastle));

        assert_eq!(next.halfmove_clock(), 12);
    }

    #[test]
    fn kingside_castle_relocates_the_rook_and_clears_both_rights_for_that_color() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::G1, MoveFlags::KingCastle));

        assert_eq!(next.piece_at(Square::E1), None);
        assert_eq!(next.piece_at(Square::G1), Some(ColoredPiece::WhiteKing));
        assert_eq!(next.piece_at(Square::H1), None, "rook's origin");
        assert_eq!(next.piece_at(Square::F1), Some(ColoredPiece::WhiteRook));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn queenside_castle_relocates_the_rook_and_clears_both_rights_for_that_color() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E8, Square::C8, MoveFlags::QueenCastle));

        assert_eq!(next.piece_at(Square::E8), None);
        assert_eq!(next.piece_at(Square::C8), Some(ColoredPiece::BlackKing));
        assert_eq!(next.piece_at(Square::A8), None, "rook's origin");
        assert_eq!(next.piece_at(Square::D8), Some(ColoredPiece::BlackRook));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert_eq!(next.side_to_move(), Color::White);
        assert_eq!(next.fullmove_number(), 2, "increments after Black's move");
    }

    #[test]
    fn rook_move_clears_only_that_corners_right() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::A1, Square::A2, MoveFlags::Quiet));

        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn king_move_clears_both_rights_even_without_castling() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::E2, MoveFlags::Quiet));

        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn capturing_a_rook_on_its_corner_clears_that_sides_right() {
        // Black knight captures the still-unmoved white rook on h1: White's own
        // king/rook never moved this game, but the right is gone regardless.
        let board =
            Board::try_from_fen("r3k2r/8/8/8/8/6n1/8/R3K2R b KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::G3, Square::H1, MoveFlags::Capture));

        assert_eq!(next.piece_at(Square::H1), Some(ColoredPiece::BlackKnight));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn move_touching_two_different_corners_at_once_clears_both_rights() {
        // A single move's `from` and `to` can each independently be a different
        // corner: a queen on a1 capturing a rook on h8 (the long diagonal, fully
        // legal) should clear White's queenside right (via `from`) *and* Black's
        // kingside right (via `to`) in the same move. An `else if` chain over the
        // four corner checks would only ever apply one of the two; four
        // independent `if`s pointing at the wrong CastlingRights constant for a
        // given corner is a second, separate way to fail the same check.
        let board = Board::try_from_fen("7r/8/8/8/8/8/8/Q3K3 w Qk - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::A1, Square::H8, MoveFlags::Capture));

        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
    }

    #[test]
    fn king_capture_clears_both_rights_not_just_king_quiet_move() {
        // Same rule as king_move_clears_both_rights_even_without_castling, but via
        // MoveFlags::Capture instead of Quiet — the exact combination that was
        // missing when the King check lived inside the Quiet arm only.
        let board =
            Board::try_from_fen("r3k2r/8/8/8/4n3/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::E4, MoveFlags::Capture));

        assert_eq!(next.piece_at(Square::E4), Some(ColoredPiece::WhiteKing));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn queenside_castle_white_relocates_the_rook() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E1, Square::C1, MoveFlags::QueenCastle));

        assert_eq!(next.piece_at(Square::E1), None);
        assert_eq!(next.piece_at(Square::C1), Some(ColoredPiece::WhiteKing));
        assert_eq!(next.piece_at(Square::A1), None, "rook's origin");
        assert_eq!(next.piece_at(Square::D1), Some(ColoredPiece::WhiteRook));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
    }

    #[test]
    fn kingside_castle_black_relocates_the_rook() {
        let board = Board::try_from_fen("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E8, Square::G8, MoveFlags::KingCastle));

        assert_eq!(next.piece_at(Square::E8), None);
        assert_eq!(next.piece_at(Square::G8), Some(ColoredPiece::BlackKing));
        assert_eq!(next.piece_at(Square::H8), None, "rook's origin");
        assert_eq!(next.piece_at(Square::F8), Some(ColoredPiece::BlackRook));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
    }

    #[test]
    fn double_pawn_push_black_sets_the_en_passant_target_behind_it() {
        // Black moves "down" the board (decreasing rank), so the jumped-over
        // square is one rank *below* `to`, not above — the opposite of White's
        // case in double_pawn_push_sets_en_passant_target_and_resets_halfmove_clock.
        // A color-blind "always the rank above from" implementation gets this
        // backward for Black specifically.
        let board = Board::try_from_fen("8/3p4/8/8/8/8/8/8 b - - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::D7, Square::D5, MoveFlags::DoublePawnPush));

        assert_eq!(next.piece_at(Square::D5), Some(ColoredPiece::BlackPawn));
        assert_eq!(next.en_passant(), Some(Square::D6));
    }

    #[test]
    fn en_passant_capture_black() {
        // Black pawn d4, white pawn e4 (just double-pushed), ep target e3.
        let board = Board::try_from_fen("8/8/8/8/3pP3/8/8/8 b - e3 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::D4, Square::E3, MoveFlags::EnPassant));

        assert_eq!(next.piece_at(Square::D4), None);
        assert_eq!(next.piece_at(Square::E3), Some(ColoredPiece::BlackPawn));
        assert_eq!(
            next.piece_at(Square::E4),
            None,
            "captured pawn removed from behind the target, not from the target itself"
        );
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn promotion_black() {
        let board = Board::try_from_fen("8/8/8/8/8/8/4p3/8 b - - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E2, Square::E1, MoveFlags::PromoteQueen));

        assert_eq!(next.piece_at(Square::E2), None);
        assert_eq!(next.piece_at(Square::E1), Some(ColoredPiece::BlackQueen));
    }

    #[test]
    fn underpromotion_to_knight_and_bishop() {
        // Queen promotion is the common case; knight/bishop use the exact same
        // mechanism but are the more usual place for an off-by-one in the
        // MoveFlags -> Piece mapping to hide, since they're rarely exercised.
        let board = Board::try_from_fen("8/4P3/8/8/8/8/8/8 w - - 0 1").expect("valid FEN");
        let knight = board.make_move(Move::new(Square::E7, Square::E8, MoveFlags::PromoteKnight));
        assert_eq!(knight.piece_at(Square::E8), Some(ColoredPiece::WhiteKnight));

        let bishop = board.make_move(Move::new(Square::E7, Square::E8, MoveFlags::PromoteBishop));
        assert_eq!(bishop.piece_at(Square::E8), Some(ColoredPiece::WhiteBishop));
    }

    #[test]
    fn promotion_capturing_a_corner_rook_clears_that_sides_castling_right() {
        // White pawn on b7 promotes by capturing Black's still-unmoved rook on a8.
        let board =
            Board::try_from_fen("r3k2r/1P6/8/8/8/8/8/R3K2R w KQkq - 0 1").expect("valid FEN");
        let next = board.make_move(Move::new(
            Square::B7,
            Square::A8,
            MoveFlags::PromoteCaptureQueen,
        ));

        assert_eq!(next.piece_at(Square::A8), Some(ColoredPiece::WhiteQueen));
        assert!(!next
            .castling_rights()
            .contains(CastlingRights::BLACK_QUEENSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::BLACK_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_KINGSIDE));
        assert!(next
            .castling_rights()
            .contains(CastlingRights::WHITE_QUEENSIDE));
    }

    #[test]
    fn promotion_replaces_the_pawn_with_the_promoted_piece() {
        // Nonzero starting halfmove_clock: a promotion is a pawn move, so it must
        // reset to 0. Starting from 0 would pass even if the code never touched
        // the clock at all.
        let board = Board::try_from_fen("8/4P3/8/8/8/8/8/8 w - - 9 5").expect("valid FEN");
        let next = board.make_move(Move::new(Square::E7, Square::E8, MoveFlags::PromoteQueen));

        assert_eq!(next.piece_at(Square::E7), None);
        assert_eq!(next.piece_at(Square::E8), Some(ColoredPiece::WhiteQueen));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn promotion_with_capture_removes_captured_piece_and_promotes() {
        let board = Board::try_from_fen("3n4/4P3/8/8/8/8/8/8 w - - 9 5").expect("valid FEN");
        let next = board.make_move(Move::new(
            Square::E7,
            Square::D8,
            MoveFlags::PromoteCaptureRook,
        ));

        assert_eq!(next.piece_at(Square::E7), None);
        assert_eq!(next.piece_at(Square::D8), Some(ColoredPiece::WhiteRook));
        assert_eq!(next.halfmove_clock(), 0);
    }

    #[test]
    fn fullmove_number_increments_only_after_blacks_move() {
        let board = Board::start_pos();
        let after_white = board.make_move(Move::new(Square::G1, Square::F3, MoveFlags::Quiet));
        assert_eq!(after_white.fullmove_number(), 1);

        let after_black =
            after_white.make_move(Move::new(Square::G8, Square::F6, MoveFlags::Quiet));
        assert_eq!(after_black.fullmove_number(), 2);
    }
}
