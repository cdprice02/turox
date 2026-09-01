# Domain docs

What to read before exploring the codebase, and how to use it.

## Read first

- `README.md`: the architecture and the module boundaries. This exists
  and is accurate; start here.
- `CONTEXT.md` at the repo root: the glossary of domain terms.
- `docs/adr/`: decision records touching the area being worked in.

The last two don't exist yet. If a file is absent, proceed silently:
don't flag it, and don't propose creating it upfront. `/domain-modeling`
creates them lazily, when a term or a decision actually needs pinning
down.

Chess brings a large amount of established public vocabulary with it
(perft, quiescence, en passant, magic bitboards, SPRT). That vocabulary
is not this project's domain model and doesn't belong in a glossary; a
`CONTEXT.md` here would be for terms turox uses in a way the chess
programming literature doesn't.

## Use the glossary's vocabulary

When output names a domain concept (an issue title, a refactor proposal,
a hypothesis, a test name), use the term as `CONTEXT.md` defines it. A
concept missing from the glossary is a signal: either the language is
being invented and should be reconsidered, or there is a real gap worth
noting for `/domain-modeling`.

## Flag ADR conflicts

If output contradicts an existing ADR, say so rather than silently
overriding it: "contradicts ADR-0007, but worth reopening because ...".

An ADR is for a decision with a rejected alternative worth recording. It
is not for restating architecture the README already covers.
