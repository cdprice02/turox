/**
Represents a bitboard as a u64. A 1 bit represents a piece on the board in that square.
This implementation uses the little-endian rank-file (LERF) mapping. Thus, the bottom
row is file 0 and the top row is file 7, and the left column is rank 0 and the right
column is rank 7.
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

impl Bitboard {
    pub fn new_empty() -> Self {
        Bitboard { value: 0 }
    }

    pub fn from_rank_file(rank: super::Rank, file: super::File) -> Self {
        let pos: u64 = rank as u64 * 8 + file as u64;

        Bitboard { value: 1 << pos }
    }
}
