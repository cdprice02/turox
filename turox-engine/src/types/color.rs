//! `Color`: which side a piece or move belongs to.

use crate::Rank;

/// Which side a piece or move belongs to.
#[allow(missing_docs)] // variant names are the doc
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    /// Both colors, White first.
    pub const ALL: [Color; 2] = [Color::White, Color::Black];

    /// The other color.
    pub const fn flip(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    /// This color's own back rank (White: 1, Black: 8) — where its king and
    /// rooks start.
    pub const fn back_rank(self) -> Rank {
        match self {
            Color::White => Rank::R1,
            Color::Black => Rank::R8,
        }
    }

    /// The opposing back rank (White: 8, Black: 1) — where this color's pawns
    /// promote. Just `self.flip().back_rank()`, spelled out as its own method
    /// so pawn-promotion call sites read as "is this pawn on its promotion
    /// rank" rather than "is this pawn on the flipped color's back rank" —
    /// without a second hand-written White/Black mapping to keep in sync with
    /// `back_rank`'s.
    pub const fn far_rank(self) -> Rank {
        self.flip().back_rank()
    }

    /// The rank a double pawn push lands on for this color (White: 4, Black: 5).
    pub const fn double_pawn_push_rank(self) -> Rank {
        match self {
            Color::White => Rank::R4,
            Color::Black => Rank::R5,
        }
    }
}
