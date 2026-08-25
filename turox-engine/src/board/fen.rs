//! Forsyth-Edwards Notation: parsing and formatting all six FEN fields (piece
//! placement, side to move, castling rights, en passant target, halfmove clock,
//! fullmove number).

use super::error::InvalidFenError;
use super::Board;
use crate::types::{CastlingRights, Color, ColoredPiece, File, Rank, Square};

impl Board {
    /// Parses a full FEN string into a `Board`.
    ///
    /// Only the piece placement field is required; a missing side-to-move,
    /// castling, en passant, halfmove clock, or fullmove number field falls back to
    /// its usual default (`w`, `-`, `-`, `0`, `1` respectively) rather than erroring,
    /// so a bare placement string is still accepted.
    pub fn try_from_fen(fen: &str) -> Result<Self, InvalidFenError> {
        let mut fields = fen.split_whitespace();

        let placement = fields.next().ok_or(InvalidFenError::MissingField {
            field: "piece placement",
        })?;
        let board = Self::parse_placement(placement)?;

        let side_to_move = match fields.next() {
            None | Some("w") => Color::White,
            Some("b") => Color::Black,
            Some(other) => {
                return Err(InvalidFenError::InvalidField {
                    field: "side to move",
                    value: other.to_string(),
                })
            }
        };

        let castling = match fields.next() {
            None | Some("-") => CastlingRights::NONE,
            Some(s) => Self::parse_castling(s)?,
        };

        let en_passant = match fields.next() {
            None | Some("-") => None,
            Some(s) => Some(Self::parse_square(s)?),
        };

        let halfmove_clock = match fields.next() {
            None => 0,
            Some(s) => s.parse().map_err(|_| InvalidFenError::InvalidField {
                field: "halfmove clock",
                value: s.to_string(),
            })?,
        };

        let fullmove_number = match fields.next() {
            None => 1,
            Some(s) => s.parse().map_err(|_| InvalidFenError::InvalidField {
                field: "fullmove number",
                value: s.to_string(),
            })?,
        };

        Ok(Board::from_parts(
            board,
            side_to_move,
            castling,
            en_passant,
            halfmove_clock,
            fullmove_number,
        ))
    }

    /// Parses just the piece-placement field (the part before the first space).
    ///
    /// Ranks are split on `/` up front rather than tracked with a decrementing
    /// counter — the previous version of this parser did `rank -= 1` on each `/`
    /// with no lower bound, so a placement string with more than seven slashes
    /// underflowed `usize` and panicked. Splitting first makes "wrong number of
    /// ranks" a plain length check instead of an arithmetic hazard.
    fn parse_placement(placement: &str) -> Result<Board, InvalidFenError> {
        let mut board = Board::default();
        let rows: Vec<&str> = placement.split('/').collect();
        if rows.len() != 8 {
            return Err(InvalidFenError::WrongRankCount {
                expected: 8,
                found: rows.len(),
            });
        }

        for (i, row) in rows.iter().enumerate() {
            // Rows read top (rank 8) to bottom (rank 1), per FEN.
            let rank_number = 8 - i;
            let rank = Rank::from_index((rank_number - 1) as u8).expect("rank_number in 1..=8");
            let mut file = 0usize;

            for c in row.chars() {
                match c {
                    '1'..='8' => {
                        let n = c.to_digit(10).expect("matched '1'..='8'") as usize;
                        if file + n > 8 {
                            return Err(InvalidFenError::TooManyFilesInRank {
                                expected: 8,
                                found: file + n,
                                rank: rank_number,
                            });
                        }
                        file += n;
                    }
                    _ => {
                        if file >= 8 {
                            return Err(InvalidFenError::TooManyFilesInRank {
                                expected: 8,
                                found: file + 1,
                                rank: rank_number,
                            });
                        }
                        let cp = ColoredPiece::try_from_fen(c)
                            .ok_or(InvalidFenError::UnexpectedCharacter(c))?;
                        let file_enum =
                            File::from_index(file as u8).expect("file < 8 checked above");
                        board.place(Square::new(file_enum, rank), cp);
                        file += 1;
                    }
                }
            }

            if file != 8 {
                return Err(InvalidFenError::NotEnoughFilesInRank {
                    expected: 8,
                    found: file,
                    rank: rank_number,
                });
            }
        }

        Ok(board)
    }

    fn parse_castling(s: &str) -> Result<CastlingRights, InvalidFenError> {
        let mut rights = CastlingRights::NONE;
        for c in s.chars() {
            let right = match c {
                'K' => CastlingRights::WHITE_KINGSIDE,
                'Q' => CastlingRights::WHITE_QUEENSIDE,
                'k' => CastlingRights::BLACK_KINGSIDE,
                'q' => CastlingRights::BLACK_QUEENSIDE,
                _ => {
                    return Err(InvalidFenError::InvalidField {
                        field: "castling rights",
                        value: s.to_string(),
                    })
                }
            };
            rights = rights.with(right);
        }
        Ok(rights)
    }

    fn parse_square(s: &str) -> Result<Square, InvalidFenError> {
        let invalid = || InvalidFenError::InvalidField {
            field: "en passant target",
            value: s.to_string(),
        };
        let mut chars = s.chars();
        let file_ch = chars.next().ok_or_else(invalid)?;
        let rank_ch = chars.next().ok_or_else(invalid)?;
        if chars.next().is_some() {
            return Err(invalid());
        }

        let file = match file_ch {
            'a'..='h' => File::from_index(file_ch as u8 - b'a').expect("checked 'a'..='h'"),
            _ => return Err(invalid()),
        };
        let rank = match rank_ch {
            '1'..='8' => Rank::from_index(rank_ch as u8 - b'1').expect("checked '1'..='8'"),
            _ => return Err(invalid()),
        };
        Ok(Square::new(file, rank))
    }

    /// Formats this position as a full 6-field FEN string.
    ///
    /// Only reads the mailbox (`piece_at`), not the bitboards — so unlike
    /// `try_from_fen` (which places pieces via `Board::place`, and so depends on
    /// `Bitboard`'s still-unimplemented core arithmetic), this works today.
    pub fn to_fen(&self) -> String {
        let mut fen = String::new();

        for rank_idx in (0..8).rev() {
            if rank_idx != 7 {
                fen.push('/');
            }
            let rank = Rank::from_index(rank_idx).expect("rank_idx in 0..8");
            let mut empty_run = 0u32;
            for file_idx in 0..8 {
                let file = File::from_index(file_idx).expect("file_idx in 0..8");
                let sq = Square::new(file, rank);
                match self.piece_at(sq) {
                    Some(cp) => {
                        if empty_run > 0 {
                            fen.push_str(&empty_run.to_string());
                            empty_run = 0;
                        }
                        fen.push(cp.to_fen());
                    }
                    None => empty_run += 1,
                }
            }
            if empty_run > 0 {
                fen.push_str(&empty_run.to_string());
            }
        }

        fen.push(' ');
        fen.push(match self.side_to_move {
            Color::White => 'w',
            Color::Black => 'b',
        });

        fen.push(' ');
        if self.castling.is_none() {
            fen.push('-');
        } else {
            if self.castling.contains(CastlingRights::WHITE_KINGSIDE) {
                fen.push('K');
            }
            if self.castling.contains(CastlingRights::WHITE_QUEENSIDE) {
                fen.push('Q');
            }
            if self.castling.contains(CastlingRights::BLACK_KINGSIDE) {
                fen.push('k');
            }
            if self.castling.contains(CastlingRights::BLACK_QUEENSIDE) {
                fen.push('q');
            }
        }

        fen.push(' ');
        match self.en_passant {
            Some(sq) => fen.push_str(&sq.to_string()),
            None => fen.push('-'),
        }

        fen.push(' ');
        fen.push_str(&self.halfmove_clock.to_string());
        fen.push(' ');
        fen.push_str(&self.fullmove_number.to_string());

        fen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pos_round_trips_through_fen() {
        let board = Board::start_pos();
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let parsed = Board::try_from_fen(fen).expect("valid FEN");
        assert_eq!(board, parsed);
        assert_eq!(board.to_fen(), fen);
    }

    #[test]
    fn to_fen_then_try_from_fen_round_trips() {
        let board = Board::start_pos();
        let parsed = Board::try_from_fen(&board.to_fen()).expect("valid FEN");
        assert_eq!(board, parsed);
    }

    #[test]
    fn placement_with_too_few_ranks_errors_cleanly() {
        let err = Board::try_from_fen("8/8/8/8/8/8/8").expect_err("only 7 ranks");
        assert!(matches!(
            err,
            InvalidFenError::WrongRankCount {
                expected: 8,
                found: 7
            }
        ));
    }

    #[test]
    fn placement_with_too_many_ranks_does_not_panic() {
        // Nine ranks: the historical bug here was an unbounded `rank -= 1` on `/`
        // that underflowed `usize` and panicked instead of returning `Err`.
        let err = Board::try_from_fen("8/8/8/8/8/8/8/8/8").expect_err("9 ranks");
        assert!(matches!(err, InvalidFenError::WrongRankCount { .. }));
    }

    #[test]
    fn unexpected_character_errors_cleanly() {
        let err = Board::try_from_fen("8/8/8/8/8/8/8/7?").expect_err("invalid FEN");
        assert!(matches!(err, InvalidFenError::UnexpectedCharacter('?')));
    }

    #[test]
    fn missing_placement_field_errors_cleanly() {
        let err = Board::try_from_fen("").expect_err("empty FEN");
        assert!(matches!(err, InvalidFenError::MissingField { .. }));
    }
}
