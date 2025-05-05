/**
Represents a bitboard as a u64. A 1 bit represents a piece on the board in that square.
This implementation uses the little-endian rank-file (LERF) mapping. Thus, the bottom
row is file 0 and the top row is file 7, and the left column is rank 0 and the right
column is rank 7.

Note: The bitboard is assumed to be of an 8x8 board.
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bitboard {
    value: u64,
}

impl std::ops::BitAnd for Bitboard {
    type Output = Bitboard;

    fn bitand(self, rhs: Self) -> Self::Output {
        Bitboard {
            value: self.value & rhs.value,
        }
    }
}

impl std::ops::BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Self) {
        self.value &= rhs.value;
    }
}

impl std::ops::BitOr for Bitboard {
    type Output = Bitboard;

    fn bitor(self, rhs: Self) -> Self::Output {
        Bitboard {
            value: self.value | rhs.value,
        }
    }
}

impl std::ops::BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Self) {
        self.value |= rhs.value;
    }
}

impl std::ops::BitXor for Bitboard {
    type Output = Bitboard;

    fn bitxor(self, rhs: Self) -> Self::Output {
        Bitboard {
            value: self.value ^ rhs.value,
        }
    }
}

impl std::ops::BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Self) {
        self.value ^= rhs.value;
    }
}

impl std::ops::Not for Bitboard {
    type Output = Bitboard;

    fn not(self) -> Self::Output {
        Bitboard { value: !self.value }
    }
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

impl Bitboard {
    const LIGHT_SQUARES: u64 = 0x55AA55AA55AA55AA;
    const DARK_SQUARES: u64 = 0xAA55AA55AA55AA55;
    const NOT_A_FILE: u64 = 0xfefefefefefefefe;
    const NOT_H_FILE: u64 = 0x7f7f7f7f7f7f7f7f;

    pub const fn zeros() -> Self {
        Bitboard { value: 0 }
    }

    pub const fn ones() -> Self {
        Bitboard { value: !0 }
    }

    pub(self) const fn one_at_pos(pos: usize) -> Self {
        Bitboard { value: 1 << pos }
    }

    pub fn single_from_rank_file(rank: super::Rank, file: super::File) -> Self {
        let square = super::Square::from((rank, file));

        Self::single_from_square(square)
    }

    pub fn single_from_square(square: super::Square) -> Self {
        let pos = square as usize;

        SINGLE_BITBOARDS[pos]
    }

    pub fn zero(self) -> bool {
        self.value == 0
    }

    pub fn nonzero(self) -> bool {
        self.value != 0
    }

    pub fn shift(self, direction: Direction) -> Self {
        Self {
            value: match direction {
                Direction::Up => self.value << 8,
                Direction::Down => self.value >> 8,
                Direction::Left => (self.value >> 1) & Self::NOT_H_FILE,
                Direction::Right => (self.value << 1) & Self::NOT_A_FILE,
                Direction::UpLeft => (self.value << 7) & Self::NOT_H_FILE,
                Direction::UpRight => (self.value << 9) & Self::NOT_A_FILE,
                Direction::DownLeft => (self.value >> 9) & Self::NOT_H_FILE,
                Direction::DownRight => (self.value >> 7) & Self::NOT_A_FILE,
            },
        }
    }
}

const SINGLE_BITBOARDS: [Bitboard; 64] = {
    let mut boards = [Bitboard::zeros(); 64];
    let mut i = 0;

    while i < boards.len() {
        boards[i] = Bitboard::one_at_pos(i);
        i += 1;
    }

    boards
};
