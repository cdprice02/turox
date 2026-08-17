use std::fmt;

/// A FEN string failed to parse.
///
/// Hand-written `Display`/`Error` rather than `thiserror`: the engine takes zero
/// runtime dependencies (see `turox-engine/Cargo.toml`), and this is the entire
/// cost of doing without the derive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidFenError {
    UnexpectedCharacter(char),
    NotEnoughFilesInRank {
        expected: usize,
        found: usize,
        rank: usize,
    },
    TooManyFilesInRank {
        expected: usize,
        found: usize,
        rank: usize,
    },
    WrongRankCount {
        expected: usize,
        found: usize,
    },
    MissingField {
        field: &'static str,
    },
    InvalidField {
        field: &'static str,
        value: String,
    },
}

impl fmt::Display for InvalidFenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidFenError::UnexpectedCharacter(c) => {
                write!(f, "unexpected character: {c:?}")
            }
            InvalidFenError::NotEnoughFilesInRank {
                expected,
                found,
                rank,
            } => write!(
                f,
                "not enough files in rank {rank}: expected {expected}, found {found}"
            ),
            InvalidFenError::TooManyFilesInRank {
                expected,
                found,
                rank,
            } => write!(
                f,
                "too many files in rank {rank}: expected {expected}, found {found}"
            ),
            InvalidFenError::WrongRankCount { expected, found } => {
                write!(
                    f,
                    "wrong number of ranks: expected {expected}, found {found}"
                )
            }
            InvalidFenError::MissingField { field } => {
                write!(f, "missing FEN field: {field}")
            }
            InvalidFenError::InvalidField { field, value } => {
                write!(f, "invalid value for FEN field {field}: {value:?}")
            }
        }
    }
}

impl std::error::Error for InvalidFenError {}
