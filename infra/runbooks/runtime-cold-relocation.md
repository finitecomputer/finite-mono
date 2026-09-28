# Cold-relocating one stopped Kata Runtime

This is an operator-only, one-Runtime move between Finite-owned Kata hosts.
It preserves the existing Runtime ID, Agent Principal, durable state ID, image
artifact, and state schema. It does not retire or purge the source. The source
compute and state remain stopped and intact until the target has passed its
observation window.

The first intended drill is the existing Upgrade Canary 0715 from lat1 to
lat3. Replace every placeholder below with a fresh read-only observation; the
name is not authority to select a Runtime.

## PRECONDITIONS

- The Core, source Runner, and target Runner run the reviewed generation that
  contains the `runtime_relocation.v1` contract.
- For an Agent enrolled in Core authentication, the target Runner must also
  advertise `supportsRelocationCredentials` in its lease capacity, which it
  does when `FC_RUNNER_RUNTIME_CORE_URL` is configured. Core will not lease
  that relocation to an older Runner. A new Runner talking to an older Core
  stops before provider work and retries. Quiesce already-leased relocations
  before deploying this Core change: the capability gate cannot recall work an
  old Runner has already claimed.
- A full lat1 Borg archive completed successfully after quiescing the hosted
  services, and its archive is visible from the independently held recovery
  credentials.
- Core shows the exact Kata Runtime bound to the expected source host and
  machine. Capture its Runtime ID, durable state ID, artifact, state schema,
  and Agent Principal (`npub`).
- The target Runner uses a different `source_host_id`, supports the same
  artifact/schema, advertises the same persisted Runtime capabilities, has
  enough space, and has no compute or durable directory for this Runtime.
  In particular, a Runtime with `runtime_retirement=true` may move only to a
  Runner with its dedicated restricted retirement Borg recovery set configured
  and tested. Do not silently downgrade the persisted capability or copy a
  broad host-backup credential to satisfy this check.
- Name the recovery boundary for writes made after the move. A bounded canary
  drill may use the stopped source archive plus a clearly labelled
  post-relocation best-effort archive. Before normal use, the target must have
  scheduled off-host coverage for its canonical durable root.
- There are no pending/running controls or retirement snapshot for the Runtime.
  Once the relocation is enqueued, Core refuses new controls for the Runtime
  (restart, stop, destroy, upgrade, including fleet rollouts) until the
  relocation is running, failed, or cancelled and released by its target
  Runner. To return to the source instead, follow CANCEL A RELOCATION.
- Before any Core or Runner deploy, no relocation may be `requested`,
  `launching`, or `cancelled` with its lease still held. `scripts/finite-status`
  does not list the last kind; use the RECONCILE query. Do not roll Core back
  while any cancelled relocation still holds its lease.
- The normal typed `stop` request has succeeded. Do not substitute
  `nerdctl stop`; Core must also record the Runtime offline.
- The source and target Runner timers are drained while staging and reviewing
  the request, and no untargeted ordinary creation request is claimable before
  the target Runner is allowed one lease attempt.

Abort on any mismatch. Do not delete, rename, or modify source state as part of
this procedure.

## ABSENT-COMPUTE RECOVERY VARIANT

For a Runtime whose source compute NO LONGER EXISTS (container and task both
gone, for example cleared by a containerd restart after a poisoned record), the
stopped-container preconditions above are unsatisfiable: the runtime reads
`stale`, not `offline` (a stop against absent compute fails, and failed
controls mark it stale), and no succeeded stop receipt can exist for the
binding. The `--source-compute-absent` flag on the enqueue accepts exactly
those two deviations; every other exact-match check still applies, and the
attestation is recorded in the `runtime_relocation.v1` envelope for
lease-time validation.

Before using the flag, the operator MUST run the bounded absence probe on the
source host and see both results exactly:

```sh
timeout 15 nerdctl --namespace finite inspect '<SOURCE_MACHINE_ID>' ; echo "exit: $?"
# required: fatal "no such object <SOURCE_MACHINE_ID>", NOT a timeout, NOT
# "context deadline exceeded" (that is a poisoned record, a different repair)
timeout 15 ctr -n finite tasks list | grep -c '<SOURCE_MACHINE_ID>'
# required: 0
```

Absence is a stronger single-writer guarantee than a stop receipt, because no
compute exists to resume writing, but only when genuinely proven: a probe
that times out or errors proves nothing and the flag must not be used.
A clean "no such object" is still only a record, not the process: the Runner additionally tests the writer itself (a non-blocking `flock` on `agent/client.sqlite3.writer-lease` plus a two-instant change manifest of the tree) and refuses the attestation with `DurableStateRootLive` if anything holds the lease or the tree keeps changing. An orphaned Kata VM (`containerd-shim-kata`, `qemu`, `virtiofsd` still alive with no record) must be found and stopped first.

**Recovery boundary for this variant.** The full-host quiesced Borg archive
precondition may be replaced by a SCOPED boundary, because the only state at
risk is one already-cold durable tree: record the `state-manifest` hash
(step 1) and take a dedicated off-host archive of the single durable
directory before the transfer. The stopped source tree still remains intact
on the source host until the observation window passes.

Everything else in STEPS applies unchanged, with step 1's "container exists
and is stopped + stop receipt" replaced by the probe above, and step 3's
enqueue carrying `--source-compute-absent`.

## STEPS

### 1. Capture the exact stopped source

From Core and the source host, record:

```text
PROJECT_ID
RUNTIME_ID
SOURCE_HOST_ID
SOURCE_MACHINE_ID
DURABLE_STATE_ID
RUNTIME_ARTIFACT_ID
STATE_SCHEMA_VERSION
EXPECTED_AGENT_NPUB
```

Verify the canonical source container exists and is stopped, the Core stop
receipt succeeded for this exact binding, and the durable tree is the one
named by the RuntimeSpec. Keep the source compute stopped.

Locate the deployed Runner binary from
`systemctl cat finite-saas-runner.service`. Use that exact binary on both
hosts so the manifest algorithm is identical:

```sh
sudo <runner-bin> state-manifest \
  --path '<source-work-root>/kata/<durable-state-id>'
```

Record the 64-character `SOURCE_MANIFEST`. The command follows no symlinks,
hashes file contents, paths, modes, and symlink targets, and rejects special
files except the known Hermes Unix sockets: `agent/hermes-home/gateway.sock`
and `agent/hermes-home/state/gateway.loop-tick.<positive-decimal-pid>.sock`.
Hermes recreates these control and loop-liveness sockets on startup; they carry
no durable state. Regular files or symlinks at those paths are still hashed.
GNU tar omits sockets, so the
stopped source and restored tree have the same manifest without deleting the
socket from the source. Any other special file remains a hard failure.

Use the same socket-aware Runner binary for both manifests. Older binaries
reject the socket rather than accepting a partial proof; trees without the
socket retain their existing v1 hashes. This exception does not prove that a
Runtime is stopped: the Core stop receipt and provider checks above remain
required.

### 2. Stage a provider-independent copy

The transfer below is initiated on the operator Mac. SSH encrypts both hops;
the Mac forwards the stream and does not retain a plaintext copy. The full Borg
archive remains the off-host, independently recoverable worst-case copy.

First create only the exact absent target parent:

```sh
ssh <target-host> \
  "sudo install -d -m 0700 '<target-work-root>/kata'"
```

Then stream one stopped durable directory, preserving ownership, modes, ACLs,
xattrs, hard links, and sparse files:

```sh
ssh <source-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<source-work-root>/kata' -cpf - '<durable-state-id>'" \
| ssh <target-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<target-work-root>/kata' -xpf -"
```

Do not add `--dereference`. Do not use a recursive copy that can cross into
another Runtime.

On the target, compute `TARGET_MANIFEST` using its deployed Runner binary:

```sh
sudo <runner-bin> state-manifest \
  --path '<target-work-root>/kata/<durable-state-id>'
```

Require `TARGET_MANIFEST` to equal `SOURCE_MANIFEST` exactly. Also require that
the target has no container named `SOURCE_MACHINE_ID`.

### 3. Enqueue the exact relocation

On the Core host, load `/etc/finite/core.env` in a root shell without printing
it, then invoke the system-installed Core CLI:

```sh
sudo sh -c '
  set -a
  . /etc/finite/core.env
  set +a
  exec /run/current-system/sw/bin/finite-saas-core runtime-cold-relocate-exact \
  --project-id "<project-id>" \
  --expected-agent-runtime-id "<runtime-id>" \
  --expected-source-host-id "<source-host-id>" \
  --expected-source-machine-id "<source-machine-id>" \
  --target-source-host-id "<target-source-host-id>" \
  --expected-agent-npub "<expected-agent-npub>" \
  --durable-state-manifest-sha256 "<source-manifest>" \
  --admin-email "<operator-email>" \
  --admin-workos-user-id "<operator-workos-user-id>"
'
```

Review the returned request. It must contain the existing Runtime ID, exact
target host, and `runtime_relocation.v1` envelope. Re-enable only the target
Runner timer.

The target Runner fails closed unless:

- the request is leased by the exact target host;
- RuntimeSpec, Runtime ID, durable state ID, machine name, and target path all
  agree;
- the staged tree still matches the approved manifest;
- `agent/identity/identity.json` is a regular file;
- target compute is absent before launch; and
- the launched `/contact` endpoint exposes `EXPECTED_AGENT_NPUB`.

Only after those checks does Core replace the source binding. The Runner
resolves fresh target-host secrets through the normal launch path; durable
state is never used as the secret transport.

For an enrolled Agent, the authenticated relocation-credential endpoint prepares
one inactive successor credential for the exact live lease. Retries reuse it.
The predecessor remains bound until completion, when Core atomically revokes
it and activates the successor on the target. If the lease expires before the
successor activates, the whole completion rolls back: the predecessor stays
current and the request stays launching until its lease is retried or the
Runner records the failure. Hosted access preferences and
native credentials carry forward from their latest committed state; routing
waits for the new process to acknowledge the current configuration generation.
An Agent that was never enrolled stays unenrolled. A revoked or inconsistent
credential fails closed; this operation cannot repair historical credentials.

## VERIFY

- The relocation creation request is `running`.
- Core still has the same Project, Runtime ID, artifact, state schema, and
  Agent Principal, now bound to the target host and same machine name.
- The target container is running and healthy.
- For an enrolled Agent, Core configuration polling succeeds with the target
  credential, the predecessor no longer authenticates, and hosted access is
  ready after the target acknowledges its configuration. A subsequent canary
  upgrade must obtain the current credential successfully. Record outcomes,
  never credential values.
- Finite Chat receives a round trip from the existing Agent Principal.
- Sites, Brain, workspace files, Hermes memory, and installed skills expected
  for the canary are present.
- Source compute remains stopped and source durable state still exists.
- No source Runner work was allowed to restart the old binding.
- The target Runner is still drained after its bounded lease attempt.
- The target has a named recovery archive for post-relocation writes, or the
  canary remains inside the explicitly bounded observation window while
  scheduled off-host coverage is completed.

Observe the canary before broad use. Record request ID, both manifests, exact
source/target bindings, Borg archive name, timestamps, and verification result;
record no secret values.

## ROLLBACK

Before Core switches the binding, the target Runner removes target compute (or
stops it when removal fails) and then proves shutdown from the durable tree
before it records a failure: no running container record binds the tree, nobody
holds the chat store's writer lease, and the tree does not change during the
observation window. A missing provider record alone is not proof. When shutdown
is unproved the Runner records nothing and prints
`relocation_target_shutdown_unproved`; the request keeps refusing source
controls. For a `launching` request the Runner's next cycle after the lease
expires tries again. A cancelled or running request is never leased again, so
there the recovery is an operator step (see RECONCILE). A `failed` request is
still not proof that compute is absent: removal can fail after the stop, and a
stopped container remains. The proof also cannot see an orphan that neither
holds the lease nor writes during the window. Core keeps the existing Runtime/link and both durable trees.
Verify compute on the target host rather than assuming cleanup succeeded. A booted
target may have changed the staged manifest even when Core rejected the final
registration. Preserve that tree under a request-specific, non-canonical name,
then restage the absent canonical path from the stopped source only after
diagnosing the failure.

If the Runner cannot tell whether Core committed completion (the response was
lost, Core or its proxy returned a 5xx, or the body was unreadable), it stops
the target first and then records the failure under its lease:

- Core accepts the record: the relocation failed, and the Runner removes the
  stopped target.
- Core refuses it or does not answer: the Runner leaves the target stopped and
  prints `completion_unconfirmed`. A refusal usually means completion committed.
  The Runner never starts compute on its own; a committed relocation is brought
  back with an ordinary typed restart, which goes through Core's current
  binding and exclusion checks.

For `completion_unconfirmed` or `relocation_target_shutdown_unproved`, follow
RECONCILE ONE RELOCATION REQUEST before touching either side.

Failure or cancellation revokes only the pending successor credential. It does
not re-activate stopped source compute or alter the predecessor's existing
activation state. A fresh relocation request receives a fresh successor.

After the first successful credential handoff, keep Core on a version that
understands relocation-owned credentials. Older Core versions cannot provision
a later upgrade from that credential lineage. If Core must be rolled back,
hold relocations and upgrades until a compatible version is restored; do not
unrevoke or rebind credential rows manually. Existing runtime authentication
uses the same schema and token contract, and no database migration is required.

After Core switches the binding, do not manually start source compute: that
would create two writers. Stop the target through Core first. A reverse
relocation requires a new exact transaction, but the old source canonical path
now contains a stale copy. Preserve that stale directory under an explicitly
approved, non-canonical rollback name; then stage the stopped target tree into
the absent canonical path, verify its manifest, and use the same contract with
source/target reversed. The current implementation intentionally does not
rename or delete either copy automatically.

If the target modified durable state and cannot be stopped cleanly, fail closed.
Preserve both sides and restore the named pre-move Borg archive to an empty
recovery target rather than guessing which tree is canonical.

## PROVE TARGET SHUTDOWN ON THE HOST

Core cannot check target compute. When a step below asks you to prove target
shutdown, collect all of this on the target host and keep it with the incident
record:

```sh
timeout 15 nerdctl --namespace finite inspect '<SOURCE_MACHINE_ID>'; echo "exit: $?"
# required: "no such object", or a status that is not running
timeout 15 ctr -n finite tasks list | grep -c '<SOURCE_MACHINE_ID>'
# required: 0
pgrep -af 'containerd-shim-kata|qemu|virtiofsd' | grep -F '<durable-state-id>'
# required: no output
test ! -e '<target-work-root>/kata/<durable-state-id>/agent/client.sqlite3.writer-lease' ||
  sudo flock -n -E 75 '<target-work-root>/kata/<durable-state-id>/agent/client.sqlite3.writer-lease' true
echo "exit: $?"
# required: 0 (75 means a live process holds the writer lease)
```

A timeout or error proves nothing. An unreachable host is not a stopped host:
if the target host cannot be reached, fence it outside Finite (power it off
through the provider console or cut its network) so that neither compute nor a
delayed Runner cycle can resume, and record how.

## CANCEL A RELOCATION

Use this to abandon a relocation that is still `requested` or `launching`, for
example when no capable target Runner can lease it. A `running` relocation
cannot be cancelled; control the new binding instead.

Capture evidence first and keep it with the incident record:

1. The RECONCILE query below for the exact request.
2. On the target host: `timeout 15 nerdctl --namespace finite inspect '<SOURCE_MACHINE_ID>'; echo "exit: $?"`.
3. The target Runner's service state: `systemctl status finite-saas-runner`.

Preview, then run the exact command on the Core host:

```sh
sudo sh -c '
  set -a
  . /etc/finite/core.env
  set +a
  exec /run/current-system/sw/bin/finite-saas-core runtime-relocation-cancel-exact \
    --relocation-request-id "<relocation-request-id>" \
    --expected-agent-runtime-id "<runtime-id>" \
    --expected-target-source-host-id "<target-source-host-id>" \
    --dry-run
'
```

Review the printed result, then repeat the command without `--dry-run`. It
prints only the request, Runtime and target host identifiers, the runner, the
status, `lease_held` and the lease expiry, never the lease token. The command
runs with Core's database access, not an operator identity, and writes no
audit record of its own: the incident record is the audit trail.

- **`requested`: no provider work happened.** The request becomes `cancelled`
  and Core admits source controls at once.
- **`launching`: target compute may exist and may be running.** The request
  becomes `cancelled`, but Core keeps the target Runner's lease and keeps
  refusing source controls. At its next Core call the target Runner is refused,
  removes its target (or stops it if removal fails), proves shutdown, and
  records the failure. That releases the lease. If it cannot prove shutdown, or
  it crashes first, nothing retries: a cancelled request is never leased again,
  and the lease stays held until the next case below.
- **The lease is still held after it expired** (the target Runner is gone,
  printed `relocation_target_shutdown_unproved`, or crashed). Stop the target
  Runner service (`systemctl stop finite-saas-runner`) or fence the host so no
  late cycle can act. Stop any running target compute, then PROVE TARGET
  SHUTDOWN ON THE HOST. Only then run the exact command again with
  `--confirm-target-compute-stopped`, first with `--dry-run`. Core releases the
  lease only after it has expired and only with that attestation, which Core
  cannot verify.

The service route `POST /api/core/v1/agent-creation-requests/<id>/cancel`
(header `authorization: Bearer $FC_CORE_API_TOKEN`, body `{}`) applies the same
rules to `requested` and `launching` requests. It refuses to cancel a cancelled
relocation that still holds its lease. Prefer the exact command, which also
checks the Runtime and target host.

A cancel revokes only the pending successor credential. The predecessor stays
current.

## RECONCILE ONE RELOCATION REQUEST

Decide from the exact request, never from counts or from the newest row. Run
this read-only query on the Core host. It prints no secrets.

```sh
sudo sh -c '
  set -a
  . /etc/finite/core.env
  set +a
  exec psql "$FC_CORE_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
    -v request_id="<relocation-request-id>"
' <<'SQL'
BEGIN READ ONLY;
SET LOCAL statement_timeout = '5s';
SELECT q.id, q.status, q.agent_runtime_id, q.target_source_host_id, q.runner_id,
       q.lease_token IS NOT NULL AS lease_held,
       q.lease_expires_at,
       COALESCE(q.lease_expires_at > clock_timestamp(), FALSE) AS lease_live,
       q.failure_message, q.updated_at,
       r.source_host_id AS bound_host, r.source_machine_id AS bound_machine,
       c.creation_request_id IS NOT NULL AS successor_exists,
       c.agent_runtime_id IS NOT NULL
         AND c.agent_runtime_id = q.agent_runtime_id AS successor_bound,
       c.revoked AS successor_revoked, c.activated AS successor_activated,
       c.lease_sha256 AS successor_lease_sha256
FROM agent_creation_requests q
LEFT JOIN agent_runtimes r ON r.id = q.agent_runtime_id
LEFT JOIN runtime_core_credentials c ON c.creation_request_id = q.id
WHERE q.id = :'request_id' AND q.relocation_spec IS NOT NULL;
ROLLBACK;
SQL
```

Check that `runner_id` is the target Runner that printed the outcome, and that
`target_source_host_id` is the host you expect. Then:

| Result | Meaning | Next step |
|---|---|---|
| `running`; `bound_host` is the target; the successor is bound, activated and not revoked, or there is no successor row for an unenrolled Agent | Core committed this relocation. The target is the Agent. | If the target is stopped, start it with an ordinary typed restart (owner restart or admin restart), which goes through Core's current binding and exclusion checks. If the Runner reported `relocation_target_shutdown_unproved`, the target may still be running as the Agent, which is correct: continue with VERIFY. |
| `running` with a successor that is missing, unbound, revoked or inactive while the Agent is enrolled | Inconsistent state | Change nothing. Preserve both trees and escalate. |
| `failed` | Core recorded the failure and admits source controls. | Recheck `bound_host` and `bound_machine` and any newer relocation for the Runtime: a historical failed row does not prove the current binding. If target compute is running, stop it before anything else, then PROVE TARGET SHUTDOWN ON THE HOST. Preserve its tree under a request-specific name, remove the compute, then continue with ROLLBACK. |
| `cancelled`, `lease_held` true, `lease_live` true | The target Runner has not released it. It may not have learned of the cancel yet, or it learned, could not prove shutdown, and stopped. Source controls stay refused. | If the Runner is still working, wait and re-run the query. If its last output for this request was `relocation_target_shutdown_unproved`, wait for the lease to expire and follow the held-lease case in CANCEL A RELOCATION. |
| `cancelled`, `lease_held` true, `lease_live` false | No Runner will release it: cancelled requests are never leased again. | Follow the held-lease case in CANCEL A RELOCATION. |
| `cancelled`, `lease_held` false | Released. Core admits source controls. | Prove target shutdown as for `failed`. |
| `launching`, `lease_live` true | The target Runner still owns the attempt. | Wait for its cycle and re-run the query. |
| `launching`, `lease_live` false | The attempt stalled. Source controls stay refused. | The target Runner's next cycle re-leases it. For a cross-host move it proves target shutdown before recording failure and, if it cannot, keeps refusing and prints `relocation_target_shutdown_unproved`: find and stop the orphan, then PROVE TARGET SHUTDOWN ON THE HOST. A same-host attempt records failure without touching the Core-bound machine. If the target Runner is down, use CANCEL A RELOCATION. |
| No row, an error, or a timeout | Unknown | Do nothing to either side. Preserve both durable trees and escalate. |

## Recover an already-completed relocation with a revoked credential

`runtime-credential-recover-exact` is a separate operator recovery for the
pre-handoff defect: a completed relocation has no successor credential and the
original creation's credential is still Runtime-bound, revoked and inactive.
Ordinary provisioning must continue to refuse this state. The command requires
an explicit attestation of this incident; a revoked credential alone does not
establish why it was revoked. Remove this compatibility command after the
confirmed affected assignments have recovered or been retired (FIN-117).

Before execution:

1. Deploy Core with relocation-credential lineage and recovery support, and a
   Runner that accepts the explicit predecessor hash on the private upgrade
   credential response. An older Core cannot serve recovery; an older Runner
   rejects this new response field before replacing compute. Ordinary responses
   omit the field, preserving their existing wire format.
2. Use `scripts/finite-status` to establish the exact active owner, Runtime,
   Project, host, machine and expected Principal. Verify direct/published routes
   and unique port ownership. Review the exact original creation and completed
   relocation records. Never choose a record by order or use this on an
   offboarding, owner-changed, stopped or ambiguously relocated Runtime.
3. Record a consistent Core backup and the current Runtime's verified recovery
   boundary. Retain its image digest, durable root and Chat continuity evidence.
   Freeze unrelated lifecycle operations for this Runtime during the repair.
4. Qualify a different target artifact. Using the currently installed artifact
   can take the upgrade's no-op path and leave the new credential undelivered.
5. Run the exact command with `--dry-run`. It rolls back all database writes and
   prints only a Runtime Operation. Review the target before executing without
   that flag under explicit production authorization.

Required arguments are `--admin-email`, `--admin-workos-user-id`, `--project-id`,
`--expected-agent-runtime-id`, `--expected-source-host-id`,
`--expected-source-machine-id`, `--expected-owner-email`, `--expected-agent-npub`,
`--expected-predecessor-creation-request-id`, `--expected-relocation-request-id`,
`--target-runtime-artifact-id`, and `--confirm-relocation-credential-loss`.
There are no credential arguments or credential output.

The transaction retains the predecessor as revoked history, creates one new
credential bound to the completed relocation, preserves hosted-access settings
and resets their application acknowledgement, and enqueues the exact upgrade.
Core writes a typed `runtime_credential_recoveries` receipt in the same
transaction, linking the upgrade to its predecessor and successor. The private
upgrade provisioning endpoint reads that receipt only after validating the live
lease and current assignment. It returns the new secret and the predecessor's
SHA-256. The Runner requires the same Core URL and exactly one matching installed
credential before changing the candidate environment. A mismatch is rejected
without writing a replacement credential. Verification of an already-running target accepts only the new
credential; it cannot substitute a value in memory and claim delivery.

No public API can create recovery authority. Audit events record the operation
but are not authorization inputs. The additive table preserves existing rows
and older binaries ignore it. Keep it and its reader until no failed recovery
can require delivery. The operation does not edit identity, Chat state, data
roots or ports.

After execution, observe the returned operation through Core and run canonical
status again. Prove new credential authentication, old credential denial,
hosted-access acknowledgement, unchanged Principal and routes, retained Chat
history and an actual reply. For FIN-117, only then resume Brain diagnostics
and verify Brain creation, write and read through an authenticated Chat turn.

The command is single-use for an exact legacy state. Repeating it after success
fails closed; it never rotates a credential again. Runner lease retries reuse
the new credential. If the upgrade fails after the transaction,
retain the successor and enqueue an ordinary exact upgrade to the same artifact
on the same owner, Project, Runtime, host and machine. Core may reuse the receipt
of the failed recovery operation only for that same binding and artifact. It
returns the same successor, never another rotation. Other upgrades receive no
replacement authorization. Do not re-run
recovery, restore the old credential or roll the database back over later writes.
Compute rollback retains the old image and data but does not restore Core
access to the old process. Keep compatible Core deployed until delivery and
verification finish. This is not an automatic fleet repair or a promise that
an image rollback also rolls back credentials.
