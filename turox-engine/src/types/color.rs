//! `Color`: which side a piece or move belongs to.

use crate::{Direction, Rank};

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
    pub const ALL: [Self; 2] = [Self::White, Self::Black];

    /// The other color.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    /// This color's own back rank (White: 1, Black: 8): where its king and
    /// rooks start.
    #[must_use]
    pub const fn back_rank(self) -> Rank {
        match self {
            Self::White => Rank::R1,
            Self::Black => Rank::R8,
        }
    }

    /// The opposing back rank (White: 8, Black: 1): where this color's pawns
    /// promote. Just `self.flip().back_rank()`, spelled out as its own method
    /// so pawn-promotion call sites read as "is this pawn on its promotion
    /// rank" rather than "is this pawn on the flipped color's back rank",
    /// without a second hand-written White/Black mapping to keep in sync with
    /// `back_rank`'s.
    #[must_use]
    pub const fn far_rank(self) -> Rank {
        self.flip().back_rank()
    }

    /// The rank a double pawn push lands on for this color (White: 4, Black: 5).
    #[must_use]
    pub const fn double_pawn_push_rank(self) -> Rank {
        match self {
            Self::White => Rank::R4,
            Self::Black => Rank::R5,
        }
    }

    /// The direction this color's pawns move: north for White, south for
    /// Black. The one place the White/Black asymmetry needs encoding for
    /// pawn pushes and their en passant bookkeeping; every call site should
    /// go through this rather than its own `match color { ... }`.
    #[must_use]
    pub const fn forward(self) -> Direction {
        match self {
            Self::White => Direction::North,
            Self::Black => Direction::South,
        }
    }
}
