---
name: finite-spec
description: "Turn an existing Finite design discussion into a specification in Linear. Use when the engineer asks to write or update the spec from agreed context."
---

# Finite Spec

Adapted from Matt Pocock's [to-spec](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/to-spec/SKILL.md).
See [LICENSE](../LICENSE).

Synthesize the current conversation and codebase understanding. This is not a
new interview. Read [issue-tracker.md](../../../docs/agents/issue-tracker.md),
[domain.md](../../../docs/agents/domain.md), and
[triage-labels.md](../../../docs/agents/triage-labels.md). Apply the relevant
AGENTS.md constraints and use the domain glossary vocabulary.

1. Explore the repo if needed to understand current behavior. Read the source
   issue, comments, and relevant decisions. Identify any unresolved conflict
   or approval gate rather than inventing a resolution.
2. Identify where tests will verify the feature's behavior. Prefer existing
   test boundaries at the highest useful level. Use as few boundaries as
   needed; ideally one. Confirm these testing decisions with the user.
3. Write the specification using the template below. Search for the existing
   plan and update it when it covers this work. Publish as an Engineering Plan
   in Linear within the user's authorized scope; keep draft-only output in
   the conversation.
4. Record the approval state. Use In Review when the required reviewers still
   need to accept the plan. Once they accept it, use Done according to the
   plan-completion rules in issue-tracker.md. Publishing a spec does not itself
   establish approval or make the plan an agent implementation assignment.
   Report the issue link and any pending reviews.

## Specification template

### Problem Statement

Describe the problem from the user's perspective.

### Solution

Describe the solution from the user's perspective.

### User Stories

Write an extensive numbered list covering all aspects of the feature:

1. As an <actor>, I want <behavior>, so that <benefit>.

### Implementation Decisions

Record the decisions made about modules, interfaces, architecture, schemas,
API contracts, and interactions. Identify ownership where it affects the
contract. Use behavior and module names rather than file paths or code that
will become stale. A small prototype snippet is appropriate when it expresses
a decision more precisely than prose; retain only that part and name its origin.

### Testing Decisions

State which external behaviors will be tested, the modules involved, and
relevant existing test patterns. Tests should verify behavior rather than
implementation details. Include the compatibility or recovery evidence required
by the applicable engineering principles.

### Out of Scope

State what this specification excludes.

### Further Notes

Include unresolved review questions, the required reviewers, and recorded
approval sources. Distinguish accepted decisions from proposals and measured
results from qualification that is still required.
