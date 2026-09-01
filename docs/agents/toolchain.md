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
| watch             | `bacon` (clippy on save), `bacon nextest` (tests)                     |
| typecheck         | `cargo check --all-targets`                                           |
| lint              | `cargo clippy --all-targets --all-features -- -D warnings`            |
| fmt               | `cargo fmt --all`                                                     |
| docs              | `RUSTDOCFLAGS="--deny warnings" cargo doc --workspace --no-deps`      |
| bench             | `cargo bench -p turox-engine`                                         |
| bench vs baseline | `cargo bench -p turox-engine -- --save-baseline before`, then `-- --baseline before` |
| self-play A/B     | `tools/selfplay/sprt.sh --base main --test my-branch`                 |
| fuzz              | `cd turox-fuzz && cargo fuzz run fen`                                  |
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
  `--workspace` commands don't touch it.
- **Clippy runs `pedantic` and `nursery`, but several individual lints are
  deliberately commented out in the root `Cargo.toml`** (`unwrap_used`,
  `expect_used`, `indexing_slicing`, `arithmetic_side_effects`,
  `unreachable`, `string_slice`, `as_conversions`, plus `cast_possible_wrap`/
  `cast_possible_truncation`/`cast_sign_loss` explicitly allowed back). Each
  needs a per-call-site judgment call across hot-path bitboard/search code
  (checked/wrapping rewrite vs. a reviewed `#[allow]`), not a blanket
  decision, so they're deferred rather than silently dropped. See the
  comment above them in `Cargo.toml` and the open issue tracking the
  remainder before adding new code that would trip one of them.
