# Repository documentation

This tree documents implemented contracts, development and current operations.
Product plans, decisions in progress and outstanding work live in
[Linear](https://linear.app/finitecomputer). Git history retains old plans and
incident narratives; they are not current operating instructions.

- [Monorepo rules](monorepo-doctrine.md)
- [Local development and integration tests](local-integration-harness.md)
- [Component vocabulary](../CONTEXT-MAP.md)
- [Production operations](../infra/runbooks/README.md)
- [Recovery invariant](adr/0001-recoverability-precedes-operator-blindness.md)
- [Issue tracker conventions](agents/issue-tracker.md)

Keep one authoritative explanation per contract. A missing capability may be
marked TODO with its Linear issue; implementation phases and migration plans
do not belong in repository documentation.
