//! UCI (Universal Chess Interface) protocol: parsing commands from stdin
//! (`position`, `go`, `isready`, ...) and emitting responses (`bestmove`, `info`,
//! ...), so `turox-cli` can drive the engine from any UCI-speaking GUI.
//!
//! `command` covers parsing; emitting responses and the stateful loop that
//! actually drives `Engine` from stdin/stdout are later issues.

pub mod command;

pub use command::{parse, Command, GoOptions};
