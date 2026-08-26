//! `Move`, packed into a `u16`, and `MoveFlags`, its 4-bit kind tag.

use super::piece::Piece;
use super::square::Square;

/// The kind of a move, packed into 4 bits. Doubles as the promotion piece selector
/// for the four promotion variants.
#[allow(missing_docs)] // variant names are the doc
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFlags {
    Quiet = 0,
    DoublePawnPush = 1,
    KingCastle = 2,
    QueenCastle = 3,
    Capture = 4,
    EnPassant = 5,
    PromoteKnight = 8,
    PromoteBishop = 9,
    PromoteRook = 10,
    PromoteQueen = 11,
    PromoteCaptureKnight = 12,
    PromoteCaptureBishop = 13,
    PromoteCaptureRook = 14,
    PromoteCaptureQueen = 15,
}

impl MoveFlags {
    /// True for any of the eight promotion variants (with or without capture).
    pub const fn is_promotion(self) -> bool {
        (self as u8) & 0b1000 != 0
    }

    /// True for `Capture`, `EnPassant`, or any promotion-with-capture variant.
    pub const fn is_capture(self) -> bool {
        matches!(
            self,
            MoveFlags::Capture
                | MoveFlags::EnPassant
                | MoveFlags::PromoteCaptureKnight
                | MoveFlags::PromoteCaptureBishop
                | MoveFlags::PromoteCaptureRook
                | MoveFlags::PromoteCaptureQueen
        )
    }

    /// The piece a promotion variant promotes to, or `None` for non-promotions.
    pub const fn promotion_piece(self) -> Option<Piece> {
        match self {
            MoveFlags::PromoteKnight | MoveFlags::PromoteCaptureKnight => Some(Piece::Knight),
            MoveFlags::PromoteBishop | MoveFlags::PromoteCaptureBishop => Some(Piece::Bishop),
            MoveFlags::PromoteRook | MoveFlags::PromoteCaptureRook => Some(Piece::Rook),
            MoveFlags::PromoteQueen | MoveFlags::PromoteCaptureQueen => Some(Piece::Queen),
            _ => None,
        }
    }

    const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => MoveFlags::Quiet,
            1 => MoveFlags::DoublePawnPush,
            2 => MoveFlags::KingCastle,
            3 => MoveFlags::QueenCastle,
            4 => MoveFlags::Capture,
            5 => MoveFlags::EnPassant,
            8 => MoveFlags::PromoteKnight,
            9 => MoveFlags::PromoteBishop,
            10 => MoveFlags::PromoteRook,
            11 => MoveFlags::PromoteQueen,
            12 => MoveFlags::PromoteCaptureKnight,
            13 => MoveFlags::PromoteCaptureBishop,
            14 => MoveFlags::PromoteCaptureRook,
            15 => MoveFlags::PromoteCaptureQueen,
            // Every 4-bit value not covered above (6, 7) is simply unused encoding
            // space; a `Move` is only ever constructed via `Move::new`, which is the
            // sole place bits get packed, so this is unreachable in practice.
            _ => unreachable!(),
        }
    }
}

/// A move, packed into a `u16`: `from(6) | to(6) | flags(4)`.
///
/// Deliberately not `{ from: (usize, usize), to: (usize, usize) }` (32 untyped
/// bytes): packed, this is 2 bytes, which is what makes a stack-allocated
/// `MoveList` of up to 256 moves (512 bytes) practical instead of costing 8 KB.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Move(u16);

impl Move {
    /// Packs a move from `from` to `to` with the given `flags`.
    pub const fn new(from: Square, to: Square, flags: MoveFlags) -> Self {
        let bits = (from.index() as u16) | ((to.index() as u16) << 6) | ((flags as u16) << 12);
        Self(bits)
    }

    /// The square this move starts from.
    pub const fn from(self) -> Square {
        match Square::from_index((self.0 & 0x3F) as u8) {
            Some(sq) => sq,
            None => unreachable!(),
        }
    }

    /// The square this move lands on.
    pub const fn to(self) -> Square {
        match Square::from_index(((self.0 >> 6) & 0x3F) as u8) {
            Some(sq) => sq,
            None => unreachable!(),
        }
    }

    /// This move's kind.
    pub const fn flags(self) -> MoveFlags {
        MoveFlags::from_bits((self.0 >> 12) as u8)
    }
}

impl std::fmt::Debug for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}{:?}", self.from(), self.to())?;
        if let Some(p) = self.flags().promotion_piece() {
            write!(f, "={p:?}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_from_to_and_flags() {
        for &flags in &[
            MoveFlags::Quiet,
            MoveFlags::DoublePawnPush,
            MoveFlags::KingCastle,
            MoveFlags::QueenCastle,
            MoveFlags::Capture,
            MoveFlags::EnPassant,
            MoveFlags::PromoteKnight,
            MoveFlags::PromoteBishop,
            MoveFlags::PromoteRook,
            MoveFlags::PromoteQueen,
            MoveFlags::PromoteCaptureKnight,
            MoveFlags::PromoteCaptureBishop,
            MoveFlags::PromoteCaptureRook,
            MoveFlags::PromoteCaptureQueen,
        ] {
            let m = Move::new(Square::E2, Square::E4, flags);
            assert_eq!(m.from(), Square::E2);
            assert_eq!(m.to(), Square::E4);
            assert_eq!(m.flags(), flags);
        }
    }

    #[test]
    fn promotion_flags_report_promotion_and_piece() {
        assert_eq!(
            MoveFlags::PromoteQueen.promotion_piece(),
            Some(Piece::Queen)
        );
        assert!(MoveFlags::PromoteQueen.is_promotion());
        assert!(!MoveFlags::Quiet.is_promotion());
        assert_eq!(MoveFlags::Quiet.promotion_piece(), None);
    }

    #[test]
    fn capture_flags_report_capture() {
        assert!(MoveFlags::Capture.is_capture());
        assert!(MoveFlags::EnPassant.is_capture());
        assert!(MoveFlags::PromoteCaptureQueen.is_capture());
        assert!(!MoveFlags::Quiet.is_capture());
        assert!(!MoveFlags::PromoteQueen.is_capture());
    }

    #[test]
    fn move_is_two_bytes() {
        assert_eq!(std::mem::size_of::<Move>(), 2);
    }
}
