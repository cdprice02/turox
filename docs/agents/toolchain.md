# Toolchain: turox

Overrides the generic `toolchain` skill for this repo. README.md explains
why each of these exists; this file is the lookup.

| Job               | Command                                                              |
| ----------------- | --------------------------------------------------------------------- |
| build             | `cargo build --workspace`                                             |
| run               | `cargo run -p turox-cli`                                              |
| test              | `cargo nextest run --workspace`, then `cargo test --doc --workspace`  |
| test one          | `cargo nextest run -E 'test(NAME)'`                                   |
| test, deep        | `cargo nextest run --workspace --release --run-ignored all`           |
| test, accepted gaps | `cargo nextest run --workspace --release --profile accepted-gaps --run-ignored all` |
| watch             | `bacon` (clippy on save), `bacon nextest` (tests)                     |
| typecheck         | `cargo check --all-targets`                                           |
| lint              | `cargo clippy --all-targets --all-features -- -D warnings`            |
| fmt               | `cargo fmt --all`                                                     |
| docs              | `RUSTDOCFLAGS="--deny warnings" cargo doc --workspace --no-deps`      |
| bench             | `cargo bench -p turox-engine`                                         |
| bench vs baseline | `cargo bench -p turox-engine -- --save-baseline before`, then `-- --baseline before` |
| self-play A/B     | `tools/selfplay/sprt.sh --base main --test my-branch`                 |
| fuzz              | `cargo fuzz run fen --fuzz-dir turox-fuzz`                            |
| mutants           | `cargo mutants -p turox-engine`                                       |
| coverage          | `cargo llvm-cov --workspace`                                          |

## Gotchas

- **nextest structurally cannot run doctests.** Not a config gap:
  doctests aren't exposed as test binaries on stable Rust. Run
  `cargo test --doc` as a second command, the way CI does. It costs no
  extra builds. Don't go hunting for a single-command fix.
- **The deepest perft tests are `#[ignore]`d and need `--release`.** The
  `dev` profile is far too slow for million-node perft even with the
  `opt-level = 1` bump in the root `Cargo.toml`.
- **Benchmarks say nothing about playing strength.** A search or eval
  change needs `tools/selfplay/sprt.sh` for a pass/fail verdict from
  actual games; `cargo bench` only answers whether it got faster.
- **CI never runs benchmarks on a PR**, only `cargo bench --no-run` to
  keep them compiling, because shared runners aren't consistent enough
  run to run for the numbers to mean anything. A weekly scheduled job
  runs them for real, informationally, alongside mutants and coverage.
- **`turox-fuzz` is outside the workspace** and needs nightly, so
  `--workspace` commands don't touch it. CI `cargo check`s it separately for
  that reason, because nothing else would notice it breaking. `cargo fuzz`
  also insists on a directory named exactly `fuzz` under the workspace root,
  so every invocation needs `--fuzz-dir turox-fuzz`; `cd`ing into the crate
  does not help, since it walks back up to the root regardless.
- **`#[ignore]` means "not in a default run"; the name says why.** Most
  ignored tests here are simply too slow for every push. Tests that are
  *expected to fail*, pinning a correctness gap an ADR has consciously
  accepted, carry an `accepted_gap_` prefix as well, and are excluded twice
  over: by `#[ignore]`, and by the default filter in `.config/nextest.toml`.
  Both are needed, and neither is redundant. The deep job runs
  `--run-ignored all`, which defeats the attribute, so only the filter
  excludes them there; `cargo llvm-cov` and `cargo mutants` shell out to
  plain `cargo test`, which never sees a nextest profile, so only the
  attribute excludes them there. Removing either one breaks a real job, and
  both failure modes were observed rather than predicted. The weekly
  `accepted-gaps` job runs exactly these tests (`--run-ignored all` plus the
  profile) and inverts the result, so it goes red when one *passes*, meaning
  a gap closed and its ADR is stale.
- **Clippy runs `pedantic` and `nursery`, plus a hand-picked set of
  restriction lints** (`unwrap_used`, `unreachable`, `wildcard_enum_match_arm`,
  `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, `dbg_macro`,
  `missing_assert_message`, `allow_attributes_without_reason`,
  `string_slice`/`as_conversions`, and more; see `[workspace.lints.clippy]`
  in the root `Cargo.toml` for the live list). `indexing_slicing` and
  `arithmetic_side_effects` are the two intentional exceptions, not deferred
  ones: this is bitboard/table-driven engine code, so indexing and
  arithmetic are the normal way to write it, not the exception a
  restriction lint is meant to catch. Every `#[allow(...)]` in the codebase
  carries a `reason = "..."` explaining the specific call site; that's what
  to read (or add to) rather than reaching for a blanket lint change.
  `#[derive(Ordinal)]` in `turox-macros` is how the ordinal enums
  (`Color`/`Piece`/`ColoredPiece`/`File`/`Rank`/`Square`) stayed clean of
  `as` without hand-writing six copies of the same accessor.
