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

## Write the standing reason, not the deliberation

A comment earns its place by being true and useful later. Prose that argues
for a change, or recounts how it was decided, stops being either as soon as
the change is old news, and it does so silently.

The test: will this sentence still be worth reading once the thing it
describes is simply how the code is?

- **Keeps**: "X rather than Y, because Y would Z." A standing tradeoff, which
  a reader can check against the code in front of them.
- **Rots**: "the other half of a policy that was only ever half applied." A
  fact about this repo's history, not about the thing being commented.
- **Rots**: "two are already on a timer: `A` and `B`." Naming the current
  instances of a rule, which go wrong the moment either changes.
- **Rots**: "until this existed, nothing verified it", "the worst of the
  available options". Arguing for a decision to a reader who can only see the
  result.

This is not "don't record alternatives": the rule above about a rejected
alternative still holds, and is the first bullet here. The line is between a
property that stays true and a change that already happened.

Commit messages and PR descriptions are where the deliberation belongs. They
are dated by construction, a reader reaches them deliberately, and nothing
there has to stay true. Moving a paragraph from a comment into the commit
message is usually the right fix, not deleting it.

## No issue or PR numbers in source

`#54`, `see #26`: these rot as the repo evolves (issues close, get
renumbered across forks, aren't visible to a reader without GitHub
access), and the reference does explanatory work the prose should be
doing itself. If a comment needs the issue number to make sense, the
comment is incomplete. State the reasoning inline and let the number live
in commit history and the PR description, where it belongs and won't go
stale.

Two things that look like this and are fine. A *qualified* reference to
another project's tracker (`rust-lang/rust#143874`) is stable, resolvable
without access to this repo, and usually the only honest way to say "waiting
on upstream". And `#N` in backticks is chess notation for mate in N, which an
engine has every reason to write.

## Nothing here is checked by a tool

Every rule in this file is a matter of judgement and is caught in review, or
not at all. That is the reason it is worth reading rather than skimming.

Two of them (em dashes, and bare issue references in `.rs` source) are narrow
enough that a script once checked them, and that script was deleted rather
than maintained. Mechanising two rules out of nine bought a green tick that
said nothing about whether the prose was any good, and it drew attention to
the two cheapest rules at the expense of the seven that decide the answer.

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
