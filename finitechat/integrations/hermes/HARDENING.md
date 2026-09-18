# Hermes runtime safety

The adapter stays thin: Rust owns identity, MLS state, delivery cursors, inbox
leases, and reply routing. See the [integration contract](README.md) and
[Runtime control contract](../../../finitecomputer-v2/docs/runtime-control-contract.md).

- Restart preserves `/data`, including identity, chat state, Hermes memory,
  workspace, tools, and user-installed skills.
- Boot recovery runs before seed/init logic. A recover-known-good boot requires
  the existing durable roots and identity/store/config files; missing state
  fails closed rather than creating another Agent Principal.
- Repair is bounded to image-owned integration/configuration state. It cannot
  reset chat stores, replace keys, or overwrite user configuration or skills.
- Provider credentials stay outside chat messages and application logs.
- Readiness requires the resident Finite Chat service, not merely a live container.
- Same-volume restart is not backup or empty-target restore proof.

Contract coverage lives in `tests/`, `../../tests/container/`, and the Runner
and Core tests. Use the root CI harness selectors and component recipes for
current checks. Outstanding lifecycle and recovery work is tracked in
[FIN-21](https://linear.app/finitecomputer/issue/FIN-21) and
[FIN-62](https://linear.app/finitecomputer/issue/FIN-62).
