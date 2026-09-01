# Issue tracker: GitHub

Issues and specs live as GitHub issues on `cdprice02/turox`. Use the `gh`
CLI; it infers the repo from `git remote -v` when run inside a clone.

## Conventions

- **Create**: `gh issue create --title "..." --body "..."` (heredoc for
  multi-line bodies).
- **Read**: `gh issue view <number> --comments`.
- **List**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`,
  with `--label` and `--state` filters as needed.
- **Comment**: `gh issue comment <number> --body "..."`
- **Label**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

"Publish to the issue tracker" means create a GitHub issue. "Fetch the
relevant ticket" means `gh issue view <number> --comments`.

Issue bodies are prose that lands in the project, so `docs/agents/voice.md`
applies to them.

## Pull requests as a triage surface

**PRs as a request surface: no.** This is a solo repo; PRs originate here
and aren't incoming requests needing triage. Flip to `yes` if that
changes, and `/triage` will run PRs through the same labels and states
using the `gh pr` equivalents.

GitHub shares one number space across issues and PRs, so a bare `#42` may
be either: resolve with `gh pr view 42`, fall back to `gh issue view 42`.

## Wayfinding operations

Used by `/wayfinder`. The map is one issue; tickets are its children.

- **Map**: an issue labelled `wayfinder:map` holding the Notes,
  Decisions-so-far, and Fog body.
- **Child ticket**: a GitHub sub-issue of the map (`gh api` on the
  sub-issues endpoint), labelled `wayfinder:<type>` (`research`,
  `prototype`, `grilling`, `task`).
- **Blocking**: native issue dependencies.
  `gh api --method POST repos/cdprice02/turox/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`,
  where `<blocker-db-id>` is the blocker's numeric database id from
  `gh api repos/cdprice02/turox/issues/<n> --jq .id`, not its `#number`
  and not its `node_id`. A ticket is unblocked when
  `issue_dependencies_summary.blocked_by` reaches zero.
- **Frontier**: the map's open children, minus any with an open blocker
  or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me`.
- **Resolve**: comment the answer, close the issue, then append a context
  pointer to the map's Decisions-so-far.
