# FiKnight account transfer — 2026-09-02

This is the operator ledger for moving the existing `Austin Finite` Project to
the dedicated `fiknight@finite.vip` account. The operation is run from this
unmerged draft branch. No application code, image, or NixOS generation is
deployed.

## Exact identity

- Project: `project_b7e3a5beaf06095c6465`
- Runtime: `runtime_d8ceb9b4f4e9bacb85b0`
- machine: `finite-kata-9edb9d1d2e2ce1c9073f` on `finite-lat-3`
- Runtime artifact: `finite-agent-runtime-2026-08-29.5`
- state schema: `runtime-state-v1`
- Agent Principal: `npub1r83u6s59v5956l5gd6my6vjqk9x0rkjef78ntchs494m5y6tq4dqychqrv`
- source account: `austin@finite.vip`
- target account: `fiknight@finite.vip`
- source name/NIP-05: `Austin Finite` / `austin-finite-b7e3a5beaf06095c@finite.vip`
- target name/NIP-05: `FiKnight` / `fiknight@finite.vip`

The target Google Workspace and WorkOS account was created as `Fiknight
Finite`. Core linked it as `user_b9540ab702bd98195b98` with personal
organization `org_696a800e548d65b8be93`. No credential value belongs in this
ledger or branch.

The fresh pre-change coordinated Recovery Snapshot is
`/data/recovery-snapshots/hosted-web-chat/20260902T142826Z`. Its manifest
passed in full. SHA-256 evidence:

- manifest: `053512249efce2f0001d41717977b1200f027f831a0229dfd2528b01f66132cd`
- Core dump: `529248577465112bad76a1a4fd1c15d3ecf3846a75d83d52eb33d7209f1357db`
- Finite Identity database: `546f32284902cc0e6502bdb8a28eafc9bb09f798ab3993460d0330ee66dd1f6a`

The exact Core dump was restored into an isolated local PostgreSQL instance.
The production rows passed stage, exact replay, rollback, a second stage, and
finalization without modifying production.

`fiknight@finite.vip` is intentionally both a deliverable Google mailbox and a
Managed Agent NIP-05. Gmail remains deliverable. Finite Sites callers must use
the typed `--nip05 fiknight@finite.vip` form for Agent grants rather than the
typed `--email` form.

## Recovery and ordering

1. Require a fresh successful `finite-hosted-web-chat-snapshot.service` run.
   Record its directory and verify its manifest. This is the pre-change
   rollback boundary for Core, Chat, Hosted Device, and Finite Identity.
2. Confirm the Runtime `/contact` document returns the exact Agent Principal
   above and that `fiknight@finite.vip` is still unbound.
3. Run the staged Core transaction from the workstation:

   ```sh
   ssh finite-lat-2 'sudo -u postgres psql -d finite_core' \
     < scripts/ops/fiknight-account-transfer-stage.sql
   ```

   This transfers the Project, creation request, and scoped Finite Private key;
   creates FiKnight's deterministic hosted Chat identity and active owner
   membership; and deliberately keeps Austin's membership active.
4. In FiKnight's isolated browser session, open the existing Project. Require
   the existing conversation history and a successful new message/reply before
   continuing. If this fails, run the rollback SQL before creating the new
   NIP-05 binding.
5. Recreate only this Runtime with its existing digest-pinned artifact through
   the existing exact rollout command. This blue/green path preserves the
   durable state, verifies the Agent Principal, and refreshes the three runtime
   name variables to `FiKnight`. First run `--plan-only`; the execution must
   name only the exact Project, Runtime, host, machine, and current artifact.
6. Recheck `/contact`, the Runtime's three name variables, chat history, and a
   new message/reply.
7. Bind `fiknight@finite.vip` to the exact Agent Principal through Finite
   Identity's loopback operator endpoint. This is the public identity commit
   point: the name is intentionally durable and non-reassignable. Keep the old
   NIP-05 active as a rollback alias during observation.
8. Require both public NIP-05 routes to resolve `fiknight` to the exact Agent
   Principal. Then run:

   ```sh
   ssh finite-lat-2 'sudo -u postgres psql -d finite_core' \
     < scripts/ops/fiknight-account-transfer-finalize.sql
   ```

   This archives only Austin's old Project membership. It does not delete the
   user, history, Runtime, key material, or old NIP-05.
9. From FiKnight's Connections page, start a fresh Google Workspace OAuth
   authorization. The old Austin Workspace token is not copied. Confirm the
   connected address is exactly `fiknight@finite.vip`, then make one read-only
   Drive or Calendar request through the Agent.

## Rollback boundary

Before the new NIP-05 is bound, run
`scripts/ops/fiknight-account-transfer-rollback.sql` to return Project control,
the creation request, the scoped inference key, and active Chat membership to
Austin. The retained FiKnight user, personal organization, inactive membership,
and unused grant are harmless audit state.

If the same-artifact replacement already completed, first restore the Core
state with the rollback SQL and then run the same exact replacement again so
the Runtime name variables return to `Austin Finite`. Do not disable the new
NIP-05 as an automatic rollback: a disabled v1 binding cannot be silently
re-enabled.

## Acceptance

- Project owner and organization resolve only to `fiknight@finite.vip`.
- Austin's Project membership is archived; FiKnight's is active and `owner`.
- Project, Runtime, machine, artifact, schema, Agent Principal, durable state,
  rooms, messages, and message authorship are unchanged.
- Project display and all three runtime name variables equal `FiKnight`.
- The active Project-scoped Finite Private key belongs to FiKnight's grant.
- `fiknight@finite.vip` resolves as `managed_agent` to the exact Agent
  Principal through `identity.finite.vip` and the `finite.vip` apex route.
- FiKnight's Google Workspace connection reports exactly
  `fiknight@finite.vip`; Austin's connection was not copied.
