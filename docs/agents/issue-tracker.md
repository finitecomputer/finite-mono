# Issue tracker: Linear

Use the [Finite workspace](https://linear.app/finitecomputer), normally the
Finite Engineering team (`FIN`), for engineering requests, specifications,
decisions, and follow-up work. GitHub hosts code, pull requests, and releases.
Use the authenticated Linear tools available in the current agent. If access
is unavailable, keep the proposed output in the conversation and report what
could not be read or published.

## Read and publish

- Read the issue body, comments, labels, status, and relationships before
  changing it. Read historical GitHub issues when referenced.
- Search by behavior and domain concept before creating an issue. Use an
  existing issue when it covers the same work; ask when the scopes differ.
- Re-read an issue before updating it so another engineer's changes survive.
  After publishing, read back the changed content and relationships.
- Keep the current specification and acceptance criteria in the issue body.
  Comments hold discussion, approval, and dated evidence. When an approved
  decision changes, update the governing text and identify affected tickets.
- Use native parent, sub-issue, blocking, related, and duplicate relationships.
  A parent relationship alone is not a dependency.
- Resolve identifiers explicitly: `FIN-123` is a Linear issue. GitHub issue
  and PR numbers belong to their named repository. Link the Linear issue from
  the implementation PR. PRs are implementation review artifacts, outside the
  request-triage queue.

## Plans, approval, and delivery

An **Engineering Plan** delivers a specification or a scoped design decision.
A **Build** issue delivers implementation, qualification, or rollout work.
Read [triage-labels.md](triage-labels.md) for their label mapping.

| Status | Engineering Plan | Build |
| --- | --- | --- |
| Todo | Design work is selected but not started | Delivery work is selected but not started |
| In Progress | Resolve the design | Implement or qualify the change |
| In Review | Required design review is pending | Review the implementation or delivery evidence |
| Done | Required reviewers accepted the plan | The issue's acceptance criteria are met |

The invoking engineer can approve work within their authority. Read existing
approval requirements before applying that rule. Changes to another
component's contract need the affected owner's agreement. For V3,
[FIN-40](https://linear.app/finitecomputer/issue/FIN-40) governs final-spec
review; one person's acceptance of direction does not establish wider approval.
Record who approved which scope, with a link to the approval or an explicit
record of the invoking engineer's decision. Material changes reopen the
affected review and readiness decision.

An accepted plan remains the specification after it is Done. Build issues
track delivery. A delivery parent, such as a rollout issue, stays open until
its delivery criteria are met. Completion of a process plan does not approve
all component plans that it references.

Before closing a plan with unfinished children, verify that Linear's sub-issue
auto-close setting will preserve them. If that cannot be verified, record the
approval and explain why the status change is pending. Keep design-only child
decisions distinct from Build children when checking which approvals are due.
`finite-tickets` creates Build issues without closing or rewriting the plan.

Spec approval permits the approved work; production operations still follow
the authorization rules in AGENTS.md. Implementation merged, artifact
released, and production qualified are separate facts when the ticket's
acceptance criteria distinguish them.
