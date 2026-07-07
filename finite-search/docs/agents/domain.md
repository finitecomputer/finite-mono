# Domain Docs

How engineering skills should consume this repo's domain documentation.

## Before Exploring

Read:

- `CONTEXT.md` at the repo root.
- Relevant ADRs under `docs/adr/`.
- The run ledger under `docs/feature-dev/` when continuing an active feature.

## Layout

This is a single-context repo:

```text
/
├── CONTEXT.md
├── docs/adr/
├── docs/runbooks/
└── scripts/
```

If a future agent adds `CONTEXT-MAP.md`, it must first add an ADR explaining why
this repo stopped being a single-context repo.

## Vocabulary

Use the terms from `CONTEXT.md` in issue titles, PRDs, commits, docs, and
review findings. If a concept does not exist there yet, either add it during a
context update or avoid inventing new vocabulary.

## ADR Conflicts

If a proposed change contradicts an ADR, call it out directly and either update
the ADR with a superseding decision or stop for a human decision.

