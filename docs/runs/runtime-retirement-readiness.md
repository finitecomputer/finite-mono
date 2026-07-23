# Runtime Retirement Readiness

Status: ACCEPTED (production rollout and independent restore passed 2026-07-21)

Owner: Paul

Opened: 2026-07-18

Expires: 2026-08-15

Acceptance: A disposable, user-owned Kata Agent with meaningful durable state
is retired through the reviewed product flow. A versioned ZIP of its complete
durable `/data` Recovery Set is written to encrypted off-host storage and
verified by reading it back. The canonical container is removed, lat1 Runner
capacity and the owner's active-Agent entitlement each decrease by exactly one,
the Agent disappears from that owner's dashboard, and its compute and
runtime-scoped access are offboarded: the provider endpoint is gone, Core clears
published app URLs and Hermes availability, removes the relay credential, and
revokes the target Runtime or Project's active Finite Private keys. The off-host
ZIP then restores onto an empty, isolated target and preserves the Agent
Principal, Hermes state, workspace
contents, file modes, and symlinks. Failures at every boundary remain safe and
retryable. Paul performs the final dashboard and restored-Agent checks; a
successful upload, retained lat1 directory, or stopped container alone does not
claim acceptance.

Paul separately authorized the production rollout on 2026-07-21: merge the
reviewed implementation, provision a dedicated restricted Borg namespace,
deploy Runtime Retirement on finite-lat-1, retire AEON Canary 0714, and require
an independent empty-target restore before proceeding. That acceptance passed.
The authorization then covered Agent Camp Demo and Sites Canary 0715, plus a
separate data-preserving cleanup of stranded Waffle. It explicitly excluded
Upgrade Canary 0715 and Waffle Prime.

PR [#110](https://github.com/finitecomputer/finite-mono/pull/110) remains the
post-mortem context for fail-closed rollout and recoverability. The broader
ordering and deferred product work remain tracked in
[`triage-and-priorities-2026-07-17.md`](../triage-and-priorities-2026-07-17.md)
and the concise
[`platform-reliability-checklist-2026-07-21.md`](platform-reliability-checklist-2026-07-21.md).

## Live session checklist

- [x] Park Latitude Kubernetes; continue normal Latitude expansion.
- [x] Bring finite-lat-3 up as the default new-Agent Runner with a hard 32-Agent limit.
- [x] Land the declarative finite-lat-1/finite-lat-3 bridge and safe lat1 rollout.
- [x] Prove one real Agent empty-target restore and the bounded in-flight-chat recovery boundary.
- [x] Implement and locally fault-test Runtime Retirement; keep all production gates off.
- [x] Separately authorize and provision the restricted retirement Borg namespace and credentials.
- [x] Deploy Runtime Retirement and pass a disposable retirement/independent-restore canary.
- [ ] Add fail-closed Agent-creation capacity admission and the contact-Paul UI.
- [ ] Extend recovery proof to the complete finite-lat-1 Recovery Set, then schedule its NixOS/RAID reprovision for an evening window.
- [ ] Consider finite-lat-2 Agent capacity later; keep its CI Runner untouched for now.

## Production acceptance evidence — 2026-07-21

- PR [#139](https://github.com/finitecomputer/finite-mono/pull/139) merged the
  reviewed Runtime Retirement implementation. PR
  [#140](https://github.com/finitecomputer/finite-mono/pull/140) added the
  independent product gates and deployed the digest-pinned Core, Runner, and
  dashboard configuration on finite-lat-1.
- The existing rsync.net account now has a retirement-only encrypted Borg
  repository. finite-lat-1 holds only its restricted writer credential; the
  recovery credential, passphrase, and Borg key export are held off-host.
- AEON Canary 0714 retired only after remote Borg readback verification. Its
  archive was then retrieved with the separate recovery credential, restored
  onto an empty finite-lat-3 target, checked against every manifest entry, and
  passed identity, mode, symlink, and all four SQLite integrity checks. A
  network-isolated Kata boot returned `status=ready` with the restored chat
  state. The retained finite-lat-1 source was not used for that restore.
- Agent Camp Demo and Sites Canary 0715 then retired sequentially through the
  same product flow. Each has one immutable Borg receipt, an inactive Runtime
  link, retained local durable state, and no remaining compute.
- Waffle was handled afterward under Paul's separate stranded-cleanup
  authorization. Its one exact runtime-bound durable-state directory was
  copied before repair, booted without external networking, and proved to
  expose the same Agent Principal as the repaired canonical runtime. It then
  adopted the accepted Runtime artifact and retired through the normal Borg
  and Core flow. The temporary repair copies were removed only after the
  immutable receipt existed; the original retained state and Borg archive
  remain.
- PR [#153](https://github.com/finitecomputer/finite-mono/pull/153) added the
  generic operator-only unrecoverable-legacy archive boundary. Immediately
  before using it for Sol 2, Core still had the exact expected Project,
  Runtime, source host, source machine, and owner binding, with no provider
  metadata, active lifecycle request, or retirement receipt; every containerd
  namespace lacked its compute and its exact durable-state directory was
  absent. Paul acknowledged this disposable pre-launch test Agent was
  unrecoverable. The audited transaction archived its membership, deactivated
  its Runtime link, removed relay access, revoked its one scoped Finite Private
  key, and retained all Core history. It did not create or claim a backup.
- Upgrade Canary 0715 and Waffle Prime remained linked, receipt-free, and
  healthy. The three healthy-batch retirements reduced running Kata sandboxes
  from 26 to 23 exactly; the temporary Waffle repair returned the count to 23.
- Kata occasionally retained a stopped task after graceful shutdown. The
  retirement request stayed retryable and did not archive or remove compute
  early; deleting only the exact verified `STOPPED` task allowed the same
  request to complete. No broad container or task cleanup was used.

## Problem statement

Hosted Agents cannot currently be retired safely. The dashboard and Core
contain a disabled removal path, and successful Core offboarding already knows
how to archive dashboard membership, release the active runtime link, revoke
runtime credentials, and retain historical records. Runtime Retirement remains
unadvertised because the Kata Runner currently removes compute without first
creating and restore-proving a provider-independent Recovery Snapshot.

Operators therefore cannot free capacity or remove an obsolete Agent from its
owner's dashboard without bypassing the lifecycle contract. Direct container
removal is not an acceptable substitute: it can strand a Runtime while leaving
Core state, credentials, entitlement, and recovery evidence inconsistent.

This run closes only that gap. Runtime Retirement means:

1. preserve one verified, encrypted, off-host ZIP of the complete Agent Runtime
   Recovery Set;
2. remove the canonical compute so it stops consuming lat1 Runner capacity;
3. archive the owner's dashboard membership and deactivate the active runtime
   link so it stops consuming the owner's Agent entitlement; and
4. retain enough Core metadata and local durable state for support and audit.

Runtime Retirement is not Stop, subscription cancellation, account deletion,
or Purge User Data.

## Authority and boundaries

If Paul explicitly marks this run `ACTIVE`, it authorizes the repository code,
tests, documentation, and disposable local infrastructure required by the
queue below. Exactly one run may be active; activation while another run is
active requires the owner-directed sequence change recorded in both documents.

Activation does not by itself authorize:

- provisioning or changing an external backup repository or credential;
- reading production recovery credentials or user data;
- deploying Core, Runner, dashboard, or host configuration;
- retiring any production Agent, including a disposable canary;
- enabling the owner-facing action for production users; or
- deleting a Provider Durable Volume, local `/data`, or any Recovery Snapshot.

Each external or production mutation requires Paul's separate explicit
authorization after its preceding local and synthetic gates pass. Ambiguous
runtime identity, state roots, storage receipts, ownership, or provider state
fail closed without mutation.

## Proposed v1 design

### Reuse the existing lifecycle

- Keep the existing internal `destroy` wire kind, but present it as **Retire
  agent**. Do not add another lifecycle state machine.
- Keep Runtime Retirement disabled unless both the persisted Runtime and the
  current Kata Runner advertise the capability.
- Extend successful Destroy completion with one typed Recovery Snapshot
  receipt. Core must reject completion without a matching verified receipt.
- Persist one immutable snapshot record per retirement request: format version,
  backend, opaque locator, byte size, ZIP SHA-256, and creation and verification
  timestamps, plus the finite-assisted Recovery Authority identifier and
  `indefinite_until_purge` retention policy. The linked audited control request
  remains the actor and consent record. Store no key, credential, signed URL,
  email, Agent name, or other secret-bearing value.
- Complete the existing Core offboarding transition only after off-host
  verification and compute removal. That transition remains the authority for
  dashboard visibility, entitlement release, endpoint clearing, credential
  revocation, and historical retention.
- Keep snapshot and restore orchestration in the Core-to-Runner lifecycle. Do
  not add backup, restore, export, or purge commands to Runtime Management Pipe.

### Recovery Set and ZIP

- The Recovery Set is the exact durable `/data` root bound by the persisted
  RuntimeSpec. The Runner must validate the canonical container, source host,
  project/runtime labels, durable-state identifier, and bind mount before
  stopping or reading it.
- Gracefully stop compute and confirm writers are quiescent before creating the
  ZIP. The exact canonical container must be stopped, its containerd task must
  be absent, and two complete source manifests must match before the ZIP is
  finalized. A bounded stop escalation is permitted, but must be recorded and
  is never described as graceful; task absence and stable manifests remain the
  archive authority. Do not copy live SQLite database and WAL files while
  Hermes is writing.
- Create one root-only staging ZIP with a versioned manifest and a `data/`
  tree. The manifest records opaque project, runtime, request, and durable-state
  identifiers; Runtime artifact digest; expected Agent Principal when
  observable; creation time; file count and size; file hashes, modes, kinds,
  and symlink targets.
- Reject unsupported file kinds, unsafe paths, symlink escapes, corrupt hashes,
  missing files, duplicate manifest entries, and unsupported format versions.
- Host Runner environment files and secret-reference material outside `/data`
  are not snapshot inputs. User-owned credentials already stored inside
  `/data` are part of the Recovery Set, which is why root-only staging and
  encrypted off-host storage are mandatory.
- Delete the local staging ZIP after verified archival. Keep the original lat1
  durable-state directory in v1. It does not consume a Runner slot and provides
  a conservative second copy; deleting it belongs to a later, separately
  authorized Purge User Data run.

### Recovery promise and authority

- The snapshot recovery point is the quiesced `/data` state immediately before
  compute removal. This is an on-retirement snapshot, not periodic backup, and
  it makes no claim about an earlier recovery point.
- Retirement and restore promise a usable post-restore chat state, not every
  in-flight token or message. The isolated local fixture requires the preserved
  Agent Principal and canonical Room/MLS/Hermes state plus two new decryptable
  chat round trips; the interrupted response may be absent, partial, or
  complete. A production restore stays network-fenced unless separately
  authorized, and proves its persisted state without silently reactivating it.
- v1 promises that authorized Finite support can independently retrieve,
  verify, extract, and boot the retained Recovery Set in an isolated target.
  It does not promise a production reactivation time, automatic reconnection,
  self-service restore, or an **Undo retirement** action. The acceptance drill
  records observed retrieval and restore time without turning it into an SLA.
- The v1 Recovery Authority is explicitly Finite-assisted and operator-trusted:
  the restricted upload credential lives on lat1, while the administrative
  recovery credential, Borg passphrase, and Borg key export remain available
  off lat1. This does not claim a User Recovery Key or operator-blind storage.
- The owner's confirmed request and the existing audited Runtime control bind
  the actor, Project, Runtime, and snapshot receipt. A later support recovery
  requires separate authorization and produces its own audit evidence.
- The retention policy is indefinite until a separately approved export,
  retention, and Purge User Data design replaces it. No v1 job prunes or
  compacts retirement archives.

### Proposed storage decision

Use one configured backend in v1. Do not build a storage-provider abstraction,
dashboard selector, replication policy, or lifecycle engine.

| Destination | Advantages | Costs and limits | v1 decision |
| --- | --- | --- | --- |
| Dedicated rsync.net Borg repository | Existing lat1 Borg/Nix pattern; off-provider from Latitude; client-side encryption, compression, integrity checks, and append-only upload credentials are supported. | The exact ZIP is carried inside a Borg archive, so recovery requires Borg plus separately retained passphrase and key export. The current shared lat1 SSH credential is broader than destination-enforced append-only access and must not become the new retirement writer. | **Selected.** Use a retirement-only repository or restricted namespace, a destination-restricted append-only upload key on lat1, and a separate administrative recovery credential plus Borg key material held off lat1. One deterministic Borg archive contains exactly one ZIP. Do not prune or compact in v1. |
| Latitude Object Storage | Direct S3-compatible ZIP object; simple retrieval; versioning and Object Lock are available. | Preview service; same provider/account failure domain as lat1; no proven production credential or restore path in this repository; would add client-side encryption, key custody, and retention configuration. | Do not implement in v1. Reconsider only if a later run explicitly prefers direct objects over the already proven Borg pattern. |
| lat2 filesystem | Simple and fast SSH/rsync transfer; ample space; useful empty-target restore-drill host. | Same Latitude/provider account; mutable general-purpose CI/build host; no independent immutability or backup authority. | Use only as an isolated restore-drill target or temporary additional copy, never as the sole Recovery Snapshot authority. |

Use an opaque deterministic archive name derived from the retirement request,
not an email or Agent name. Retain archives indefinitely until a separately
approved retention and Purge User Data design exists.

Revalidate the external-service facts when this run is activated. The current
references are [rsync.net's Borg documentation](https://www.rsync.net/products/borg.html),
[Latitude Object Storage documentation](https://www.latitude.sh/docs/storage/object-storage),
the deployed [`backups.nix`](../../infra/nixos/modules/backups.nix), and the
[`lat2` host notes](../../infra/hosts/lat2/README.md).

### Ordered transition and retry rules

1. Core authenticates the owner, deduplicates an existing active request, and
   creates one leased Runtime Retirement control. The Agent remains visible and
   consumes both slots while the operation is incomplete.
2. The Kata Runner takes the per-runtime lock, validates canonical compute and
   the exact durable-state binding, gracefully stops the container, and keeps
   the control lease alive during archival.
3. The Runner creates and locally verifies the ZIP, uploads it under the
   deterministic retirement-request name, retrieves it from the remote
   repository, and verifies the exact ZIP SHA-256 and manifest. Repository
   listing or upload exit status alone is insufficient.
4. After remote verification, the Runner removes the canonical container and
   verifies its exact container ID and name and its containerd task are absent,
   and that the Runner's active-sandbox count decreased by exactly one. This
   releases one Runner capacity slot.
5. The Runner removes the plaintext staging tree while retaining the original
   durable state root. Cleanup is a required retryable phase: a failure returns
   the same request to the queue, and a retry re-verifies the remote archive
   from its persisted receipt before trying cleanup again.

6. Only after staging cleanup succeeds, the Runner completes the leased control
   with the typed snapshot receipt.
   One Core transaction stores the immutable receipt, marks the Runtime
   offline, archives only the target Project membership, deactivates only the
   target runtime link, clears its published app URLs and Hermes availability,
   and revokes only its runtime-scoped relay and Finite Private credentials. This
   releases one owner entitlement and makes the Agent disappear.

Retries use the same request, persisted phase record, and archive name. A
transient failure returns the leased request to the queue instead of marking it
terminally failed; lease renewal keeps long artifact and transfer phases bound
to the same worker. Before remote verification, any
failure leaves compute, visibility, entitlement, and credentials intact, though
the Agent may remain stopped and visibly retryable. After remote verification,
a retry may finish retirement. If compute is already absent, completion is
allowed only when the exact remote archive re-verifies against the leased
request and RuntimeSpec; absent compute without that evidence fails closed.

No path may create a second conflicting snapshot for one request, silently
select another durable directory, manually rewrite Core rows, or turn an absent
container into generic relaunch work.

### Unrecoverable legacy inventory

Do not special-case pre-launch Agent names or identifiers. When both canonical
compute and the expected durable-state directory are independently confirmed
absent, the owner may acknowledge that the legacy Runtime is unrecoverable. An
operator can then use `finite-saas-core runtime-archive-unrecoverable` with the
exact Project, Runtime, source host, source machine, and owner email plus all
three required acknowledgement flags.

The Core transaction fails closed if that binding changed, provider metadata
exists, a lifecycle control is active, or a retirement snapshot exists. On
success it reuses the normal offboarding boundary: archive dashboard
membership, deactivate the Runtime link, remove relay credentials, revoke
scoped Finite Private keys, and retain all Project, Runtime, link, and audit
history. This releases inventory; it does not claim or create a backup.

### Operator retirement for managed test deploys

An operator may retire a clearly identified Finite-managed test deployment with
`finite-saas-core runtime-retire-exact`. The command requires the reviewed
Project, Runtime, source host, and source machine identifiers, records the
operator identity in the existing admin audit log, and fails closed if the
active binding changed. It enqueues the normal `destroy` control and waits for
the same encrypted off-host Recovery Snapshot, verified readback, compute
removal, credential revocation, and Core offboarding used by owner retirement.
It is not an unrecoverable archive, owner impersonation, direct database edit,
or container deletion.

Run it only from the root-only Core environment on the deployed host:

```console
finite-saas-core runtime-retire-exact \
  --project-id <project> \
  --expected-agent-runtime-id <runtime> \
  --expected-source-host-id <host> \
  --expected-source-machine-id <machine> \
  --admin-email <operator-email> \
  --admin-workos-user-id <operator-workos-user-id>
```

The command prints the terminal control request. A failure or timeout retains
the same retryable request and recovery state; do not enqueue another target or
fall back to manual cleanup.

### Product surface

- Rename the existing disabled **Remove agent** action to **Retire agent**.
- Enforce ownership in Core; a hidden or disabled dashboard element is not an
  authorization boundary.
- Confirmation states that retirement stops the Agent, removes it from the
  dashboard, releases its active slot, preserves a support-held recovery
  snapshot, and provides no self-service restore or undo.
- While the existing request is active, keep the Agent visible with a
  **Retiring** state and disable duplicate submission. On failure, keep it
  visible with a retryable error. Disappearance means Core offboarding
  committed, not merely that the container stopped.
- Retirement does not delete Finite Chat history, accounts, published Finite
  Sites, Brain server records, billing records, or other product-owned state.

## Proposed queue

Work top-down only after this run is explicitly made `ACTIVE`. Every retained
item is required.

### P0 — Freeze and prove the recovery artifact

- Specify `finite.agent-runtime-recovery-zip.v1`, its safe relative layout,
  manifest fields, hash rules, mode/symlink behavior, staging-space preflight,
  and fail-closed verifier. Add fixtures for valid, corrupt, truncated,
  path-traversing, symlink-escaping, unsupported, and non-empty-target cases.
- Add a disposable local Kata/Apple Container fixture with an existing Agent
  Principal, Hermes and Finite Chat SQLite state, memory, workspace files,
  permissions, and symlinks. Stop it, create the ZIP, restore only that ZIP
  onto an empty isolated target, boot with outbound side effects fenced, and
  prove identity and content preservation.
- Prepare an exact external-storage activation request: repository/namespace,
  append-only upload-key restriction, pinned host key, off-lat1 administrative
  credential, Borg passphrase and key-export custody, archive naming, and
  no-prune policy. Provision or access it only after separate authorization.
- Upload a synthetic ZIP, retrieve it through the independent recovery path,
  verify its exact bytes, and repeat the isolated restore. Do not proceed if
  lat1 is the only holder of any required key or recovery material.

### P0 — Make Runtime Retirement crash-safe

- Add the immutable Core snapshot record and typed Destroy completion receipt.
  Prove wrong kind, request, runtime, project, durable-state identifier,
  backend, locator, size, hash, lease, and replay all fail closed; identical
  completion replay is idempotent.
- Implement the Kata stop, ZIP, upload, remote readback, verification, compute
  removal, staging cleanup, and completion sequence. Renew the existing lease
  rather than increasing one fixed timeout to hide large snapshots.
- Make every transition boundary restartable. Fault-inject before and after
  stop, ZIP finalization, upload, readback, receipt creation, container removal,
  staging cleanup, and Core completion. An absent container is acceptable only
  with the exact remotely verified retirement artifact.
- Prove Core completion archives only the intended membership, deactivates only
  the intended runtime link, revokes only target-scoped credentials, preserves
  Runtime/Project/audit rows and local `/data`, and cannot be invoked by Stripe,
  billing lapse, Stop, account deletion, or an unsupported Runner.
- Keep Phala and every other adapter false. Enable the capability only for a
  configured Kata Runner after all local and synthetic acceptance gates pass.

### P1 — Expose and exercise the bounded product flow

- Replace the disabled maintenance copy with the owner-authorized **Retire
  agent** confirmation and Retiring/failed states. Add dashboard route,
  authorization, dedupe, visibility, and error tests. Add no retired-Agent
  list, restore button, storage choice, retention control, bulk action, or
  billing behavior.
- Run the complete local product flow against a stateful existing synthetic
  Agent, not only a newly created empty runtime. Prove normal restart, upgrade,
  chat, Sites, and owner dashboards remain unchanged for non-retired Agents.
- Prove how an existing healthy Kata Runtime acquires the newly accepted
  persisted capability through a supported Runtime Upgrade or other existing
  Runner/Core reconciliation path. Do not SQL-backfill capabilities. If an
  Agent image rollout is required, name it explicitly in the later deployment
  request; missing-compute Runtimes remain excluded.
- With separate deploy and production-retirement authorization, enable one
  disposable Kata canary only. Record exact revisions and baseline Runner and
  entitlement counts, retire it, verify the encrypted off-host ZIP through
  remote readback, verify both counts decrease by one, and verify the Agent
  disappears only after Core commits.
- Retrieve the canary ZIP without using the retained lat1 state, restore it to
  an empty isolated target, and prove the retained Agent Principal and durable
  contents. Keep public ingress, Finite Chat sends, Telegram/Discord, email,
  webhooks, Sites publishing, billing, and other outbound side effects fenced.
- Extract the exercised operational procedure into an indexed runbook with
  `PRECONDITIONS`, `STEPS`, `VERIFY`, and `ROLLBACK`. Update the governing
  recovery/retirement TODOs only to the level actually proved.
- Produce the exact Acceptance Request below. Paul performs it last. Do not
  enable the owner-facing action broadly before acceptance.

## Evaluation, rollback, and stop conditions

### Repository implementation evidence (2026-07-21)

- The versioned ZIP/verifier has positive and negative fixtures for exact
  content, modes, symlinks, unsafe paths, corruption, truncation, identity
  mismatch, and non-empty restore targets. A SQLite WAL fixture restores and
  accepts a new write.
- Core has an immutable receipt migration, exact receipt validation,
  same-request lease renewal/retry, idempotent completion, independent
  default-off product gate, and an exercised real-Postgres transaction test.
- The configured Kata path requires task absence and two stable manifests,
  verifies Borg readback before removal, persists restartable phases, checks
  removal convergence, and retains original `/data`. Synthetic tests cover
  readback failure before removal and restart after removal before Core commit.
- The owner UI remains independently default-off, reports Retiring/retrying
  without exposing Runner failure details, and disappears only after Core
  archives the target membership.
- The repository-wide services-only `just dev smoke` gate passes with the new
  migration and default-off behavior.
- The production Borg provisioning, deployment, disposable canary, and
  independent empty-target restore are recorded above. Repository tests remain
  synthetic evidence and are not substituted for that acceptance result.

Required repository gates are Core and Runner unit/integration tests, dashboard
lint/test/build, runtime fixture tests, migration rollback where supported, and
`just dev smoke`. The snapshot verifier and retirement state machine also need
deterministic fault-injection tests for every irreversible boundary.

The deployment rollback is to keep or restore Runtime Retirement capability to
false and hide the action. Already completed retirements remain retired; a
service rollback is not an Agent restore. Because v1 retains local `/data` and
the verified off-host ZIP, no rollback deletes either copy.

Stop and escalate without mutation on:

- an ambiguous or missing canonical container, RuntimeSpec, durable state root,
  owner, Agent Principal, or source-machine binding;
- any ZIP creation, integrity, upload, retrieval, decryption, or restore error;
- a lease loss that cannot be resolved by idempotent replay;
- a capacity or entitlement count changing by anything other than the one
  target Agent;
- credential revocation or membership archival affecting another Project;
- an off-host key whose only usable copy is on lat1;
- pressure to delete retained local state or snapshots as part of retirement;
  or
- any need for manual container reconstruction, generic missing-compute
  relaunch, direct database edits, or provider-specific exception logic.

## Acceptance Request (complete when ACTIVE)

- **Revision:** exact deployed mono revision, Core and dashboard image digests,
  Kata Runner/NixOS closure, Runtime artifact, migration revision, snapshot
  format version, and Borg repository identifier.
- **Where:** `https://finite.computer` with a dedicated disposable Agent owned
  by the acceptance account; finite-lat-1 read-only operational evidence; and
  an empty isolated lat2 or local Apple Container restore target. Secrets and
  live identifiers remain in encrypted evidence, not this public repository.
- **Time:** 30 minutes after the archive and isolated target are prepared.
- **Steps and observations:** record the canary's Agent Principal, workspace
  checksums, dashboard presence, owner active-Agent count, and Runner capacity;
  request retirement and observe Retiring; observe disappearance only after
  completion; observe both counts decrease exactly one; independently retrieve
  and verify the ZIP; restore it onto the empty target; boot fenced compute and
  observe the same Agent Principal, Hermes state, and workspace checksums;
  confirm unrelated Agents still load and chat normally.
- **Pass:** one intentional retirement releases exactly the two intended slots,
  hides only the intended Agent, removes only its provider endpoint, offboards
  only its recorded runtime access, and the independently retrieved ZIP
  restores its declared Recovery Set without using lat1's retained source
  directory.
- **Fail/stop:** preserve the request id, exact revisions, count-only evidence,
  service logs, and archive verification result; disable Runtime Retirement and
  the UI action; do not delete state, edit Core rows, retry against another
  directory, reconstruct compute manually, or broaden the run.

## Out of scope

- Periodic Agent backups, recovery-point objectives, multi-destination
  replication, or a generic backup platform.
- Self-service restore, a retired-Agents page, undelete, generic relaunch, or
  recovery of a Runtime whose canonical state binding is already ambiguous.
- Automatic retention, pruning, compaction, export, local `/data` deletion, or
  Purge User Data.
- Billing cancellation, refunds, subscription lifecycle, account deletion, or
  automatic retirement for inactivity or non-payment.
- Bulk retirement, fleet cleanup, dormant-Agent scheduling, or capacity
  autoscaling.
- Finite Chat history deletion, Sites unpublishing, Brain cleanup, or changes to
  product-owned data outside the Runtime Recovery Set.
- Phala or another Runner adapter.
- Generic missing-compute relaunch or retirement remains out of scope. The one
  owner-authorized Waffle cleanup is recorded above; Sol 2 remains excluded
  because its durable-state binding cannot be proved from finite-lat-1.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`docs/runs/README.md`](README.md)
- [`docs/runs/production-baseline-2026-07-15.md`](production-baseline-2026-07-15.md)
- [`finitecomputer-v2/CONTEXT.md`](../../finitecomputer-v2/CONTEXT.md)
- [`finitecomputer-v2/docs/runtime-control-contract.md`](../../finitecomputer-v2/docs/runtime-control-contract.md)
- [`finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md`](../../finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md)
- [`finitecomputer-v2/docs/hermes-runtime-test-matrix.md`](../../finitecomputer-v2/docs/hermes-runtime-test-matrix.md)
- [`infra/runbooks/hosted-web-chat-recovery.md`](../../infra/runbooks/hosted-web-chat-recovery.md)
