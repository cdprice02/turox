//! UCI (Universal Chess Interface) protocol: parsing commands from stdin
//! (`position`, `go`, `isready`, ...) and emitting responses (`bestmove`, `info`,
//! ...), so `turox-cli` can drive the engine from any UCI-speaking GUI.
//!
//! `command` covers parsing and `response` covers emitting; the stateful
//! loop that actually drives `Engine` from stdin/stdout using both is a
//! later issue.

pub mod command;
pub mod response;

pub use command::{parse, Command, GoOptions};
pub use response::Response;
