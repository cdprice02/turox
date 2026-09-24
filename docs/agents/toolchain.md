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
| refactor gate     | `tools/refactor-gate.sh --base main`                                  |
| tree shape, EBF   | `tools/treeshape/measure.py --ref main --ref HEAD --depth 7`          |
| profile           | `cargo build --profile samply -p turox-cli`, then `samply record target/samply/turox-cli` |
| fuzz              | `cargo fuzz run fen --fuzz-dir turox-fuzz`                            |
| mutants, scoped   | `cargo mutants -p turox-engine --file '**/NAME.rs'`                    |
| coverage          | `cargo llvm-cov --workspace`                                          |

## What lives in `tools/`

Anything not shipped to a chess GUI. `turox-engine`, `turox-cli` and
`turox-macros` are the engine; everything that measures it, feeds it, or checks
the repo around it lives here, in whatever language suits the job. `bookgen` is
a workspace member despite living here, so every `--workspace` command reaches
it; `turox-fuzz` stays outside because it needs a different toolchain, which is
a different reason from the one `bookgen` used to have.

Two rules, both learned rather than assumed:

**A tool is not installed until this table names it.**
`tools/treeshape/measure.py` measured the exact quantity a refactor has to
prove and was referenced from nowhere in the repo; `[profile.samply]` sat in
the root manifest with no README section and no row here. Both worked. Neither
was reachable by anyone who did not already know it existed.

**A tool owes verification in proportion to what breaks when it is wrong, not
to the language it is written in.** `tools/lichess/test-run-bot.sh` guards the
leak that got this machine's address null-routed for a day, so it earns a place
in CI on that measure, and does not yet hold one: it needs job control, which a
runner has no terminal to provide, and the attempt took a runner down. Run it
by hand after touching `run-bot.sh`. `tools/gamelog/summarize.py` and
`tools/treeshape/measure.py` break nothing when wrong, since a bad number is
visible to whoever asked for it, and they stay unverified on purpose.

The same measure cuts the other way, and did: a prose checker covering two of
`voice.md`'s nine rules was deleted rather than given tests, because a rule
that needs judgement is not made safer by mechanising the part that does not.

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

  **A refactor is the exception**, and `tools/refactor-gate.sh` is what
  decides whether it qualifies. A change that is move-identical (same node
  counts and same principal variation, see `CONTEXT.md`) at unchanged
  throughput cannot have changed playing strength, because same moves at the
  same speed is the same engine, so a match would be measuring nothing. Run
  the gate; it names the outcome and tells you when a match is still owed.
  Reach for the SPRT directly whenever what the engine *plays* is meant to
  change.

- **The refactor gate's two halves are not equally trustworthy.**
  Move-identity is exact: same node counts and same principal variation, or
  not. Throughput is close to useless, and the measurement is the reason
  rather than the hardware being slow.

  Comparing a tree *against itself*, so every number is noise, criterion
  reports the second side as nine to twenty percent **faster**, never near
  zero. That is a bias, not scatter, and it runs one way: the first process of
  a run pays for a cold machine. The gate measures the baseline first, so the
  bias favours passing.

  **More samples makes it worse.** At ten samples, three null comparisons in
  eight were called an improvement; at thirty, seven in eight, because a
  narrower interval around a biased estimate is just more confidence in the
  wrong answer. A discarded warm-up pass roughly halves the bias and does not
  remove it; the gate does that and sets a floor of twenty percent, below
  which it reports nothing either way.

  So a clean gate run means "no regression larger than about twenty percent",
  which is enough for a pure refactor and is not a throughput measurement.
  Anything finer needs a quiet machine and an alternating base/candidate
  design that nothing here does. An SPRT is no substitute: at the usual bounds
  it detects about ten Elo, and a few percent of speed is worth a few Elo.
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
- **A whole-crate `cargo mutants` run does not finish.** Every mutant costs a
  rebuild and a test run, so the full set takes hours. CI shards it eight ways
  round-robin; locally, scope it (`--file '**/phase.rs'`, `--re SomeName`) or
  run a shard. `--shard` is zero-indexed, so eight shards are `0/8` through
  `7/8` and `8/8` silently selects nothing.
- **Nothing checks prose.** `docs/agents/voice.md` is enforced by review and
  by reading it, not by a script. One existed for its two most mechanical
  rules and was deleted: a green tick on two rules out of nine was not evidence
  the prose was good, and it drew attention away from the seven that decide
  whether it is.
- **Clippy runs `pedantic` and `nursery`, plus a hand-picked set of
  restriction lints** (`unwrap_used`, `unreachable`, `wildcard_enum_match_arm`,
  `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, `dbg_macro`,
  `missing_assert_message`, `allow_attributes_without_reason`,
  `string_slice`/`as_conversions`, and more; see `[workspace.lints.clippy]`
  in the root `Cargo.toml` for the live list). `indexing_slicing` and
  `arithmetic_side_effects` are the two intentional exceptions, not deferred
  ones: this is bitboard/table-driven engine code, so indexing and
  arithmetic are the normal way to write it, not the exception a
  restriction lint is meant to catch. `allow_attributes` requires `#[expect(...)]`
  in place of `#[allow(...)]`, each with a `reason = "..."` explaining the
  specific call site; that's what to read (or add to) rather than reaching for
  a blanket lint change. An expectation also fails the build once its lint
  stops firing, so a suppression that has outlived its reason is reported.
  `#[derive(Ordinal)]` in `turox-macros` is how the ordinal enums
  (`Color`/`Piece`/`ColoredPiece`/`File`/`Rank`/`Square`) stayed clean of
  `as` without hand-writing six copies of the same accessor.
