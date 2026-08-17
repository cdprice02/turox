//! UCI (Universal Chess Interface) protocol: parsing commands from stdin
//! (`position`, `go`, `isready`, ...) and emitting responses (`bestmove`, `info`,
//! ...), so `turox-cli` can drive the engine from any UCI-speaking GUI.
//!
//! Not yet implemented — waits on `search` to have something to report.
