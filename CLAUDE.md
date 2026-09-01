# Project: turox

## Overview

A chess engine written from scratch in Rust. The point is the process:
hands-on practice with bit-manipulation (bitboards, magic numbers,
Kogge-Stone parallel prefix) and with real Rust performance work
(profiling, target-feature tuning, benchmark-driven iteration). Chess is
the vehicle, not the end goal.

The concrete target: get the engine playing on lichess via `lichess-bot`,
so performance and correctness work has a real rating to point at rather
than "the tests pass." That makes the UCI layer load-bearing, not a
permanent stub.

`README.md` is the reference for architecture, tooling, and the reasoning
behind each part of the loop. It is accurate and written for a human
reader; read it rather than re-deriving any of it here.

## Build & run

`docs/agents/toolchain.md` is the command table for this repo, and
overrides the generic `toolchain` skill.

## Architecture

See README.md: `types -> board -> move_gen -> search / eval / uci`, plus
why `types` sits at the crate root and why `turox-engine` takes zero
runtime dependencies.

## Status

See README.md's Status section. Keep it current there; don't mirror it
here.

## How we work

- I write the implementations myself (algorithms, bit-twiddling, control
  flow); that's the point of the exercise. Guide me, explain the
  technique, name the gotchas, rather than handing me finished code,
  unless I ask directly or it's scaffolding/plumbing/non-hot-path code
  that isn't the exercise.
- Pure audit/cleanup/reorg/tooling passes (doc rewrites, file splits,
  lint setup, dependency and CI work) are fully yours to write end to end
  when I say so, not just plumbing within a feature PR.
- You write the tests, before or alongside my implementation, not after:
  property tests (proptest) against an independent reference
  implementation wherever one is derivable, concrete FEN and scenario
  tests for rule-heavy or stateful logic otherwise. Comprehensive
  test-writing isn't the part I'm here to practice.
- When reviewing my code, verify against actual test output, not a
  read-through. This project has had real bugs that looked correct on
  inspection and weren't.
- Watch for {Color}x{direction} and {Color}x{side} mappings: White vs
  Black crossed with kingside/queenside, N/S/E/W, or +/- rank deltas.
  That exact shape has repeatedly produced scrambled bugs here
  (`Board::make_move`, castling rook lookup, magic mask computation,
  `between`/`line` direction classification). Write a concrete asymmetric
  test case every time it shows up rather than trusting it by inspection.
- PR granularity from a plan is a starting point, not a contract. Related
  cleanup can land on the same branch when that makes a cleaner history,
  especially for docs-only or reorg-only changes meant to be squashed.

## Voice

No em dashes anywhere in this repo. Doc comments explain why, not what.

Read `docs/agents/voice.md` in full before writing doc comments, module
docs, README prose, config comments, or commit messages.

## Agent docs

| File                           | Read it when                                      |
| ------------------------------- | -------------------------------------------------- |
| `docs/agents/voice.md`         | writing any prose that lands in the repo           |
| `docs/agents/toolchain.md`     | running build, test, lint, bench, fuzz, self-play  |
| `docs/agents/issue-tracker.md` | creating, reading, or updating an issue            |
| `docs/agents/triage-labels.md` | applying a triage label                            |
| `docs/agents/domain.md`        | exploring the codebase or naming a concept         |
