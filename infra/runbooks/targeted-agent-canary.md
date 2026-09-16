# Target one new-agent canary

A normal Launch Code grants admission without selecting a host. The root-only
`finite-saas-core launch-code-target-exact` command binds one **unused Standard
code** to a registered Kata host. Redeeming that code through the unchanged
browser onboarding flow saves `target_source_host_id` in the creation transaction.
No existing Project, Runtime, or Chat identity is moved.

## Qualification boundary

The target must be drained, empty, and capped at one Runtime before binding.
Run `scripts/finite-status` on the app host and target before and after the test.
Capture those timestamped receipts before binding. Core rejects hosts with any
recorded Runtime and competing code reservations, but it cannot atomically
observe the host-local drain/ceiling. Those two checks are an explicit manual
preflight limitation; do not run the command against an unverified host.
The app-host JSON includes pending/launching requests under
`sections.fleet_convergence.agent_creation_requests`; inspect the exact canary
request and actual Runtime host, not queue order. The same JSON exposes all
bindings under `canary_host_reservations`, including unused/expired/revoked
codes, their batch ids and issuers. A database before migration 0026 reports an
empty reservation list. Keep source Telegram bots alive.

Before issuing or redeeming a code, complete the lat2 deploy helper's running
executable verification. A system closure path, configured unit, changed PID,
or HTTP health response alone is insufficient: the September 15 attempt ran
old Core after an apparently successful activation and launched on lat4.
Treat any executable mismatch as a failed deployment. Correct the exact service
under the reviewed deployment boundary, then repeat canonical status and
executable verification before continuing.

Issue a one-code Standard batch in the signed-in admin dashboard. Save its
one-time plaintext privately; do not put it in command arguments, logs, or git.
Record the code **id**, batch id, and the issuer's existing Core-linked WorkOS
user id. Run the deployed Core binary with `/etc/finite/core.env` in its
root-only environment:

```sh
finite-saas-core launch-code-target-exact \
  --code-id CODE_ID --expected-batch-id BATCH_ID \
  --target-source-host-id finite-lat-5 \
  --operator-email austin@finite.vip --operator-workos-user-id WORKOS_USER_ID
```

The command requires a matching unrevoked host-bound Kata credential, exact
unused/unexpired/unrevoked code and one-code batch, and the code issuer's existing
email/WorkOS binding. Local access to Core's database credentials is the
operator capability; this CLI does not authenticate an interactive WorkOS
session. The command never creates or relinks an operator identity. A repeated
binding to the same host succeeds while the code is unused; retargeting and
post-redemption binding fail. A durable audit event records the first binding.

The database permits one root binding per canary host. Migration 0027 permits
one same-host retry child. `launch-code-retry-target-exact` defaults to a
rollback-only preview; `--execute` rechecks the exact original misplacement and
fresh one-code batch before appending the binding and audit event. It refuses
an occupied target, active control work, and a retry of a retry. Preserve both
bindings and their history during binary rollback. A host named by a binding
accepts **only explicitly targeted creation**.
The restriction survives redemption, revocation and expiry; it is not a timer
or an inference from available capacity. It does not drain existing-runtime
lifecycle operations. Broader capacity release is a separate reviewed change;
this canary command intentionally has no release or retarget switch.

Redeem the code in the regular new-agent form while the target remains drained.
Use the reservation's `creation_request_id` to identify the exact request. Require
`request_status=requested`, the intended `request_target_source_host_id`, and
empty `request_runner_id`, `agent_runtime_id`, and `actual_source_host_id`.
A missing request, missing target, existing claim, or different host stops the
procedure. Preserve the records for investigation. Only after those checks pass,
undrain that host at its one-runtime ceiling.
Verify the exact resulting Runtime, identity readiness and a real Chat reply.
Re-drain after the claim/test. This proves launch for an existing account;
it does not prove fresh account enrollment or the eventual fleet capacity.

## Return the host to the shared pool

Keep the Runner drained. Verify the exact root reservation, successful canary's
actual host, fresh ready health, and effective capacity through `finite-status`.
Every bound code must be consumed, expired or revoked; no targeted creation or
canary control operation may be in flight. Deploy and verify the running Core
executable before using its root-only command:

```sh
finite-saas-core launch-host-release-exact \
  --reservation-code-id ROOT_CODE_ID --source-host-id HOST_ID \
  --expected-canary-runtime-id RUNTIME_ID \
  --operator-email OPERATOR_EMAIL --operator-workos-user-id WORKOS_USER_ID
```

Inspect the rollback-only preview, then repeat with `--execute`. Verify the
release receipt and unchanged canary bindings, requests and Runtime destinations
through `finite-status`. Only then disable drain on that exact host. Ordinary
Launch Codes remain unbound: TRF migrations and new-user onboarding share the
normal queue, claimed by an eligible Runner with capacity. Existing destinations
remain intact. A release changes admission, not enrollment or migration state.

Migration 0029 appends a release receipt and audit event without deleting target
bindings or rewriting user state. Redemption continues to read those bindings;
the creation lease reader ignores released reservations for untargeted work.
Explicit request targets and Runner drain/capacity checks still apply. The database fence refuses new bindings after release, including writes from
older Core executables. The successful canary must belong to the exact root
reservation or its retry. Repeating the same exact release is a no-op.
Core versions supporting 0026–0028 ignore the receipt and conservatively block
ordinary launches on that host. Keep the additive schema on binary rollback;
re-enable drain to stop new admission without deleting any Agent or Chat history.

## Writers, readers and rollback

Migration 0026 adds only `launch_code_host_targets` and its host index. Existing
rows and ordinary codes are unchanged. The operator writer locks the same code
and batch rows as redemption. Redemption reads the binding after acquiring those
locks; a binding committed while it waited cannot disappear behind an older
statement snapshot. The creation insert writes the already-supported target
field atomically. The lease reader filters both targeted requests and reserved
hosts. Missing host identity cannot claim a targeted request. Existing Runner
and dashboard versions need no new wire fields. Runner capacity/drain checks
remain authoritative and are not bypassed by targeting.

Deploy the candidate Core before binding any code, using the normal CI-built
lat2 closure. Capture the pre-deploy Postgres backup/checksum and previous exact
system closure per `postgres-backup-restore.md` and `deploy-core.md`.
Core versions before migration 0026 ignore code bindings and the host reservation: before binary
rollback, drain every reserved host and revoke all unused bound code batches.
Already-created requests retain their host target, but keep the qualification
host drained until a compatible Core is restored. Leave the additive table and
audit history in place. Do not restore a whole database backup over newer user
writes to undo a canary; retain any created Runtime and Chat history.

Migration 0028 retains cohort-child history; the unused batch-targeting command
was retired when the host joined the shared pool.
Core versions supporting 0026/0027 can read those bindings and replay startup
migrations; retain the expanded schema during binary rollback.
