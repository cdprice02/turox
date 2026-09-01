# Documentation voice

Applies to doc comments, module docs, README, commit messages, CI and
config comments: everywhere in this repo, not just source code.

## No em dashes

Use a colon, semicolon, comma, parentheses, or restructure the sentence.
This is listed first because it is the rule that actually slips: it once
went through at scale in generated Rust doc comments before being caught.

## Don't cite config from source

State the reasoning inline instead. A doc comment that defers to
`docs/agents/voice.md` or `CLAUDE.md` is skipping the explanation it owes
its reader, who is looking at a function, not at the repo's agent config.

## No issue or PR numbers in source

`#54`, `see #26`: these rot as the repo evolves (issues close, get
renumbered across forks, aren't visible to a reader without GitHub
access), and the reference does explanatory work the prose should be
doing itself. If a comment needs the issue number to make sense, the
comment is incomplete. State the reasoning inline and let the number live
in commit history and the PR description, where it belongs and won't go
stale.

## Module docs stay short

What is in the module, and how the pieces relate to each other or to
other modules. Function-specific reasoning (a gotcha, a design tradeoff,
a bug a specific function caused) belongs on that function's own doc
comment, not in a module-level "# Design" section the reader has to jump
to and back from. Cross-cutting reasoning that genuinely spans several
functions is the one exception worth keeping at module level.

## Doc comments explain why, not what

A doc comment that walks through a function's control flow step by step,
or narrates what the signature and the body a few lines below already
make obvious, is bloat to delete rather than to trim. Reserve doc
comments for what a reader genuinely can't get from the code: a hidden
invariant, an alternative considered and rejected, a subtle ordering
requirement, a bug a specific input shape caused before.

Test it before writing one: if the doc comment could be deleted without a
future reader losing anything a plain read of the body would already tell
them, it is too long. This was caught at scale in
`turox-engine/src/search/negamax.rs`, which reached 30% doc-comment lines
by file, nearly all of it restated control flow rather than load-bearing
reasoning.

## Fix stale "not yet implemented" notes

Docs here get written design-doc-first, before the implementation exists,
so "not yet implemented," "still-unimplemented," "stubbed pending X," and
"without X yet" are correct when written and routinely stale by the time
the PR that implements X lands two PRs later. When touching a file for
any reason, check nearby docs for this pattern and fix it if the thing
they are waiting on now exists, rather than only fixing the doc tied to
the current change.

## Pass over what you touch

When touching a function or file for any other reason, check it against
the rules above: the comments load-bearing to the change or immediately
adjacent to it, not an audit of the whole file on every unrelated touch.
