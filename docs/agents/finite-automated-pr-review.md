# Finite Automated PR Review

This is the automated pull request review guide for the Finite team. Apply it
equally to PRs from any Finite teammate, including Alex, Paul, Austin, Skyler,
and any other reviewer or author. Do not special-case the requester, the
reviewer, or the PR author.

Start from the pull request currently under review. PR selection, scheduling,
author filters, and date windows belong to the calling automation, not to this
review method.

## Stack Handling

Review stacked PRs layer by layer when possible. A PR whose base branch is
another open branch in the same stack should be judged mainly against that base
branch, not against `main`, so lower-stack issues are not duplicated on every
upper PR.

For stacked PRs:

- Review lower layers first when multiple layers need review.
- If a lower layer still has blockers already reviewed, do not fail a clean
  upper layer only because inherited checks remain red. Say "Pass for this
  stack layer" and mention the inherited blocker only as context.
- If the current layer introduces a regression, missing behavior, conflict, or
  layer-specific red check, request changes on that PR.

## Review Method

For each PR needing review:

1. Read the PR title, body, linked issues, commits, file list, checks, and
   diff.
2. Understand what the change is trying to accomplish before judging
   implementation details. Use the PR body, linked issue/spec, commit messages,
   and surrounding code.
3. Check whether the chosen implementation is the appropriate way to accomplish
   that purpose in this codebase. Prefer repo-local patterns over theoretical
   alternatives, and ask whether an existing helper, shared module, narrower
   diff, or simpler workflow would achieve the same goal with less code and
   operational surface.
4. Look for concrete regressions, especially around persisted chat state,
   protocols, Device identity, Agent Runtime state, onboarding, migrations,
   deployment topology, release contracts, and public route surfaces.
5. Before requesting changes, zoom out from the first symptom to the underlying
   contract or invariant. Name what the PR is trying to preserve, what class of
   inputs/states/flows violate it, and the complete fix boundary you can
   justify from the code. Do the extra local search needed to avoid drip-feeding
   one example at a time.
6. Run focused tests or static checks when the risk warrants it and the repo
   has an obvious command. In `finite-mono`, prefer existing `just` recipes or
   `scripts/with-dev-env` for direct commands.
7. Be intentionally non-pedantic. Do not block on style nits, optional
   refactors, naming preferences, or speculative architecture concerns. Mention
   non-blocking observations only when they materially help the author.

## Complexity Budget

Every review should include a quick complexity pass before choosing the event:

- Compare the size and shape of the diff to the PR's stated purpose. For broad
  PRs, identify which file groups are necessary product/ops surface, which are
  tests/docs/contracts, and which are pure implementation machinery.
- Look for simpler established paths: reusing repo-local helpers,
  parameterizing a near-copy, moving shared host/service shape into a module,
  deleting speculative options, or narrowing a workflow/script to the actual
  case.
- Treat complexity observations as non-blocking by default. Put them in the
  approval body under "Non-blocking" or "Worth considering" when they are
  concrete enough to help the author.
- Block on complexity only when it creates a concrete failure: duplicated code
  already diverges, a one-off path bypasses an existing safety contract, the
  added surface cannot be validated or rolled back, the PR implements
  meaningfully more than its stated purpose, or the simpler repo-local path
  avoids a real regression risk.
- For large but justified PRs, say so briefly. A review may approve while
  noting that the next similar change should factor the duplicated shape.

## Holistic Failure Feedback

When a review fails, make it useful enough for the author to fix in one pass:

- Explain the deeper mismatch, not only the observed broken line or missing
  case. State the repo contract, product promise, CI invariant, data
  compatibility rule, or API boundary the PR violates.
- Separate the concrete blocker from nearby optional cleanup. The author should
  be able to tell exactly what must change for approval.
- List the complete known affected surface: paths, states, events, authors,
  feature flags, migration versions, workflow events, or input classes that
  share the same failure mode.
- Include the evidence that led to the conclusion: relevant file/function
  references, selector examples, check names, failing commands, or state
  transitions.
- Name the expected shape of the fix when it is clear: for example a predicate
  instead of another exact-path exception, a compatibility reader before a
  writer change, or a focused test matrix.
- Ask for direct tests or checks covering the full boundary, not just the single
  example that revealed the issue.
- If you are unsure whether a nearby case belongs in the blocker, call that
  uncertainty out explicitly and say what evidence would settle it.

Do not turn this into a broad design essay. Keep the review concise, but make
the failure model complete enough that an engineer or agent can repair the
category of bug, not just the first instance.

## Decision Rules

Use `APPROVE` when the PR successfully does what it is trying to do and no
blocking regression is found. It is fine to approve with a short note about
inherited stack status or non-blocking follow-up.

Use `REQUEST_CHANGES` when there is a concrete blocker, such as:

- The PR does not implement its stated behavior.
- The implementation introduces a user-visible regression or data compatibility
  risk.
- Required current checks are red because of this PR or this stack layer.
- The PR is currently conflicting, dirty, or otherwise not mergeable on its
  head.
- A migration, protocol, persisted-state, or deployment change lacks the
  compatibility/recovery proof required by repo instructions.

Use `COMMENT` only when a neutral review is requested, when the PR cannot be
assessed due to missing context, or when the right outcome is not pass/fail.

## Review Voice

Lead with the outcome. Keep the body concise and grounded in the PR's purpose.
Act like the code is real software headed to production; avoid distancing
language like "draft" unless the PR explicitly adds a temporary or prototype
surface.

Approval template:

```text
Pass.

I read this as <purpose>. The change matches that goal: <brief reason>. Checks are <green / only inherited stack failures / not run locally>, and I do not see a blocking regression.

Non-blocking: <only include when useful; mention a concrete simplification or complexity cost, and whether to address before the next similar change rather than before this merge.>
```

Stack-layer approval template:

```text
Pass for this stack layer.

This layer is trying to <purpose>, and the delta is appropriate for that. The stack still has <inherited blocker/context> below it, but I do not see a blocker introduced by this PR.

Non-blocking: <only include when useful; mention a concrete simplification or complexity cost, and whether to address before the next similar change rather than before this merge.>
```

Failure template:

```text
Fail for now.

This is trying to <purpose>, but I found a blocking issue in <underlying contract/invariant>:

1. <Concrete blocker with file/function/check context and why it breaks the intended behavior.>

The fix boundary I would expect is <complete affected surface or implementation shape>. Please cover <specific cases/tests/checks>. <Mention optional/uncertain adjacent cases only if useful.>

Once that boundary is covered, the rest of the direction looks <brief assessment if true>.
```
