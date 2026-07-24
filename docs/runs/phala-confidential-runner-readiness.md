# Phala Confidential Runner Readiness

Status: ACTIVE (2026-07-23 — Paul explicitly authorized one paid
`tdx.medium` canary for `paul@finite.vip`, the activation path, and bounded
create/start/restart/stop authority under the hard cap of one)

Sequence note: On 2026-07-23, Paul moved work here from
[`Stripe Production Activation`](stripe-production-activation.md). That run is
PAUSED with its queue preserved. This activation is limited to the internal
canary and does not authorize a Stripe Price, another paid resource, or
customer admission.

Owner: Paul

Opened: 2026-07-11

Expires: 2026-09-05

Acceptance: After the unchanged promoted Runtime image passes local Docker and
Kata, `paul@finite.vip` redeems one Confidential Launch Code through the
normal product UI and Core launches exactly one live Phala `tdx.medium` CVM,
verified as 2 vCPU and 4096 MB. No Confidential Stripe Product or Price is
activated by this run. A forced Runner crash at each create boundary never
produces a second CVM. Paul completes multiple real chat turns from the Hosted
Web Device and a second independent Device after invite/status reports the
expected Agent Principal, saves and recalls memory, publishes a Site, and
makes a real Finite Private request with the Core-provisioned runtime key. Paul
then exercises restart, recover-known-good, Runtime Upgrade, and rollback
without re-pairing or changing the Agent Principal, workspace, or installed
skills. A provider-independent Recovery Snapshot of the declared Agent Runtime
Recovery Set restores onto an empty replacement Phala CVM, and fresh
nonce-bound attestation binds that CVM to the accepted compose and image
digest. The repository-wide paid-cohort restore gate for Core, Finite Chat and
the Hosted Web Device, Sites, Brain, and identity/recovery authorities is also
green, as are the inherited untrusted owner-claim and stuck-launch escape
gates. Public Phala logs stay off; API inventory contains no unknown Finite
apps, CVMs, or revisions; Core's provision/reservation ledger is reconciled;
and every retained volume is attached to a known CVM handle. Measured cost and
recovery objectives are published in retained operational/release records.
Paul performs the final browser and recovered-data checks; mocks, a
provider-volume-only restart, or a booting health endpoint do not claim
acceptance.

## Problem statement

Kata has proved the production Runner shape. The next bounded outcome is a
Confidential Hosting Tier that preserves that shape while adding Phala as a
second Runner. Standard remains $200/month on Kata. Confidential is
$300/month and initially maps in Core to one Phala `tdx.medium` runtime with 2
vCPU and 4 GB RAM.

The repository already contains a Phala experiment, but it is not a safe base
for users. It shells through a CLI whose response drift required three
production fixes, discovers provider identity after creation instead of
persisting it, can leak paid CVMs across ambiguous failures, does not enforce
its configured capacity, cannot perform Runtime Upgrade, and maps provider
delete to a data-destructive action. The public creation API and dashboard
also carry an internal Runner class even though the accepted contract says
Core owns placement.

This run replaces that experimental path with the Phala HTTPS API and closes
only the generic Runner gaps that a reliable second backend exposes. It does
not create a new agent model, image, dashboard control plane, provider
framework, or TEE product architecture.

## Authority and boundaries

ACTIVE status plus Paul's 2026-07-23 instruction authorizes repository work,
production deployment, credential installation, and one paid internal
`tdx.medium` launch-code canary owned by `paul@finite.vip`. The worker may
create/start/restart/stop that one resource through Core under the hard cap of
one. It does not authorize a second paid resource, provider-console mutation,
provider delete, Stripe Product or Price activation, customer admission,
recovery-key custody changes, or a higher cap.

Do not activate this run while [Stripe Checkout
Readiness](stripe-checkout-readiness.md) remains ACTIVE; its accepted closure
is a prerequisite, and the billing queue below owns only the new tier/Price
delta. This run's acceptance is necessary but does not by itself authorize a
paid cohort: the repository-wide recovery, untrusted owner-claim, and
stuck-launch escape/reaper gates must all pass independently. An internal
canary cannot stand in for any of them.

Issue #227 tracks the standing non-disruptive, at-most-15-minute hosted
Recovery Set cadence gate. Until it lands, the v2 empty-target restore proof
does not silently satisfy that separate gate. Paul must record a narrow
one-Agent internal-canary exception before launch; the existing ACTIVE
provider-spend authorization does not generalize that exception to customer
capacity.

Issues #228 and #229 track the distinct Agent Runtime external Recovery Set
and nonce-bound TDX verifier already required below. Their code paths and
offline fixtures are prelaunch gates; the one authorized live canary supplies
the provider replacement and fresh measurement evidence needed to complete
their production acceptance. Until then, do not use the canary for durable
user data or claim the Confidential run accepted.

The accepted Runner, runtime-control, recovery, and agent-boundary documents
govern without being reopened here. This run adds three Phala-specific scope
guards:

- Product work stays on Finite Chat/Agent Platform Channel. Normal agent-local
  repair goes through `finite-agentd` first; the dashboard gains only the
  existing Core lifecycle escape hatches and no shell, filesystem, environment,
  provider, or inbound Runtime Management Pipe commands.
- Docker, Kata, and Phala use one digest-pinned `linux/amd64` image, immutable
  `/runtime`, and the same `/data` contract. Phala adds only adapter config,
  secret transport, lifecycle, sizing, and attestation.
- Billing, drain, and stop never imply provider deletion. Runtime Retirement
  and Purge User Data remain the separate transitions defined by the active
  control contract.

## Settled product and operating decisions

1. **Sell promises, not providers.** The dashboard offers Standard and
   Confidential Hosting Tiers. It may explain the confidential-computing
   properties accurately, but it never submits `kata`, `phala`, an instance
   type, or a provider handle. Core maps the selected entitled tier to
   placement.
2. **Keep both current prices.** Standard remains $200/month and maps new
   runtimes to the current Kata policy. Confidential is $300/month and maps new
   runtimes to Phala Medium. Amount and interval remain Stripe Price facts, not
   hard-coded entitlement arithmetic.
3. **Pin placement at creation.** Billing lapse, Portal activity, a future
   price remap, or a changed tier offer cannot move an existing runtime.
   Cross-Runner migration is a later explicit lifecycle design, not part of
   this run.
4. **Use the Phala API, never the Phala CLI.** Production, tests, runbooks, and
   Nix dependencies must contain no `phala` subprocess path. The adapter uses a
   typed Rust HTTP client and client-side encrypted environment transport.
5. **Give every Confidential runtime the requested shape.** The adapter asks
   the live instance catalog for `tdx.medium` and refuses creation unless it is
   exactly 2 vCPU and 4096 MB. It never silently substitutes a similarly named
   size. Keep the current explicit 40 GB disk for the first canary rather than
   accepting Phala's 20 GB default; resize only from measured image, recovery,
   and customer-data headroom. Provision and update must assert the returned
   capacity because current Phala surfaces disagree on disk units.
6. **Run one adapter class per worker.** Kata and Phala use distinct workers,
   service identities, source-host ids, credentials, placement capacity, and
   adapter state. They may initially share the control host, so this is not a
   host failure-domain claim. The Runner daemon does not become a
   multi-provider orchestrator.
7. **No destructive ordinary lifecycle.** Phala stop/start/restart/update are
   supported. `DELETE /cvms/{id}` is unreachable from restart, recovery, stop,
   billing, drain, rollback, or generic `destroy`. Runtime Retirement stays
   unavailable until an external Recovery Snapshot is restore-proven and a
   reviewed Phala retirement transition can deprovision the provider CVM while
   retaining that snapshot. Purge remains later and separately authorized.
8. **Start with the honest O1 recovery posture.** The first Confidential
   release may use Phala Cloud KMS and a narrowly controlled Finite-Assisted
   Recovery Authority only if its Operator-Privacy Level says exactly that.
   It must not claim that Finite or Phala is cryptographically excluded. A
   stronger Onchain KMS or user-only authority requires its own accepted
   design and recovery proof.

## What changes seriously

There are five serious changes. Everything else should be an implementation
detail or a deletion of old coupling.

| Serious change | Why it is unavoidable | What stays small |
| --- | --- | --- |
| Hosting Tier becomes a Core-owned placement input | The current customer request carries `runner_class`, Rust/SQL default to Phala while the production dashboard defaults to Kata, and one Stripe Price cannot entitle two different promises safely. | Two closed tiers and two explicit mappings; no catalog, scheduler language, or provider marketplace. |
| RuntimeSpec and Provider Runtime Handle become durable before readiness | The worker currently chooses one process-global artifact and records only late, lossy host/machine strings. A crash after paid provider creation can orphan compute or create a duplicate. | One versioned opaque handle, one persisted immutable spec, and the already accepted `validate/ensure/inspect/adopt` contract. |
| Phala becomes a real API adapter | CLI schema drift already caused three launch failures. Restart ignores desired environment, upgrade is unsupported, and destroy may delete data. | One `phala.rs` adapter next to `kata.rs`; no generic TEE client. |
| Confidential admission gains recovery and attestation gates | Durable provider storage is not backup, and “running in TDX” is not evidence of the exact image, freshness, or a usable recovery path. | Reuse the declared Agent Runtime Recovery Set, repository-wide paid gate, canonical image, boot health, and promotion ladder; add only Phala quote verification and the fenced empty-target restore. |
| Production gains a separately fenced Phala worker | Current Nix deployment defines only one Kata worker and runtime-control leases are too loosely fenced by source host. Paid CVMs also need capacity and orphan accounting. | A second one-class Nix unit with a bound credential, an initial cap of one billable resource, and the same Core lease protocol. |

The user-facing Project model, Agent Principal, Finite Chat protocols,
`finite-agentd`, Runtime Management Pipe direction, Runtime image, `/data`
layout, Finite Private grant model, managed-skills behavior, and Kata
execution mechanics do not change.

## Genericify here; special-case there

| Concern | Generic contract now | Adapter-specific implementation |
| --- | --- | --- |
| Product selection | Hosting Tier, Runner class, and Runtime Resource Class are resolved once and persisted with the Project by Core; Runtime operations reference that placement. | Kata and Phala remain internal placement values. |
| Launch input | One immutable RuntimeSpec carries operation id, Project/Runtime ids, release/image digest, resource class, durable-state identity, endpoints/health contract, bounded environment, and secret references. | Kata translates it to containerd; Phala translates it to compose and encrypted API fields. |
| Provider identity | Core stores a versioned opaque Provider Runtime Handle before readiness and uses it for inspect/adopt. | Phala's handle contains its provision/create identifiers and revision facts; Kata keeps its own label/volume identifiers. |
| Lifecycle | Capability-reported ensure, inspect/adopt, restart, stop, recover, and Runtime Upgrade have provider-neutral outcomes and fencing. | Commands, API operations, polling, rollback mechanics, and attestation remain inside each adapter. |
| Runtime behavior | Same digest, entrypoint, `/data` contract, one-shot recovery boot intent, health, chat, and conformance suite. | Phala mounts the dstack socket only for attestation and supplies TEE-specific evidence; Kata does not emulate it. |
| Dashboard identity | Stable Project/Runtime ids and an explicit provider-neutral contact URL. | Provider ids, node names, app ids, and log URLs never become navigation or authorization keys. |
| Recovery | One versioned Agent Runtime Recovery Set, snapshot format, authority record, Core-owned empty-target drill, and preservation assertions, plus the existing repository-wide paid gate. | Provider volume handling and Phala attestation during restore are backend-specific. |

Do not genericify Enclavia, a local agent, provider pricing catalogs, KMS
governance, or TEE APIs now. A later Enclavia adapter should implement the
same Runner and image contracts independently. A local agent may reuse the
same Agent Platform Channel, image behavior, and data contract without being
forced through SaaS placement or compute lifecycle. Do not add a provider
registry, plugin ABI, multi-adapter worker, local scheduler, or “confidential
provider” super-client in anticipation of either.

## Phala API facts that the implementation must pin

The Phala Cloud API is beta and its narrative documentation currently lags the
live service in places. The adapter must make these assumptions explicit and
test them against a paid staging workspace:

- Base URL: `https://cloud-api.phala.com/api/v1`.
- Authentication: `X-API-Key`. Pin an explicit `X-Phala-Version`; the
  [current official SDK
  source](https://github.com/Phala-Network/phala-cloud/blob/main/go/version.go)
  uses `2026-06-23` while an older versioning page still names `2026-01-21`.
  Never depend on the server default.
- Creation is two-phase: [provision a
  configuration](https://docs.phala.com/api-reference/cvms/provision-dstack-app),
  but provision has no documented general idempotency key. Core must persist a
  client-chosen operation correlation/name before the first request and send it
  in the provision input. Then
  persist every returned identifier and encryption input needed to replay the
  commit, including `app_id`, `compose_hash`, and the environment-encryption
  public key. Verify the signed KMS/application key and its identifier binding
  before encryption. Persist `provisioned_at` and its 14-day expiry, then
  encrypt the complete environment locally and [commit the
  CVM](https://docs.phala.com/api-reference/cvms/create-cvm-from-provision).
  Do not call commit until Core acknowledges that write, and never commit an
  expired provision. Persist the returned CVM record identifiers before
  polling application readiness. If the provision response is lost, enter
  explicit reconciliation by the pre-persisted correlation. Because the public
  API cannot enumerate its provision cache, wait through the recorded expiry
  and confirm no matching app, CVM, or revision before a new fenced Core
  operation may provision again.
- Use [CVM details](https://docs.phala.com/api-reference/cvms/get-cvm-details)
  and operation state for reconciliation. Mutating responses mean an
  operation started, not that the Runtime is ready.
- Use API [restart](https://docs.phala.com/api-reference/cvms/restart-cvm),
  [shutdown](https://docs.phala.com/api-reference/cvms/shutdown-cvm),
  [start](https://docs.phala.com/api-reference/cvms/start-cvm), and
  [update](https://docs.phala.com/api-reference/cvms/update-cvm). Do not use
  documented [delete](https://docs.phala.com/api-reference/cvms/delete-cvm)
  in normal operations because Phala describes it as unrecoverable.
- The live [CPU instance
  catalog](https://cloud-api.phala.com/api/v1/instance-types/cpu) identified
  `tdx.medium` as 2 vCPU, 4096 MB, and $0.116/hour on 2026-07-11. The adapter
  verifies live shape and price before accepting a lease. Region scheduling
  and account quota remain provision-time gates unless a stable documented
  API for them is confirmed in staging.
- [Named volumes are documented to survive restart and
  update](https://docs.phala.com/phala-cloud/production-checklist), but Phala
  publishes no managed snapshot/restore API or durability SLA. Recovery must
  leave the provider through an application-owned encrypted export.
- `public_logs` defaults are unsafe for user workloads. Set it false
  explicitly and use the existing redacted startup/health contract rather than
  public conversation logs.
- Management attestation alone has no caller nonce. Freshness requires the
  canonical image to request a quote through `/var/run/dstack.sock` with
  verifier-chosen report data, followed by verification outside the CVM using
  Phala's [attestation
  guidance](https://docs.phala.com/phala-cloud/attestation/get-attestation).

At the [current quoted
rate](https://docs.phala.com/phala-cloud/pricing), a continuously running
Medium CVM with the retained 40 GB disk is about $89/month for Phala compute
plus storage before bandwidth, Finite Private inference, taxes, or support.
The $300 customer price is fixed by product decision, not derived from this
estimate. Publish actual staging charges in the retained API-only cost/runbook
record and keep a hard billable-resource cap; do not build a metering or margin
engine in this run.

## Queue

Work top-down. Every retained item is required.

### P0 — Make placement and identity Core-owned

- Use the deployment contract's expand/backfill/contract sequence. First add
  nullable Hosting Tier, Runtime Resource Class, placement, artifact, and
  versioned-handle fields with readers that accept old rows; then backfill and
  dual-read/write; enable Confidential only after the rollback window; make
  fields required and remove old public/provider fields in a later generation.
  Prove N−1 Core/Runner readers can still control the new rows and opaque
  handle envelope before contraction.
- Keep Hosting Tier to two values and Runtime Resource Class
  provider-neutral. Keep the existing Billing Class meaning intact; do not
  overload `grandfathered/sponsored/standard` access-origin language with
  hosting placement.
- Give each fact one authority. The Project owns entitled Hosting Tier plus
  resolved Runner/resource placement. A creation or recovery operation owns
  idempotency, provider correlation, provisional-handle history, and desired
  artifact. The Agent Runtime owns the current artifact, Provider Runtime
  Handle, and contact facts. Replacement and restore reuse Project placement
  rather than rerunning current product policy.
- Migrate every existing paid subscription, Launch Code, and unlaunched request
  deterministically to Standard unless it already has explicit proven Phala
  placement. Preserve the placement of every existing runtime. Remove SQL and
  Rust defaults that silently select Phala; new missing or unknown values fail
  closed.
- Remove `runnerClass` and provider instance shape from public Core creation
  requests, dashboard-signed onboarding drafts, route handlers, and browser
  state. Core derives placement only after verifying the paid or sponsored
  entitlement.
- Stop lowercasing or overloading provider handles into
  `source_machine_id`/`source_host_id`. Keep a stable Project/Runtime id for
  dashboard routes and authorization, and store an explicit normalized contact
  endpoint instead of assuming `published_app_urls[0]`.
- Fence creation and control leases by the persisted placement and worker
  identity. An empty advertised class set supports nothing, not every class.
  A Phala worker cannot claim Kata lifecycle work even if configuration ids
  collide.
- Replace the one global Runner token with a rotatable Core credential keyring
  bound to worker identity, advertised class, and source-host id. Support
  overlap rotation, targeted revocation, and tests proving one worker
  credential cannot impersonate the other.
- Split “drain” into “do not accept new creations” while keeping existing
  runtime controls alive. Prove rollback can stop new Confidential placement
  without stranding restart, stop, recovery, or upgrade for existing CVMs.

### P0 — Add only the $300 Confidential delta to the accepted Checkout path

- Begin only after Stripe Checkout Readiness is accepted and removed. Extend
  that proven path and `billing-v0.md` to two recurring Prices: Standard at
  $200/month and Confidential at $300/month. Preserve amounts and intervals as
  Stripe facts and use separate allowlisted Price ids.
- Let Checkout select a Hosting Tier, then have the signed webhook fetch the
  current Subscription and map exactly one recognized recurring Price to one
  tier. Unknown, missing, or conflicting hosted-agent Prices fail closed.
- Store the resulting tier in Core's paid entitlement. Extend operator-issued
  Launch Code Batches with an explicit Hosting Tier so an internal
  Confidential canary does not need fake Stripe state; existing/default
  batches remain Standard.
- Preserve the accepted stale-event, inactive-billing, cancellation, and
  idempotency behavior. Add the tier-specific proof that a changed Price cannot
  mutate existing placement or issue lifecycle work. Disable or reject Billing
  Portal tier switching until an explicit cross-Runner migration contract
  exists.
- Keep the dashboard copy outcome-based and evidence-bounded. It may say
  “Confidential” and describe the accepted Operator-Privacy Level, but cannot
  promise generic TEE security, disaster recovery, or operator blindness from
  the tier name alone.

### P0 — Finish the small generic Runner contract

- Replace the transitional process-global artifact/environment selection with
  a Core-persisted RuntimeSpec bound to each creation lease. A worker may cache
  immutable artifacts but cannot substitute its own tag or digest.
- Evolve `RuntimeLauncher` around the accepted
  `validate/ensure/inspect/adopt/restart/stop` operations. Restart, stop, and
  recover-known-good are mandatory for both sold tiers. Report Runtime Upgrade
  and Runtime Retirement capabilities explicitly; Upgrade is required by this
  run's Phala acceptance, while Retirement may remain unavailable until its
  recovery-safe provider transition exists. Product code and the dashboard do
  not infer support from an OCI artifact or provider string.
- Add a versioned opaque handle write that can occur while a creation lease is
  still launching. For Phala, first persist a client-chosen provider
  correlation/name, then persist all provision outputs required to replay the
  encrypted commit and the provision/expiry timestamps. Wait for Core
  acknowledgement before commit, then persist the returned CVM record
  identifiers before readiness. Retry the same operation id and identifiers
  after ambiguity; never generate a second paid resource merely because the
  worker restarted. A lost provision response enters an explicit
  `provision_unknown` reconcile state in Core's durable ledger. It stays
  blocked through the provision expiry, after which API proof that no matching
  app/CVM/revision exists is required before a new fenced Core operation may
  provision again. Treat volumes only as facts attached to known CVM handles;
  do not assume a standalone volume-list API.
- Define handle schema compatibility and migration rules so deploying or
  rolling back a Runner does not make existing runtimes unmanageable. Core may
  store and version the envelope but cannot interpret provider-specific fields.
- Define empty-target restore as a Core-owned, operator-authorized recovery
  workflow on the same Project and Agent Runtime record, not a standalone
  script/provider path. It fences the old handle, preserves that handle in
  audit history, records the replacement handle before readiness, and
  atomically switches current contact/artifact/handle facts only after restore
  health passes. Every crash boundary resumes or rolls back without allowing
  two ready runtimes with the same Agent Principal.
- Extract only the shared container contract already common to Docker, Kata,
  and Phala: canonical image digest, entrypoint, port/health, `/data` mount,
  bounded environment, secret references, and recovery boot intent. Do not
  move Kata's containerd commands, host paths, labels, or transactional
  replacement logic into a generic provider layer.
- Ensure restart and recover receive the complete current RuntimeSpec rather
  than adapter-local defaults. Secret values remain behind references and
  never appear in argv, logs, handles, compose, database diagnostics, or test
  snapshots.

### P0 — Replace the CLI experiment with a typed Phala API adapter

- Move Phala implementation out of the monolithic runner file into
  `src/phala.rs` beside `kata.rs`. Remove `FC_RUNNER_PHALA_BIN`, CLI JSON
  parsers/fixtures, subprocess invocations, work-directory plaintext env
  files, CLI package dependencies, and CLI operator instructions.
- Implement a narrowly typed HTTP client for the pinned API version, bounded
  timeouts, redacted structured errors, and `Retry-After`-aware retries for
  transient `429`/`503` and genuinely transient `409` responses. After an
  ambiguous timeout, inspect current operation/state before repeating a
  mutation.
- Implement provision → persisted operation handle → encrypted commit →
  persisted CVM handle → bounded provider/app readiness. Use the same
  `app_id + compose_hash` after ambiguity so Phala's documented idempotency can
  adopt one CVM. Track the 14-day provision expiry and never silently
  reprovision when a response is lost or a delayed retry finds stale or
  superseded commit material. Reconcile by the pre-persisted correlation and
  block rather than guess when the API cannot prove absence.
- Encrypt the complete environment locally with Phala's documented envelope
  and send only ciphertext plus the full key-name set. Use a maintained
  official non-CLI helper if one is compatible with Rust; otherwise stop until
  the documented construction is locked to official cross-language test
  vectors and receives focused cryptographic review. No home-grown format and
  no plaintext staging file are acceptable. Verify the signed KMS/application
  encryption key and reject signature, app-id, or KMS-id mismatches before
  encrypting.
- Generate additive compose from the shared container contract: exact
  digest-pinned canonical image, `linux/amd64`, named volume at `/data`, one
  public application endpoint, and no feature-specific Runner environment
  branches. The Phala provision request explicitly selects Cloud KMS for the
  accepted first posture and sets `public_logs=false`.
- Query and assert `tdx.medium == 2 vCPU/4096 MB` and its live price. Treat
  region availability and account quota as provision-time results unless a
  stable documented preflight endpoint is confirmed. Contract-test provision
  and update disk units and assert that returned capacity is exactly 40 GB.
  Capacity reports count actual non-deleted Finite CVMs from the API; `None` is
  never an acceptable active-count implementation. Admission accounting joins
  API-visible apps/CVMs/revisions with Core's durable provision/in-flight
  reservation ledger and volume facts attached to known CVM handles.
- Implement inspect/adopt, start, graceful stop, restart, endpoint discovery,
  readiness, and safe state reconciliation by CVM id. Provider name search is
  an operator inventory aid, not the primary identity path.
- Implement the generic Runtime Upgrade outcome with Phala's compose/digest
  update. Use revision redeploy for rollback only after paid staging proves
  whether it preserves the current complete environment; otherwise submit the
  prior compose/digest together with the current complete environment.
  Model update/rollback as a fenced state machine with exactly one `/data`
  writer: the old revision is stopped/detached before a replacement may write,
  and ambiguous transitions reconcile fail-closed before retry. Preserve the
  same named volume, Agent Principal, and endpoint contract, and verify the
  resulting digest and environment key set before committing Core facts.
- Transport recover-known-good as a versioned one-shot boot intent in the
  complete RuntimeSpec. Because Phala environment updates replace the full set,
  prove recovery cannot drop an existing key or leave the Runtime permanently
  in recovery mode.
- Make destructive delete and ordinary `destroy` unsupported capabilities.
  Inventory API-visible Finite apps, CVMs, and revisions; reconcile them with
  Core's provision/reservation ledger and CVM-attached volume facts; alert on
  mismatches with safe identifiers. Never automatically delete an unknown
  resource.
- Prefer bounded polling/reconciliation for this first API adapter. Do not add
  a webhook receiver unless staging evidence proves polling cannot meet the
  declared readiness objective.

### P0 — Make the shared image recoverable and attestable

- Keep one image build and promotion workflow. Every Phala test records the
  exact digest that passed the immediately preceding Kata rung. CI rejects a
  second Phala Dockerfile, mutable production image tag, alternate entrypoint,
  provider-specific Hermes config, or provider-specific skills source.
- Finish the existing recover-known-good design as a narrow, idempotent,
  image-owned boot mode shared by Docker, Kata, and Phala. It may repair only
  generated/bootstrap Finite Chat and Hermes state already allowed by the
  runtime recovery plan. It preserves Agent Principal material, MLS/client
  store, Hermes memory, workspace, user-installed tools, connections, and
  skills; detected identity/store corruption stops with an escalation state
  instead of minting a new agent.
- Produce the existing redacted startup report/health facts needed to classify
  boot, chat readiness, recovery action, artifact, writable state roots, and
  failure code without public logs or a shell. Do not add feature data or an
  inbound Runtime Management Pipe.
- Declare the Agent Runtime Recovery Set and snapshot format. It must cover the
  full `/data` contract, including `/data/workspace`, and the key/manifest
  material needed to restore the same Agent Principal and product state. The
  optional entrypoint Restic path now snapshots `FINITE_AGENT_STATE_ROOT`,
  default `/data`, and hardening evidence must reject the legacy `/data/agent`
  snapshot root. That closes only the filesystem-root coverage defect; it is
  not proof of an application-consistent barrier, independently recoverable
  key authority, or the existing
  service-consistent empty-target restore gate for Core, Finite Chat/Hosted Web
  Device, Sites, Brain, and identity/recovery authorities, or for the required
  non-chat user export and dashboard recovery disclosure.
- Add an image-owned application-consistent snapshot barrier: quiesce or
  checkpoint each live store, produce a versioned integrity manifest, and
  resume only after the snapshot boundary is durable. Test an active chat turn
  and a crash before, during, and after the barrier; a filesystem checksum of
  an uncoordinated live copy is not acceptance.
- Encrypt each snapshot with an independently recoverable per-agent data key
  that survives total loss of the source Phala workspace, CVM, and Cloud KMS
  path. Wrap that key to reassignable Recovery Authority envelopes so
  Finite-Assisted Recovery use is consented/audited and that authority can
  later be revoked or removed without rewriting user data.
- Store encrypted snapshots outside Phala with integrity/version metadata,
  retention, failed-backup alerting, and a documented restore command. Drill
  decryption with the source Phala/KMS path unavailable. Publish observed
  RPO/RTO in retained recovery/runbook documentation and obtain Paul's
  acceptance before customer copy promises either.
- For empty-target restore, stop and fence the source before the replacement
  can open the same Agent Principal or `/data`. Core enforces exactly one ready
  Runtime for that Project across the transition and performs the audited
  handle/contact switch described above. An explicitly authorized restore
  drill may add one separately capped stopped-source retention exception only
  after the source is stopped; no other Phala provisioning may occur while
  that exception is open.
- Make the restored replacement the canonical canary and keep the old CVM
  stopped/retained with a named owner, expiry, storage-cost record, and
  separately authorized recovery-safe Runtime Retirement handoff. At expiry it
  must be retired after reconfirming the retained external snapshot, or Paul
  must explicitly extend the exception; it never rolls silently into ordinary
  capacity. Prove identity, chat membership, Hermes memory, workspace checksum,
  connections, installed baseline, user overrides, and normal future restart
  on the replacement. Only this evidence satisfies Agent Runtime Recovery
  Readiness.
- Add the dstack socket mount only in Phala compose and the smallest
  image-owned nonce quote surface needed for attestation. Verify externally:
  nonce freshness, Intel TDX quote/certificate chain, fresh DCAP collateral,
  acceptable TCB status, expected RTMR event replay, exact compose hash, exact
  image digests, and the accepted OS/KMS measurements. Store a redacted
  evidence record tied to the Runtime and Product Release.
- Update the release's Operator-Privacy Level and Break-Glass wording from the
  tested Cloud KMS, logs, attestation, snapshot authority, and restore facts.
  Marketing or UI copy cannot outrun this evidence.

### P1 — Add the isolated production worker and cost controls

- Define a second Nix-managed Runner unit/environment with
  `FC_RUNNER_CLASS=phala`, a unique worker/source-host id, its own Core Runner
  credential, a host-only Phala API key, a distinct state directory, and no
  Phala CLI package. Run it as an unprivileged, systemd-sandboxed service with
  credentials loaded by systemd, no Kata/containerd/CNI socket or host-runtime
  privilege, and network access limited to required Core and Phala endpoints.
  Document secret names and locations only. If it shares the Kata control host,
  record that host as a common failure domain.
- Keep the Kata unit unchanged except for shared contract/conformance changes.
  Prove both workers can run concurrently and that each claims only its own
  creation and lifecycle work.
- Add startup preflight for pinned API version, account identity,
  `tdx.medium` shape/price, private-log policy, and readable inventory. Treat
  region scheduling and account quota as provision-time gates unless a stable
  documented preflight appears. A failed preflight prevents new Phala leases
  without disabling controls for already known CVMs.
- Begin with an ordinary admission cap of one current billable/non-deleted
  Finite Phala resource or Core-ledger in-flight reservation, not merely one
  running CVM. Empty-target restore may add exactly one stopped-source
  retention exception under the owner/expiry/no-new-provisioning rules above;
  it is not ordinary capacity. Reconcile API-visible apps/CVMs/revisions plus
  Core reservations and CVM-attached volume facts before and after live tests;
  expose creating/running/stopped/retained/unknown counts and actual charges.
  Add dollar and elapsed-time alarms, non-destructive shutdown for a positively
  identified orphan, and a named retirement/retention handoff for each retained
  resource. Raising ordinary capacity requires Paul to compare observed Phala,
  bandwidth, inference, backup, and support cost against the $300 tier.
- Replace the Docker-only `phala_durable_smoke` claim with an explicitly
  opt-in live Phala workflow. It accepts secrets only from the CI secret store,
  creates at most one new CVM within the billable-resource cap, always performs
  non-destructive reconciliation, and leaves a clear operator handoff instead
  of deleting data after uncertainty.
- Add an API-only runbook for preflight, deploy/adopt, status, restart,
  recovery, upgrade/rollback, graceful stop, attestation, restore, inventory,
  stuck operation handling, and cost inspection. It must say when to stop and
  escalate; it must not instruct an operator to reach for the CLI or delete a
  CVM.

### P1 — Evaluation ladder and failure injection

- Add exact API fixtures and a fake Phala server for headers/versioning,
  instance discovery, signed encryption-key verification, two-phase create,
  provision expiry/supersession, encrypted-env envelopes, response schema
  drift, disk units, transient/permanent errors, polling, lifecycle, update,
  rollback environment semantics, private logs, capacity, and redaction.
  Fixtures contain no real identifiers or secrets.
- Run the same Runner conformance suite against fake, Docker, Kata, and Phala:
  duplicate ensure, stale fencing, restart, stop/start, recovery, artifact
  identity, health, secret non-disclosure, and unsupported destructive
  capabilities.
- Inject worker termination after provision response, after provision-handle
  persistence, after create response, after CVM-handle persistence, and before
  application readiness. A lost unpersisted provision response blocks in
  inventory reconciliation; every later retry adopts one resource or fails
  closed, and Core/provider inventories converge without blind reprovisioning.
- Inject crashes before/after every update/rollback writer transition and
  before/after the empty-target restore handle switch. Prove exactly one
  `/data` writer, one ready Agent Principal, a retained auditable old handle,
  and a recoverable current handle at every boundary.
- Extend the accepted billing suite only for both Prices, both Launch Code
  tiers, unknown/conflicting Prices, backwards-compatible Standard migration,
  immutable existing placement, and the absence of provider fields in browser
  requests, URLs, signed drafts, and public DTOs. Rerun rather than duplicate
  the inherited stale-webhook, inactive-subscription, and idempotency cases.
- Run the canonical image tests for fresh seed, user skill override,
  multi-turn Hermes, memory, Sites, Finite Private, normal restart, deliberately
  broken generated chat state, recover-known-good preservation, and known-good
  image rollback. The local Docker rung must pass before Kata; Kata must pass
  before any live Phala spend.
- Add a real Standard/Kata SaaS regression from dashboard entitlement through
  Core's class-fenced lease and Kata worker to chat plus lifecycle controls.
  The Apple Container `saas-smoke` is not this gate. Run the corresponding
  unchanged Standard/Kata path before the Confidential/Phala browser flow.
- In the live Phala staging account, verify the API version and schemas,
  signed encryption-key binding, exact Medium shape, 40 GB provision/update
  units, region scheduling/quota failure behavior, provision expiry, operation
  idempotency after forced client timeouts, volume and environment survival
  across restart/stop/update/rollback, private log behavior, nonce-bound
  attestation, empty-target restore, actual billing granularity, and a
  zero-mismatch API inventory/Core-ledger reconciliation.
- Run the complete accepted Phala rung from
  `hermes-runtime-test-matrix.md` against that same live CVM, including
  invite/status identity, Hosted Web Device plus a second Device, multiple real
  Hermes turns, memory after restart, Sites, Core-provisioned Finite Private
  with no operator key override, skills behavior, attestation evidence, and
  empty-target recovery.
- Run relevant root `just`/Nix gates, Rust formatting/clippy/tests, dashboard
  lint/test/build, runtime image tests, secret scanning, and docs/static
  checks. Publish immutable release facts/evidence locations in the retained
  Product Release/compatibility record and operational evidence in the API-only
  runbook. The run may link to those records but is not their archive. Never
  commit credentials, raw quotes containing sensitive material, user data, or
  provider console dumps.

### P1 — Rollout, rollback, and handoff

- Ship through explicit generations: deploy additive readers/schema and the
  dark adapter; backfill and dual-read/write while Confidential placement
  stays disabled; prove the N−1 handle reader and the existing two-generation
  Runtime Upgrade compatibility gate; then enable the canary. Contract old
  fields only after the rollback window and after existing Standard/Kata
  creation and controls pass unchanged.
- With explicit authorization, enable one internal Confidential Launch Code
  canary, then the $300 Stripe test-mode path. Only after the full acceptance
  evidence may Paul authorize a production Price. An invited paid cohort still
  requires separate green recovery, untrusted owner-claim, and stuck-launch
  gates.
- The immediate rollback is to disable new Confidential Checkout,
  Confidential Launch Code issuance, and new Phala placement. Keep the Phala
  worker and compatible handle reader running for lifecycle control of every
  existing CVM.
- Rollback never converts Confidential to Standard, changes an image silently,
  stops a healthy runtime, revokes recovery keys, retires compute, or deletes a
  volume. Failed or ambiguous creations remain in reconcile/adopt or explicit
  stopped-retained state until a human resolves inventory.
- Put external handoffs for the Phala staging/production workspace and API key,
  approved region/quota, Stripe test/production Price ids, recovery storage and
  authority custody, DNS/public endpoint policy, Nix deploy, and first-cohort
  admission in retained `infra/README.md`/runbook or deployment docs. Do not
  fake these with placeholders that appear production-ready.
- Before acceptance, extract durable terminology to `CONTEXT.md`, any truly
  cross-cutting decision to an ADR, API operations and recovery/cost objectives
  to retained runbooks, deployed artifact facts to `compat/matrix.toml` or the
  Product Release manifest, and completed migration facts to the migration
  log. The closing commit deletes this run per run-document governance.

### P1 — Acceptance Request

- Produce the exact Acceptance Request defined in `README.md`, including the
  deployed revisions, dedicated account/workspace, expected observation after
  each paid-provider, recovery, and attestation step, stop conditions, and
  estimated minutes. Paul completes it and the acceptance statement at the
  top. The final
  check includes the browser experience, real chat, recovery-mode outcome,
  restored data/identity, attestation summary, provider inventory, and observed
  cost. No automated suite or operator-only API transcript substitutes for it.

## Stop and escalate

Stop new Confidential admission and ask Paul for direction if any of these is
true:

- the live `tdx.medium` shape, region, quota, API version, or price differs from
  the pinned staging contract, or provision/update does not return exactly the
  intended 40 GB disk;
- no reviewed non-CLI environment encryption path can be verified against
  official test vectors;
- a signed encryption key cannot be verified and bound to the expected app and
  KMS, or a lost/expired/superseded provision cannot remain blocked through
  expiry and then reconcile no matching app, CVM, or revision before fresh
  provisioning;
- an ambiguous create cannot deterministically adopt one CVM, or provider/Core
  inventory contains an unexplained Finite resource;
- restart, update, rollback, or recovery loses a key, changes Agent Principal,
  requires re-pairing, or mutates user-owned data;
- public logs must be enabled to operate the service, or safe diagnostics
  expose messages, invite material, keys, environment values, or file content;
- attestation cannot prove freshness and the exact accepted artifact/config,
  or the desired Confidential copy is stronger than the Cloud KMS/O1 evidence;
- the Agent Runtime Recovery Set cannot restore consistently onto an empty
  target, the source/replacement single-active fence fails, the repository-wide
  paid-cohort restore/export/disclosure gate is incomplete, the accepted
  Recovery Authority becomes unavailable, or observed recovery objectives are
  unacceptable;
- any billable or in-flight Phala resource exceeds the configured count,
  dollar, time, owner, or retention boundary;
- Stripe/Portal behavior could move an existing runtime indirectly, or the
  second Price cannot map unambiguously to one Core entitlement;
- rollback would require deletion, migration, or abandonment of an existing
  Confidential runtime.

## Out of scope

- Enclavia implementation, selection, pricing, or production evaluation.
- Local-agent packaging, discovery, SaaS scheduling, or dashboard lifecycle.
- A provider plugin system, generic confidential-compute API, multi-provider
  worker, dynamic provider catalog, or automatic cheapest-provider scheduler.
- Cross-Runner migration, automatic tier switching, high availability,
  replicas, shared volumes, or live failover.
- Onchain KMS governance, user-only Recovery Authority, O2/O3 privacy, or
  removal of Finite-Assisted Recovery before an equivalent empty-target
  recovery path passes.
- Arbitrary rescue shell, SSH, file browser, environment editor, provider
  console proxy, or dashboard-driven agent configuration.
- Runtime Management Pipe commands, feature-specific status, or changes to the
  Finite Chat/Agent Platform protocol beyond the already accepted narrow
  recovery behavior.
- Public launch, production customer admission, creating external Price/API
  objects, or production deployment without separate authorization.

## Governing repository documents

- [Run-document governance](README.md)
- [Monorepo doctrine](../monorepo-doctrine.md)
- [Fedimint monorepo structure analysis](../fedimint-monorepo-structure-analysis.md)
- [Recoverability precedes operator-blindness](../adr/0001-recoverability-precedes-operator-blindness.md)
- [Managed skills are product revisions](../adr/0002-managed-skills-are-hot-swappable-product-revisions.md)
- [`finite-agentd` is the agent-owned boundary](../adr/0003-agentd-is-the-agent-owned-platform-boundary.md)
- [Finite Computer v2 language](../../finitecomputer-v2/CONTEXT.md)
- [Runner Contract v1](../../finitecomputer-v2/docs/runner-contract-v1.md)
- [Runtime Control Contract](../../finitecomputer-v2/docs/runtime-control-contract.md)
- [Runtime recovery and observability plan](../../finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md)
- [Hermes Runtime test matrix](../../finitecomputer-v2/docs/hermes-runtime-test-matrix.md)
- [Billing v0](../../finitecomputer-v2/docs/billing-v0.md)
- [Finite stack deployment](../../finitecomputer-v2/docs/finite-stack-deployment.md)
- [Pairing and SaaS postmortem](../../finitecomputer-v2/docs/postmortem-pairing-and-saas-2026-07-06.md)
- [Infrastructure contract](../../infra/README.md)
