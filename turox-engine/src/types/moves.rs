//! `Move`, packed into a `u16`, and `MoveFlags`, its 4-bit kind tag.

use super::piece::Piece;
use super::square::Square;

/// The kind of a move, packed into 4 bits. Doubles as the promotion piece selector
/// for the four promotion variants.
#[allow(missing_docs, reason = "variant names are the doc")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    #[must_use]
    pub const fn is_promotion(self) -> bool {
        self.bits() & 0b1000 != 0
    }

    /// True for `Capture`, `EnPassant`, or any promotion-with-capture variant.
    #[must_use]
    pub const fn is_capture(self) -> bool {
        matches!(
            self,
            Self::Capture
                | Self::EnPassant
                | Self::PromoteCaptureKnight
                | Self::PromoteCaptureBishop
                | Self::PromoteCaptureRook
                | Self::PromoteCaptureQueen
        )
    }

    /// The move is en passant.
    #[inline]
    #[must_use]
    pub const fn is_en_passant(self) -> bool {
        matches!(self, Self::EnPassant)
    }

    /// The piece a promotion variant promotes to, or `None` for non-promotions.
    #[must_use]
    pub const fn promotion_piece(self) -> Option<Piece> {
        match self {
            Self::PromoteKnight | Self::PromoteCaptureKnight => Some(Piece::Knight),
            Self::PromoteBishop | Self::PromoteCaptureBishop => Some(Piece::Bishop),
            Self::PromoteRook | Self::PromoteCaptureRook => Some(Piece::Rook),
            Self::PromoteQueen | Self::PromoteCaptureQueen => Some(Piece::Queen),
            Self::Quiet
            | Self::DoublePawnPush
            | Self::KingCastle
            | Self::QueenCastle
            | Self::Capture
            | Self::EnPassant => None,
        }
    }

    /// `None` for the two 4-bit values (6, 7) that are simply unused encoding
    /// space: a total function over the full `u8` domain `Move::flags` can
    /// `.expect()` against, rather than a partial one hiding an `unreachable!()`
    /// behind an assumption about who's allowed to call it.
    const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Quiet),
            1 => Some(Self::DoublePawnPush),
            2 => Some(Self::KingCastle),
            3 => Some(Self::QueenCastle),
            4 => Some(Self::Capture),
            5 => Some(Self::EnPassant),
            8 => Some(Self::PromoteKnight),
            9 => Some(Self::PromoteBishop),
            10 => Some(Self::PromoteRook),
            11 => Some(Self::PromoteQueen),
            12 => Some(Self::PromoteCaptureKnight),
            13 => Some(Self::PromoteCaptureBishop),
            14 => Some(Self::PromoteCaptureRook),
            15 => Some(Self::PromoteCaptureQueen),
            _ => None,
        }
    }

    /// This variant's flag bits (its `#[repr(u8)]` discriminant).
    // Not `#[derive(Ordinal)]` like `Color`/`Piece`/`Square`: those discriminants
    // are dense 0..N ordinals where the number is incidental to the type: this
    // one's discriminants are sparse (0-5, then 8-15) and *are* the point, a bit
    // pattern `Move` packs directly. `Ordinal::to_u8` would work here too, but
    // would misdescribe what these values mean.
    #[allow(
        clippy::as_conversions,
        reason = "sparse bit-pattern discriminants, not an Ordinal; this is the intended way to read them"
    )]
    const fn bits(self) -> u8 {
        self as u8
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
    /// This move's packed representation, for a caller (a transposition
    /// table entry) that needs the raw bits rather than a `Move` value.
    /// `pub(crate)`, not `pub`: nothing outside this crate has a reason to
    /// see a move's bit layout rather than working through `Move` itself.
    #[must_use]
    pub(crate) const fn bits(self) -> u16 {
        self.0
    }

    /// Reconstructs a move from its packed bits (the inverse of [`Self::bits`]),
    /// for a caller (a transposition table probe) that has raw bits rather
    /// than a `Move` value. `pub(crate)`, matching `bits`'s own visibility:
    /// nothing outside this crate has a reason to work with a move's bit
    /// layout directly.
    #[must_use]
    pub(crate) const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Packs a move from `from` to `to` with the given `flags`.
    #[must_use]
    #[allow(
        clippy::as_conversions,
        reason = "from.to_u8()/to.to_u8()/flags.bits() are u8; From isn't const-callable yet (rust-lang/rust#143874), so widening to u16 stays `as`"
    )]
    pub const fn new(from: Square, to: Square, flags: MoveFlags) -> Self {
        let bits = from.to_u8() as u16 | ((to.to_u8() as u16) << 6) | ((flags.bits() as u16) << 12);
        Self(bits)
    }

    /// The square this move starts from.
    ///
    /// # Panics
    ///
    /// Never: the stored bits are masked to 6 bits (`& 0x3F`), always < 64.
    #[must_use]
    pub const fn from(self) -> Square {
        Square::from_u8(self.0.to_le_bytes()[0] & 0x3F)
            .expect("masked to 6 bits (& 0x3F), so always < 64")
    }

    /// The square this move lands on.
    ///
    /// # Panics
    ///
    /// Never: the stored bits are masked to 6 bits (`& 0x3F`), always < 64.
    #[must_use]
    pub const fn to(self) -> Square {
        Square::from_u8((self.0 >> 6).to_le_bytes()[0] & 0x3F)
            .expect("masked to 6 bits (& 0x3F), so always < 64")
    }

    /// This move's kind.
    ///
    /// # Panics
    ///
    /// Never: `Move` is only ever built via `Move::new`, which packs one of
    /// the 14 valid flag patterns `MoveFlags::from_bits` recognizes.
    #[must_use]
    pub const fn flags(self) -> MoveFlags {
        MoveFlags::from_bits((self.0 >> 12).to_le_bytes()[0]).expect(
            "Move is only ever built via Move::new, which packs one of the 14 valid patterns",
        )
    }

    /// Formats this move in UCI notation: `e2e4` for a quiet move or
    /// capture, `e7e8q` for a promotion (lowercase piece letter, no `=` or
    /// capture marker; the four promotion pieces only, matching
    /// `MoveFlags::promotion_piece`'s own `Option<Piece>`).
    ///
    /// Castling needs no special case here: `pseudo_legal_moves` already
    /// encodes a castling move's `to()` as the king's own real destination
    /// square (`e1g1`, not `e1h1`, the rook's square) as part of computing
    /// the move at all, not something this function has to know about or
    /// reconstruct. Worth confirming for yourself rather than taking on
    /// faith, since "does castling need special-casing" is exactly the
    /// kind of thing that looks like it should from the UCI spec alone.
    ///
    /// # Panics
    ///
    /// Never: `is_promotion()` true implies `promotion_piece()` returns `Some`.
    #[must_use]
    pub fn to_uci(self) -> String {
        let mut s = format!("{}{}", self.from(), self.to());
        if self.flags().is_promotion() {
            let p = self
                .flags()
                .promotion_piece()
                .expect("is_promotion() is true, so promotion_piece() always returns Some");
            s.push(match p {
                Piece::Knight => 'n',
                Piece::Bishop => 'b',
                Piece::Rook => 'r',
                Piece::Queen => 'q',
                #[allow(
                    clippy::unreachable,
                    reason = "is_promotion() gates this on the flag's own promotion bit, which promotion_piece() never maps to Pawn/King"
                )]
                Piece::Pawn | Piece::King => {
                    unreachable!("promotion_piece() never returns Pawn/King when is_promotion() is true")
                }
            });
        }
        s
    }

    /// Parses a UCI move string (`"e2e4"`, `"e7e8q"`) and resolves it
    /// against `legal`, the legal moves in the position this string is
    /// meant to apply to. Returns `None` for a malformed string (bad
    /// square notation, a promotion letter that isn't one of the four real
    /// pieces) or a well-formed one that just isn't a legal move here (a
    /// UCI move string alone carries no capture/en-passant/castling
    /// information, so there's no way to reconstruct the real
    /// `MoveFlags` without a legal move list to resolve it against; see
    /// this module's own `Move` doc for why re-deriving flags from the
    /// string independently would just duplicate `pseudo_legal`'s logic
    /// with a second chance to get it wrong).
    ///
    /// `Square::try_from_algebraic` parses the two 2-character square
    /// substrings; a fifth byte, if present, is the promotion letter
    /// (`n`/`b`/`r`/`q`, lowercase only, per the UCI spec).
    ///
    /// Works in bytes, not `chars()`, on the strength of the `is_ascii()`
    /// check up front: a UCI move string is never legitimately anything
    /// else, and rejecting non-ASCII input outright (rather than walking a
    /// `Chars` iterator to stay Unicode-correct for input that was never
    /// going to be valid anyway) sidesteps `str`'s char-boundary slicing
    /// panics entirely, not just in the common case. `bytes.get(4)`
    /// (`None` past the end of the slice) replaces a separate length
    /// branch for the optional promotion byte with a single match arm.
    #[must_use]
    pub fn from_uci(s: &str, legal: &[Self]) -> Option<Self> {
        if !(4..=5).contains(&s.len()) || !s.is_ascii() {
            return None;
        }

        let from = Square::try_from_algebraic(s.get(..2)?)?;
        let to = Square::try_from_algebraic(s.get(2..4)?)?;
        let promotion_piece = match s.as_bytes().get(4) {
            Some(b'n') => Some(Piece::Knight),
            Some(b'b') => Some(Piece::Bishop),
            Some(b'r') => Some(Piece::Rook),
            Some(b'q') => Some(Piece::Queen),
            Some(_) => return None,
            None => None,
        };

        legal
            .iter()
            .find(|&m| {
                m.from() == from && m.to() == to && m.flags().promotion_piece() == promotion_piece
            })
            .copied()
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

    const ALL_FLAGS: [MoveFlags; 14] = [
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
    ];

    #[test]
    fn round_trips_from_to_and_flags() {
        for from in Square::ALL {
            for to in Square::ALL {
                for flags in ALL_FLAGS {
                    let m = Move::new(from, to, flags);
                    assert_eq!(m.from(), from, "from={from:?} to={to:?} flags={flags:?}");
                    assert_eq!(m.to(), to, "from={from:?} to={to:?} flags={flags:?}");
                    assert_eq!(m.flags(), flags, "from={from:?} to={to:?} flags={flags:?}");
                }
            }
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
