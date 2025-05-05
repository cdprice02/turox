pub mod board;
use board::Board;

#[derive(Debug, Default)]
pub struct Engine {
    board: Board,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fen(&self) -> String {
        // TODO: implement FEN color to move, castling, en passant, etc
        format!("{} w KQkq - 0 1", self.board.fen())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod fen {
        use super::Engine;

        #[test]
        fn initial_state() {
            let engine = Engine::new();
            assert_eq!(
                engine.fen(),
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
            );
        }
    }
}
