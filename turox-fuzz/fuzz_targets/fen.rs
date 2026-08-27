//! Coverage-guided fuzzing of `Board::try_from_fen`, the one place the engine
//! will take untrusted input directly off the wire: UCI's `position fen
//! <...>` command. `Err` is a correct outcome for a malformed string; a panic
//! is not. `tests/fen_props.rs` already checks the same property
//! (`try_from_fen_never_panics_on_arbitrary_input`) over a proptest-generated
//! `".{0,64}"` regex; this is the coverage-guided version of it, exploring
//! inputs that random regex sampling won't reliably hit.
//!
//! Run with `cargo +nightly fuzz run fen` from this directory (`cargo-fuzz`
//! requires nightly; not part of the stable CI job for that reason, run on
//! demand instead).

#![no_main]

use libfuzzer_sys::fuzz_target;
use turox_engine::board::Board;

fuzz_target!(|data: &str| {
    let _ = Board::try_from_fen(data);
});
