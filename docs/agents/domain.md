# Domain documentation

Finite Mono is a multi-context monorepo. Before exploring or changing a
component:

1. Read the root `CONTEXT-MAP.md` when it exists and follow only the pointers
   relevant to the task.
2. Read the component's `CONTEXT.md` when it exists.
3. Read relevant system-wide ADRs under `docs/adr/`.
4. Read relevant component ADRs, such as `finite-brain/docs/adr/`.

Missing context documents are not an error. Create or update them lazily when
domain-modeling work resolves terminology, boundaries, or durable decisions.

Use glossary terms from the relevant `CONTEXT.md` in specifications, tickets,
tests, and code. If a proposed change conflicts with an ADR, name the conflict
explicitly instead of silently overriding it.
