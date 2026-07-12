# Phala confidential Runner — dark worker and API-only operations

This runbook covers the separately fenced Phala worker defined by
`infra/nixos/modules/finite-saas-phala-runner.nix`. The checked-in unit is
**dark**: it has no `wantedBy`, no timer, is hard-drained, and has ordinary
capacity one. Merging or deploying its definition does not authorize starting
it, creating a Phala resource, installing credentials, enabling Confidential
placement, or admitting a customer.

Phala and Kata may initially share finite-lat-1. That is a common host failure
domain, not high availability. They remain different worker identities,
credentials, classes, state directories, privileges, and provider adapters.

This is an HTTPS API runbook. Never install or invoke the Phala CLI. There is
no provider-delete procedure here: shutdown, recovery, rollback, billing, and
drain preserve the CVM and its data. Runtime Retirement remains unavailable
until the accepted external Recovery Snapshot and reviewed retirement
transition exist.

## Fixed boundary

| Fact | Required value |
|---|---|
| Worker unit | `finite-saas-runner-phala.service` |
| Worker id / Core credential record | `finite-phala-runner-1` |
| Source host id | `finite-lat-1-phala-control-1` |
| Runner class | `phala` |
| State directory | `/var/lib/finite-saas-runner-phala` |
| Secret file | `/etc/finite/phala-runner.env`, `root:root`, mode `0600` |
| Core credential process variable | `FC_CORE_RUNNER_API_TOKEN` |
| Phala key variable | `FC_RUNNER_PHALA_API_KEY` |
| Core endpoint | `http://127.0.0.1:4200` |
| Phala API origin | `https://cloud-api.phala.com/api/v1` |
| Required headers | `X-API-Key` from the host-only file; `X-Phala-Version: 2026-06-23` |
| Resource class | `tdx.medium`, exactly 2 vCPU and 4096 MB |
| Disk | exactly 40 GB after provision and update |
| Public logs / sysinfo | explicitly false |
| Ordinary billable-resource cap | one, including API-visible non-deleted resources and Core in-flight reservations |

The standard environment name `FC_CORE_RUNNER_API_TOKEN` is process-local.
Its value must come from a distinct Core keyring record named
`finite-phala-runner-1`, bound to the worker id, `phala` class, and source-host
id above. It must never equal the Kata worker token. The Phala key is an
operator/provider credential; it is never included in RuntimeSpec, encrypted
Runtime environment, logs, provider handles, snapshots, or user data.

The unit has no shell/provider tools in `PATH`, no capabilities or devices,
and makes containerd, Docker, Podman, Kata/CNI paths inaccessible. Network
configuration supplies only loopback Core, while the typed Rust adapter pins
the official Phala HTTPS origin and API version without an override. Phala's
CDN addresses are not stable enough for a checked-in IP allowlist; if a future
security review requires kernel-enforced destination filtering, put a
name-validating egress proxy in front of the worker before enabling it. Do not
replace the pinned origin with a general outbound proxy or configurable URL.

## Preconditions — all required before a live start

1. The Phala readiness run is ACTIVE and the Stripe prerequisite named there
   is closed. Starting a live worker or spending money has separate explicit
   authorization.
2. The shipped `finite-saas-runner` uses the reviewed typed HTTPS adapter for
   every Phala operation. Source/static checks show no Phala subprocess,
   `FC_RUNNER_PHALA_BIN`, provider CLI package, or delete capability.
3. Core has additive placement/handle readers, a credential keyring entry
   bound to the fixed worker identity, class and source host, and creation is
   disabled. The previous Core/Runner generation can still read/control the
   new opaque handle envelope.
4. The exact canonical Runtime digest passed local Docker and the immediately
   preceding Kata rung. The same Core artifact id is in
   `/etc/finite/phala-runner.env`; no mutable image reference is accepted.
5. The reviewed client-side environment-encryption helper verifies the signed
   KMS/application key and binding. Plaintext environment never enters a
   command argument, provider log, temporary world-readable file, or retained
   operation record.
6. The complete Agent Runtime Recovery Set, external encrypted snapshot,
   recovery authority, repository-wide restore gate, and nonce-bound
   attestation verifier are ready for the authorized test stage.
7. API inventory, Core reservations/handles, and CVM volume facts reconcile to
   zero unknowns. Ordinary current/in-flight billable count is below one.
8. The secret file was installed out of band from
   `infra/hosts/lat1/systemd/phala-runner.env.example`, is `root:root 0600`, and
   contains non-empty distinct credentials. Never print or `source` it.

If any precondition is false, leave the unit dark and stop.

## Dark deployment verification

A separately authorized Nix deployment may install the definition while it
remains dark. These read-only checks must all pass:

```sh
systemctl is-enabled finite-saas-runner-phala.service  # expected: disabled
systemctl is-active finite-saas-runner-phala.service   # expected: inactive
systemctl cat finite-saas-runner-phala.service
systemd-analyze security finite-saas-runner-phala.service
```

Inspect only non-secret unit facts. Confirm there is no timer, `WantedBy`,
containerd requirement, socket access, provider CLI path, or privilege. Do not
use `systemctl enable` as an activation shortcut. A future canary generation
must deliberately change the checked-in drain/start policy after all gates and
authorization; rollback changes new admission back off while retaining
lifecycle control for known CVMs.

## API request discipline

All provider calls go through the reviewed typed adapter. It reads the key
from the systemd environment, sends `X-API-Key` only as an HTTPS header, pins
`X-Phala-Version`, bounds response size/pagination/timeouts/retries, and
redacts credentials and environment values. Do not reproduce calls with
`curl`, a browser, an SDK REPL, the Phala CLI, or ad-hoc scripts. Those paths
bypass schema, redaction, operation fencing, and persisted-handle rules.

The tables below name HTTP methods/paths for review and incident correlation;
they are not copy-paste commands. Never include request/response bodies,
headers, signed encryption material, quotes, or user data in a ticket or
terminal transcript. Retain only safe Core ids, opaque handle version,
operation correlation, redacted state, timestamps, shape/digest facts, and
counts.

## Preflight

The dispatch-only `Phala read-only preflight` workflow runs the typed
`finite-saas-runner phala-preflight` command behind the `phala-staging`
GitHub Environment. It performs authenticated reads only and retains a
redacted shape/price/capacity/count summary. Configure the environment-scoped
secret named `PHALA_CLOUD_API_KEY`; never put its value in a workflow input or
repository variable. A green read-only preflight is not the live Phala rung
and does not authorize a provision.

Before the worker may advertise a new lease, its startup preflight performs:

| Check | HTTPS operation | Pass condition |
|---|---|---|
| Version/auth/account access | authenticated read using pinned headers | accepted API version and expected Finite workspace identity |
| Instance catalog | `GET /instance-types/cpu` | `tdx.medium` is exactly 2 vCPU, 4096 MB, and the reviewed live price |
| Provider capacity | `GET /teepods/available` | response schema is recognized; region/quota are still provision-time gates |
| Inventory | every page of `GET /cvms/paginated` | bounded complete read; every Finite resource reconciles |
| Private operation | provision/update request contract | `public_logs=false`, expected Cloud KMS, exact 40 GB; no mutation during preflight |

A preflight failure blocks new creation leases. It must not erase handles or
disable lifecycle controls for already known CVMs. Stop and escalate on API
version/schema drift, changed Medium shape/price, incomplete inventory, an
unknown resource, or inability to keep logs private.

## Provision and adopt

Provisioning is always a Core-led fenced operation; an operator never creates
a CVM directly.

1. Core persists the immutable RuntimeSpec, operation id, Finite correlation
   name, desired artifact, placement, resource class, and in-flight reservation
   before the first provider mutation.
2. Reconcile the full API inventory and cap. One active/non-deleted Finite CVM
   **or** one Core in-flight reservation consumes ordinary capacity.
3. `POST /cvms/provision` with exact Medium/40 GB/Cloud KMS/private-log facts.
   Persist `app_id`, compose hash, signed encryption-key input,
   `provisioned_at`, and 14-day expiry before proceeding.
4. Verify the signed key and app/KMS binding; locally encrypt the complete
   environment. Core must acknowledge the persisted provisional handle before
   commit.
5. `POST /cvms` commits only an unexpired provision and verified encrypted
   environment. Persist the returned versioned CVM handle before readiness
   polling.
6. `GET /cvms/{cvm_id}` by handle until the bounded provider state is stable;
   then verify exact image/config/environment-key facts and generic Runtime
   health. Provider name search is inventory assistance, never identity.
7. Reconcile provider inventory, Core operation/reservation ledger, handle,
   revision, and attached-volume facts again.

Adopt uses a persisted CVM id and `GET /cvms/{cvm_id}`. It never guesses from
an endpoint or lowercases a provider id into a source-machine id. Exactly one
matching resource may be adopted; zero or multiple matches stop the operation.

## Status and restart

Status joins Core's persisted Runtime/operation state with
`GET /cvms/{cvm_id}` and the generic Runtime health/release facts. A provider
state transition means an operation started, not that the agent is ready.

Normal restart begins as a typed Core lifecycle request and a class/source-host
fenced lease. The adapter sends `POST /cvms/{cvm_id}/restart` with non-force
semantics, polls the same handle, then waits for generic readiness. It must
preserve the named volume, complete environment, Agent Principal, endpoint
contract, workspace and chat state. Do not restart by provider name or use
force as a timeout fallback.

## Recover known-good Runtime

Recovery begins only through Core's typed recover-known-good operation. Carry
the versioned one-shot boot intent in the complete desired RuntimeSpec and use
the shared canonical image behavior. Phala environment changes replace the
full set, so inspect/merge the complete existing environment and prove no key
is lost. Clear the one-shot intent only after health reports the accepted
outcome.

Recovery never creates a new Agent Principal, repairs arbitrary user files,
changes room membership, drops connections/skills/workspace, or silently
falls back to another artifact. If the narrow image-owned recovery fails, stop
for a reviewed restore/migration path.

## Runtime Upgrade and rollback

Upgrade is a fenced Core operation with one `/data` writer:

1. Persist desired digest/compose and operation id; inspect current handle,
   revision, complete environment, volume and writer state.
2. `POST /cvms/{cvm_id}/compose_file/provision` for the exact digest and 40 GB
   contract. Verify and persist the returned update inputs before encryption.
3. Stop/detach the old writer as required by the staged provider contract.
   Ambiguity stops here; never allow two revisions to write `/data`.
4. `PATCH /cvms/{cvm_id}/compose_file` with the verified encrypted **complete**
   environment. Poll by CVM id, verify resulting digest/config/key set, generic
   health and attestation, then commit Core facts.
5. Reconcile revisions, one-writer state, handle, volume and inventory.

Rollback is the same state machine with the previous accepted digest/compose
and current complete environment. Do not assume provider revision redeploy
preserves environment until staging proves it. Rollback never changes Hosting
Tier, placement, Agent Principal, volume, recovery authority, or user data.

## Graceful stop

A typed Core stop request leases to this worker and sends
`POST /cvms/{cvm_id}/shutdown`. Poll until the positively identified handle is
stopped, reconcile the volume and inventory, then let Core record `offline`.
Stopping the systemd worker does **not** stop a CVM. Billing lapse, drain, and
rollback never imply provider shutdown unless the specific typed stop request
was authorized.

## Attestation

Management/API state alone is not fresh attestation. The canonical image must
request a quote through its Phala-only `/var/run/dstack.sock` mount using
verifier-chosen nonce/report data. Verify outside the CVM: nonce freshness,
Intel TDX chain and fresh collateral, acceptable TCB, RTMR replay, exact
compose hash and image digests, and accepted OS/KMS measurements. Bind the
redacted evidence record to Runtime id, opaque handle version, Product Release
and timestamp. Never retain raw secrets, environment, messages, user files, or
unreviewed quote payloads in this repository.

Stop if the nonce is replayed, collateral is stale, measurements/digests do
not match, or the evidence supports only a weaker privacy claim than the
declared Cloud KMS/O1 posture.

## Empty-target restore

Restore requires a proven external encrypted Recovery Snapshot; a Phala volume
is not a backup.

1. Obtain explicit restore authority. Verify snapshot integrity/version,
   Recovery Authority envelopes, owner, retention and target placement without
   the source Phala/KMS path.
2. Fence and gracefully shut down the source. Confirm it cannot write `/data`
   and Core has no other ready Runtime for the Project.
3. Open the one allowed stopped-source retention exception with named owner and
   expiry. While open, block every unrelated Phala provision.
4. Provision one empty replacement through the normal persisted Core flow,
   restore the complete declared Recovery Set, and boot the unchanged accepted
   image in restore mode.
5. Verify identity, chat membership, Hermes memory, workspace checksum,
   connections, installed baseline/user overrides, future restart, generic
   health, digest and fresh attestation.
6. Atomically switch Core's canonical handle/contact only after those checks.
   Keep the old source stopped and retained under the explicit handoff.

At retention expiry, stop and ask Paul to authorize a reviewed recovery-safe
Runtime Retirement or extend the exception. This runbook intentionally has no
provider deletion step.

## Inventory and cost

Read every bounded page of `GET /cvms/paginated` and join API-visible Finite
apps/CVMs/revisions against Core provision/reservation operations, current and
historical handles, and CVM-attached volume facts. Record safe counts for
creating, running, stopped, retained and unknown resources. Expected unknown
count is zero. Never automatically mutate an unknown resource.

Ordinary capacity is consumed by any current billable/non-deleted Finite
resource or in-flight reservation, regardless of running state. The one
stopped-source restore exception is separately owned/expiring and blocks new
provisioning; it does not raise ordinary capacity.

Cost inspection records the verified live catalog rate, actual provider
account charges from a documented API/export when available, storage,
bandwidth, Finite Private inference, backup and support observations, plus
elapsed operation and retention time. The current CVM API is not a billing
ledger; do not infer actual charges solely from status or multiply a nominal
rate and call it measured cost. If actual charges cannot be obtained safely,
record that gap and do not raise capacity. Only Paul may raise the ordinary cap
after reviewing measured cost against the $300 tier.

## Stuck and ambiguous operations

| Last durable boundary | Required action |
|---|---|
| Before a provision response was persisted | Keep the Core operation blocked. Because provision cannot be enumerated, wait through recorded 14-day expiry, then prove no matching app/CVM/revision before a new fenced operation. |
| Provision identifiers persisted, not committed | Inspect expiry and correlation. Resume the same operation only with verified persisted inputs; an expired provision stays blocked and reconciles. |
| Create response lost before CVM handle persisted | Inventory by the pre-persisted correlation. Adopt exactly one match; zero or multiple matches stop and escalate. Never blind-reprovision. |
| CVM handle persisted, readiness unknown | `GET /cvms/{cvm_id}` and resume bounded reconciliation by id. Do not search by endpoint/name as authority. |
| Update/rollback writer transition ambiguous | Fence both transitions, prove exactly one `/data` writer, inspect revisions/environment/volume, then resume or leave stopped. |
| Worker crash/outage | Keep new admission off. Restart the unprivileged worker only after durable Core/provider inventory is reconciled; known lifecycle operations retain their handles. |
| Unknown app, CVM, revision or volume | Make no automatic mutation. Record safe identifiers/counts, stop new admission and escalate. |

Never resolve uncertainty by starting another provision, using force restart,
changing placement, abandoning a handle, or deleting a resource.

## Worker service operations

Only after every precondition and explicit live-start authorization, a dark
preflight can be run with the checked-in drain still true:

```sh
systemctl start finite-saas-runner-phala.service
systemctl status finite-saas-runner-phala.service
journalctl -u finite-saas-runner-phala.service --since today
```

Logs must contain only redacted operation/state facts. Stop immediately if a
secret, encrypted environment, user content, raw quote, or full provider
response appears. `systemctl stop finite-saas-runner-phala.service` stops only
the control worker; it does not change a CVM. Keep the unit disabled until a
reviewed Nix generation deliberately supplies the accepted start policy.

## Stop and escalate

Keep new Confidential admission off and stop for Paul/security/recovery review
if any of these is true:

- credentials are missing, shared with Kata, unbound, exposed, or accepted for
  a different workspace/worker/class/source host;
- the binary contains a Phala CLI/subprocess or delete path, configurable API
  origin, plaintext environment transport, public logs, or host-runtime access;
- API version/schema, `tdx.medium` shape/price, 40 GB units, region or quota
  differ from the accepted staging facts;
- a signed encryption key/binding cannot be verified, provision ambiguity
  cannot remain blocked, or exactly one CVM cannot be adopted;
- API/Core/handle/revision/volume inventory has any unexplained mismatch, or
  billable/in-flight/retained count, elapsed time or cost exceeds its boundary;
- restart, recovery, upgrade, rollback or restore changes identity, requires
  re-pairing, loses an environment key or user data, or permits two writers;
- private diagnostics are insufficient, attestation is stale/mismatched, or
  recovery cannot restore onto an empty target with the source unavailable;
- resolution would require deletion, provider-console mutation, migration,
  customer admission, or any authority not explicitly granted.

## Rollback

Rollback of the worker means: keep Confidential Checkout/Launch Code issuance
and new Phala placement disabled, return the checked-in worker to drained/dark,
and keep the compatible handle reader plus lifecycle control for every known
CVM. Do not convert Confidential placement to Kata, silently change an image,
stop a healthy CVM, revoke recovery keys, retire compute, or delete a volume.
Ambiguous resources remain fenced in reconcile/adopt or explicit
stopped-retained state until a human resolves them.
