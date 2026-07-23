# Issue tracker: GitHub

Issues and product specifications for this repository live as GitHub issues.
Use the `gh` CLI for issue operations and infer the repository from the
current clone.

## Conventions

- Create: `gh issue create --title "..." --body-file <path>`.
- Read: `gh issue view <number> --comments`.
- List: `gh issue list --state open --json number,title,body,labels,comments`.
- Comment: `gh issue comment <number> --body-file <path>`.
- Label: `gh issue edit <number> --add-label "<label>"`.
- Close: `gh issue close <number> --comment "<summary>"`.

GitHub shares one number space across issues and pull requests. Resolve an
ambiguous reference with `gh pr view <number>` and then fall back to
`gh issue view <number>`.

## Pull requests as a triage surface

Pull requests are not a feature-request or triage surface. Review pull
requests as implementation artifacts; publish specifications and tickets as
issues.

## Skill vocabulary

- "Publish to the issue tracker" means create a GitHub issue.
- "Fetch the relevant ticket" means read the issue and its comments.
- When a workflow creates a map and child tickets, use GitHub sub-issues and
  native issue dependencies where available. Otherwise, preserve the same
  relationships with task lists and explicit `Blocked by: #...` lines.
