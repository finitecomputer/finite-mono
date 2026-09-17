---
name: finite-tickets
description: "Split a Finite plan, specification, or agreed conversation into small implementation tickets in Linear, with acceptance criteria and native dependencies."
---

# Finite Tickets

Adapted from Matt Pocock's [to-tickets](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/to-tickets/SKILL.md).
See [LICENSE](../LICENSE).

Read [issue-tracker.md](../../../docs/agents/issue-tracker.md),
[triage-labels.md](../../../docs/agents/triage-labels.md), and
[domain.md](../../../docs/agents/domain.md). Use the glossary and apply relevant
AGENTS.md constraints.

## 1. Gather context

Use the plan, specification, or conversation supplied by the user. Fetch a
referenced issue's full body, comments, and relationships. Check the approval
record; a Done status alone cannot replace the required approval evidence.
An agreed conversation can supply approval within the engineer's authority.
If required review remains open, draft the breakdown and identify that gate
before publishing ready Build tickets.

Read existing child and related issues before splitting the work. Reuse the
approved breakdown on a repeat run; reconcile changed scope rather than
creating duplicate tickets. New behavior outside the approved source remains
proposed until the affected approval is updated. Existing approved tickets
remain valid unless that change affects them.
Explore the codebase if it is not already understood.
Look for useful prefactoring: make the change easy, then make the easy change.

## 2. Draft vertical slices

Each ticket is a **tracer bullet**: a narrow, complete path through the layers
needed for that behavior, including its tests. It is independently demonstrable
or verifiable and small enough for one fresh agent context. Put necessary
prefactoring first. Declare the tickets that block each slice; no blockers
means it can start once selected for work.

A wide mechanical refactor is the exception. If no vertical slice can remain
green, use **expand–contract**: add the new form, migrate callers in bounded
batches, then remove the old form. Each batch depends on the expansion; removal
depends on every batch. If even batches cannot remain green alone, make the
shared integration branch and final integration-and-verification ticket
explicit. State where green CI is promised.

## 3. Review the breakdown

Show a numbered list with each ticket's title, blockers, and delivered behavior.
Ask whether the size is right, whether each blocker is necessary, and whether
tickets should be merged or split. Revise until the user approves the breakdown.

## 4. Publish

Publish approved Build issues in dependency order. Use native blocking links
and the source plan as parent when appropriate. Include the specification link
in each ticket. Apply `ready-for-agent` using the configured mapping unless
the work requires the human route or the user directs otherwise. Report missing
workspace setup rather than silently claiming the tickets are classified.

Keep the parent specification and its status unchanged. Parent links must not
stand in for blocking links. Read back created or updated issues and report
their identifiers. Work the **frontier**: selected tickets whose blockers are
complete, following triage-labels.md's work-selection rule.

## Ticket template

### Parent

Link the source plan or specification, when one exists.

### What to build

Describe the end-to-end behavior from the user's perspective.

### Acceptance criteria

- [ ] Independently verifiable criterion.
- [ ] Independently verifiable criterion.

### Blocked by

Link the blocking tickets, or state "None." Set matching native relationships.

Use module names, contracts, and behavior rather than file paths or code that
will become stale. Retain a small, attributed prototype snippet only when it
expresses a decision more precisely than prose.
