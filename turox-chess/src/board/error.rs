//! `InvalidFenError`: why a FEN string failed to parse.

use std::fmt;

/// A FEN string failed to parse.
///
/// Hand-written `Display`/`Error` rather than `thiserror`: the engine takes zero
/// runtime dependencies (see `turox-engine/Cargo.toml`), and this is the entire
/// cost of doing without the derive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidFenError {
    /// A character that isn't valid anywhere in a FEN string.
    UnexpectedCharacter(char),
    /// A rank's piece-placement field named fewer than 8 files.
    NotEnoughFilesInRank {
        /// Files a rank must always cover (8).
        expected: usize,
        /// Files this rank's field actually covered.
        found: usize,
        /// The rank (0 = rank 1) with too few files.
        rank: usize,
    },
    /// A rank's piece-placement field named more than 8 files.
    TooManyFilesInRank {
        /// Files a rank must always cover (8).
        expected: usize,
        /// Files this rank's field actually covered.
        found: usize,
        /// The rank (0 = rank 1) with too many files.
        rank: usize,
    },
    /// The piece-placement field didn't have exactly 8 `/`-separated ranks.
    WrongRankCount {
        /// Ranks a board must always have (8).
        expected: usize,
        /// Ranks the piece-placement field actually had.
        found: usize,
    },
    /// A required space-separated FEN field was absent.
    MissingField {
        /// The field's name (e.g. `"side to move"`).
        field: &'static str,
    },
    /// A FEN field was present but its value couldn't be parsed.
    InvalidField {
        /// The field's name (e.g. `"castling rights"`).
        field: &'static str,
        /// The value that failed to parse.
        value: String,
    },
}

impl fmt::Display for InvalidFenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedCharacter(c) => {
                write!(f, "unexpected character: {c:?}")
            }
            Self::NotEnoughFilesInRank {
                expected,
                found,
                rank,
            } => write!(
                f,
                "not enough files in rank {rank}: expected {expected}, found {found}"
            ),
            Self::TooManyFilesInRank {
                expected,
                found,
                rank,
            } => write!(
                f,
                "too many files in rank {rank}: expected {expected}, found {found}"
            ),
            Self::WrongRankCount { expected, found } => {
                write!(
                    f,
                    "wrong number of ranks: expected {expected}, found {found}"
                )
            }
            Self::MissingField { field } => {
                write!(f, "missing FEN field: {field}")
            }
            Self::InvalidField { field, value } => {
                write!(f, "invalid value for FEN field {field}: {value:?}")
            }
        }
    }
}

impl std::error::Error for InvalidFenError {}
