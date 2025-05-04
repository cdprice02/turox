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

    pub fn board(&self) -> &Board {
        &self.board
    }
}
