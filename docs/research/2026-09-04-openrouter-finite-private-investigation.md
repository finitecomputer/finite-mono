# OpenRouter to Finite Private: investigation notes

Recorded September 4, 2026 from the read-only fleet investigation that day.
This is a snapshot for discussion, not an approved migration plan or executable
runbook. The draft PR can remain unmerged as a record of these findings. It
does not authorize production changes, a deployment, or a migration tonight.

## Findings

The investigation found 41 running bots configured to use OpenRouter for their
main chat model:

| Host | OpenRouter main-chat bots |
| --- | ---: |
| box1 | 26 |
| TRF | 12 |
| smoke | 3 |
| lat3 / lat4 | 0 |
| Total | 41 |

All 53 active hosted agents on lat3/lat4 had Finite Private saved as their main
route. Another 10 active bots explicitly referenced OpenRouter for auxiliary
tasks or fallback despite having a different main provider. The reported scope
is therefore **at least 51 bots needing attention** for a complete OpenRouter
exit. Saved configuration is not proof of the provider used by every request;
automatic selection and session overrides remain unverified.

Of the 41 main-chat bots, 23 had credentials matching active Finite Private
keys. The remaining 18 lacked a Finite Private key in the checked environment
files, including mounted shared environments: 3 on box1, 12 on TRF, and 3 on
smoke. This does not establish that those accounts lack an entitlement or that
no credential exists elsewhere.

Existing saved provider settings survive reconciliation. Changing a shared
default alone would not switch all existing bots. A main-chat switch would
also change Claude/GPT conversations to GLM-5.3-Flash on Finite Private; it is
a model change as well as an endpoint change.

## Unresolved checks

- **Routing completeness:** inventory main, auxiliary, delegation, fallback,
  automatic provider selection, session overrides, and launch/recreation
  defaults. Several bots explicitly use OpenRouter for vision; a compatible
  private replacement has not been demonstrated.
- **Capacity:** Finite Private was healthy at the time of inspection, but the
  recorded load tests did not establish the planned 120-user capacity gate.
  Bot count alone does not establish simultaneous demand or available headroom.
- **Accounting:** the investigation reported 311 usage reservations older than
  15 minutes awaiting settlement. Their cause and impact remain unexplained;
  this observation is not a diagnosis or authority to alter accounting rows.
- **Real conversations:** gateway state and successful inference calls do not
  prove replies in users' actual chats. Text replies, tool calls, image handling,
  and continuing existing conversations need end-to-end verification.

## What a separately authorized migration would need

1. Refresh the inventory and resolve the unknown routes and credential gaps.
   Use `scripts/finite-status` for platform/fleet status; add any missing probe
   there instead of retaining the temporary investigation script as an operator
   command.
2. Trace each configuration writer and reader through launch, reconciliation,
   session selection, and gateway execution. Prove the affected existing-state
   and mixed-version behavior on synthetic state before production changes.
3. Name and verify the backup and rollback boundary for each batch, preserving
   durable history, identity, and recovery data. Keep stopped legacy copies
   stopped so duplicate channel consumers do not compete for messages.
4. Establish capacity and settlement evidence, then switch small batches with
   gateway restarts where required and explicit stop/rollback criteria. Run
   `scripts/finite-status` before and after each rollout and verify actual chat
   behavior before expanding.

The initial estimate was several hours for a staged main-chat switch, with
same-night completion conditional on these checks passing. A complete exit
from OpenRouter is broader and remains unproven. No schedule is committed.

## Evidence and limitations

The investigation read saved Hermes configuration and environment files on
box1, TRF, smoke, lat3, and lat4, checked active runtime/key records, and reviewed
fleet status and endpoint health. Local temporary evidence was retained as
`/tmp/finite-provider-inventory-{0..4}.jsonl`,
`/tmp/finite-provider-core.jsonl`, and
`/tmp/finite-private-fleet-status.json`. These files are ephemeral and are not
part of this PR. Runtime directories include inactive state; the 53-agent
figure used active runtime records, not a count of directories.

The notes preserve the earlier investigation; production was not queried again
when preparing this PR. Counts can change and are not a current rollout gate.
No credentials, user identifiers, raw environment files, production edits,
credential issuance, or service restarts are included.

Related repository context:

- [Runner Finite Private route](../../infra/runbooks/runner-finite-private-route.md)
- [Finite Private routing migration](../../infra/runbooks/finite-private-routing-migration.md)
- [GLM-5.3-Flash cutover gates](../../infra/runbooks/finite-private-glm-5.3-flash-production-cutover.md)
- [Historical degraded-admission and load-test evidence](../runs/glm-5-3-flash-degraded-admission.md)

The historical degraded-admission record is context, not evidence that its
temporary mode was still active on September 4.
