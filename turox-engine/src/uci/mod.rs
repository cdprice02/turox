//! UCI (Universal Chess Interface) protocol.
//!
//! Parsing commands from stdin (`position`, `go`, `isready`, ...) and emitting responses
//! (`bestmove`, `info`, ...), so `turox-cli` can drive the engine from any UCI-speaking
//! GUI.
//!
//! `command` covers parsing, `response` covers emitting, and `session` is the stateful
//! loop that drives an `Engine`'s `Board` from stdin/stdout using both.

pub mod command;
pub mod response;
mod session;

pub use command::{parse, Command, GoOptions};
pub use response::Response;
pub(crate) use session::run as run_session;
