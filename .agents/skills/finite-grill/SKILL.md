---
name: finite-grill
description: "Grill a Finite plan or design, resolve domain terms, and capture decisions in Linear. Use when an engineer asks to examine an idea before specifying or building it."
---

# Finite Grill

Adapted from Matt Pocock's `grill-with-docs`, `grilling`, and `domain-modeling`
at [3cca18b](https://github.com/mattpocock/skills/tree/3cca18b368ae95cdbdebbff572ccafa662551015).
See [LICENSE](../LICENSE).

Use the project's AGENTS.md for engineering constraints. Read
[domain.md](../../../docs/agents/domain.md) for glossary and decision handling,
and [issue-tracker.md](../../../docs/agents/issue-tracker.md) for Linear and
approval rules. Start from the relevant issue and its discussion when one exists.

Interview the user until you reach a shared understanding. Map the discussion
as a **design tree**: every decision leads to the decisions that depend on it.

Work in rounds. The **frontier** is the set of questions whose prerequisites
are settled. Ask the whole frontier in one round, number each question, and
give your recommended answer. Then wait for the user's answers.

```text
Q1 — <Question title>: <Question and relevant choices>
Recommendation: <Your recommended answer and reason>

Q2 — <Question title>: <Question and relevant choices>
Recommendation: <Your recommended answer and reason>
```

Each answer changes the tree. Resolve what it settles and compute the next
frontier. A question that depends on an unanswered question belongs to a later
round. Read prior notes so you do not ask resolved questions again.

Finding facts is your job. Inspect the code and relevant records rather than
asking the user for facts you can retrieve. Independent exploration can run
in parallel when delegation is available. While a fact is unresolved, ask only
questions that do not depend on it. Decisions belong to the user and the
required reviewers; do not infer an absent owner's agreement.

Apply the domain-modeling rules as terms and decisions become clear. Challenge
conflicting vocabulary, test concrete scenarios, and capture resolved context
in its current authorized destination. Scale the questions to the change's
relevant engineering principles and ownership boundaries.

The grill ends when every identified decision is resolved or explicitly
deferred with its consequence and owner, and the user confirms the shared
understanding. Hand off the agreed context and any remaining review gates.
Specification, ticket publication, and implementation remain separate actions.
