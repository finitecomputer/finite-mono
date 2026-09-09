# FIN-37: bounded, pull-based Connections

Status: direction and implementation boundaries agreed with Paul on 2026-09-09.
Detailed architecture decisions below are pending. This is the implementation
plan, not evidence that the feature is implemented or ready to deploy.

## Problem and outcome

Connections currently depends on Finite Chat for status and configuration.
A chat outage can prevent managing connections, and this dependency blocks
finitechat retirement. Remove that dependency in the existing product while
preserving current settings, user overrides, chat history and recovery.

Core participates in every Connections management flow. The dashboard writes
intended Connections changes to Core. An agent-resident component fetches its
own desired state, applies revisions through existing local operations, and
reports observed status to Core. The dashboard reads that status from Core.
The agent continues running its existing configuration while Core is unavailable.

This is bounded configuration management, not a universal Hermes reconciler.
Core owns only settings explicitly managed through Connections; preserve all
other configuration. Treatment of conflicting local edits inside that managed
scope requires the decision checkpoint below. Do not silently turn periodic
fetches into permission to overwrite local state.

## Agreed boundaries

- Reuse agentd's local validation, configuration offers, ownership tracking,
  durable ledger, rollback and process supervision where their contracts fit.
  The shared-operation extraction in PR #859 may help; reassess it before reuse.
- Apply changes by revision. Repeated fetches are not repeated user actions.
  Desired, applied and observed status must not be conflated.
- Keep Core responsible for account/Project authority and desired/status
  records. Keep knowledge of Hermes configuration paths and effectors local to
  the agent. The browser cannot submit arbitrary YAML, environment keys, shell
  commands, filesystem paths or upstream addresses.
- Use an outbound agent-to-Core path independent of chat readiness. No inbound
  management listener, runner-address registry, WireGuard dependency, dedicated
  connection-manager sidecar or general-purpose command bus for this work.
- Agent authentication and secure secret delivery are still required. Pull
  removes inbound machine credentials, not all authentication requirements.
- Keep Runtime lifecycle with Core/Runner. Do not add Connections operations
  to the generic Runtime Management Pipe or claim an existing config/skills
  polling channel without verifying the code.
- Public HTTPS for hosted Hermes web/Desktop is FIN-39's separate client-access
  requirement. It does not block FIN-37 implementation. FIN-58 owns UI/hot reload
  and later user chat/history cutover; FIN-59 owns eventual finitechat retirement.
- Build production components with reproducible local tests. Temporary test
  fixtures must be identified; no alternate demo implementation or placeholder
  provisioning path may stand in for a shipping component.

## Phases and exit criteria

### 1. Contract and through-line review, before implementation

Trace current dashboard actions, authorization, local settings/credential
writers and readers, startup, retries, rollback and recovery. Compare existing
agentd operations with the proposed revision-driven flow. Record reusable
behavior and gaps; do not rewrite proven operations merely to fit a new API.

Prepare a small contract proposal with payload examples, ownership boundaries,
state transitions and failure cases. Consult Paul on the architectural decisions
listed below before implementing the affected contract. The smallest adequate
GET desired / POST status shape is a starting point, not finalized endpoint
names, schemas, storage or a promise that two routes alone solve the problem.

Exit: agreed contract decisions recorded here or in a linked design document,
with the approving conversation/date. No security, authority or delivery-policy
choice is inferred from approval of this overall direction.

### 2. One real inference-setting slice

Implement the agreed Core persistence and owner-authorized dashboard path,
agent authentication/fetch/report path, and bounded revision application using
existing agentd/Hermes behavior. Make the agent-resident loop independent of
chat initialization. Show pending, applied and failure states accurately.

Use actual Core, migrated Postgres, agentd and pinned Hermes locally. External
identity/provider fixtures may be used when named explicitly; they do not prove
those external integrations. No mocked Core/agentd/Hermes handler counts as the
end-to-end acceptance proof. A runtime restart in this proof must use the actual
entrypoint/startup path being changed, or the gap remains open.

Exit: the acceptance matrix below passes and is reproducible from a documented
command. Report exactly what was proved and what remains unqualified.

### 3. Current Connections operations and migration

Extend the reviewed contract to Google, Telegram and SimpleX, including the
existing pairing/approval/reset flows. Stop for agreement before adding a
one-shot operation mechanism or reinterpreting those operations as desired
state. Do not quietly add a queue as a workaround.

Define and prove how existing locally configured connections enter the managed
scope. An absent Core record must not silently erase an existing connection.
Test real provider behavior where it is part of the claim. Define behavior for
unknown versions/types, removed connections and credential revocation.

Exit: all current user capabilities work without finitechat, with explicit
existing/new-agent, mixed-version, credential and recovery evidence.

### 4. Shipping and eventual cleanup

Reviewed inactive Core/dashboard foundations may merge and deploy separately
when compatibility and rollback boundaries are clear. Agent runtime changes
join FIN-57's combined R1 candidate with FIN-39's Hermes serving/auth prerequisites.
No per-PR, per-provider, prototype or admin-toggle agent rollout.

After qualification and explicit production authorization, canary then promote
the same immutable digest. Record every agent restart/replacement, including
canaries. Run scripts/finite-status before and after rollout. Coordinate with
FIN-56's Sites candidate if both are ready, without an indefinite dependency.

Remove the old chat configuration transport only after the replacement and
migration are qualified. Preserve history and recovery material. FIN-59's later
runtime removal should join another planned release if R2 is needed. Track
merged, built, deployed and enabled separately.

## Local acceptance matrix

| Situation | Required evidence |
| --- | --- |
| Authorized inference change | Dashboard writes Core state; the actual agent fetches it; pinned Hermes validates/applies it; observed status names the correct applied revision. |
| Chat unavailable | The entire flow works with chat initialization/transport unavailable and without chat identity, room, hosted-device binding or owner-claim delivery. |
| Agent offline | Core retains the requested change; dashboard shows it pending; it applies when the agent returns. |
| Invalid configuration or failed activation | Honest failure status; the last working settings and credentials remain usable or are restored. |
| Lost report / repeated fetch | No repeated destructive effect or unnecessary restart; durable state supports the agreed retry behavior. Do not claim exactly-once external effects without proof. |
| Agent or reconciler restart | Resume the agreed revision correctly and preserve existing state. |
| Rapid revisions / stale status | Earlier reports cannot make the UI claim the newest revision was applied; ordering/cancellation follows the agreed contract. |
| Authorization | Wrong owner and cross-agent reads/writes/status reports fail closed; auth is independent of chat. |
| User/Hermes local changes | Unmanaged fields survive; managed-field conflicts follow the explicitly agreed policy. |
| Core or network outage | Existing agent operation continues; retry/backoff and later convergence are bounded. |
| Existing/new versions and state | Safe migration, unsupported-version behavior and rollback; an all-candidate test is not compatibility proof. |

## Stop and consult Paul on major architectural decisions

This is an explicit user instruction. Before dependent implementation, present
the concrete choice, alternatives, recommendation and consequences; wait for
Paul's response. Continue only independent read-only work while it is pending.
Record the answer and update this plan before resuming. Lack of response is not
approval. If a previously agreed choice proves insufficient, stop again rather
than broadening the design silently.

| Decision | Status / checkpoint |
| --- | --- |
| Core in every Connections management flow; bounded agent pull | Agreed 2026-09-09. Do not reopen without new evidence. |
| Reuse local handlers, apply revisions, preserve unmanaged settings | Agreed 2026-09-09. Implementation must preserve these boundaries. |
| Agent identity/auth bootstrap, key storage, rotation and revocation | Pending. Do not select a shared secret, signing protocol or enrollment system unilaterally. |
| Secret storage/delivery and OAuth refresh-token ownership | Pending. Do not infer a policy from the PDF or carry forward inbound management credentials. |
| Revision ordering, retries, acknowledgement persistence and local drift inside managed fields | Pending. Present crash/retry and user-edit examples, including what remains authoritative after each failure. |
| Existing-agent adoption and reimage/recovery | Pending. No automatic import, overwrite, purge or claim that all state can be recreated from Core. |
| Pairing, approval, reset, test-now and other one-shot actions | Pending before that extension. A command queue or destructive desired-state action is an architectural decision. |
| Additional daemon/service, generic framework, transport or new ownership boundary | Stop; outside this plan. |
| Additional agent rollout or production mutation | Stop; explicit authorization for the concrete candidate/cohort remains required. |

Routine implementation choices within an agreed contract—file organization,
naming, ordinary refactors and tests—do not need repeated permission. Endpoint
or schema choices that alter authority, effects, compatibility or recovery do.

## Existing decisions and superseded work

ADR 0003 describes the current chat-command implementation and explicitly says
there is no central desired configuration or continuous reconciliation. This
plan changes transport and introduces narrowly scoped Connections desired
state; it does not discard the ADR's local ownership, conflict protection,
rollback, process-boundary or recovery requirements. Amend the relevant ADR
sections when the detailed replacement contract is agreed.

Austin's September 4 “Connections Management — Today vs. Desired State” PDF
motivates the direction. It labels itself a strawman. Its claims about an
existing config/skills polling channel, schema-only new integrations, absence of
local state and restoring everything by reimage are not accepted facts or
implementation requirements without further agreement and proof.

PRs #860 and #862, and the uncommitted codex/fin-37-runner-ingress work, are the
superseded inbound-control approach. Do not base the pull implementation on
that stack or inherit its management routes, target files, credential registry,
signing protocol or ingress service. Reuse only independently justified local
operations/tests. Those local HTTPS test results do not qualify pull behavior.
PR #858 remains provisional hosted Hermes access; #845 remains useful UI work.
This document does not close PRs or mutate the paused worktree.

## Progress and decision log

- 2026-09-09: Paul agreed to bounded Core-owned Connections state, agent pull,
  revision-driven reuse of local operations, and preservation of unmanaged
  settings. Requested a written plan before implementation and consultation on
  major architectural decisions.
- 2026-09-09: Plan recorded before pull implementation. Next work is phase 1's
  through-line review and concrete contract proposal; pending decisions above
  have not been implemented or approved.
