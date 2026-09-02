# Platform rollout — the manual cross-component wave

This page orchestrates a **manual platform rollout**: the coordinated wave that
carries saas-runner upgrades, a saas-core migration generation, the lat1 NixOS
closure (Core, chat server, Hosted Web Device, Sites, Brain, Identity,
Caddy), the dashboard digest, and an agent-runtime image pin through the fleet
in one sitting. Component-level procedures remain the authority for their own
mechanics — this page supplies the ORDER, the GATES between them, and the
verification ritual. Record every completed wave in
[`infra/deployment-changelog.md`](../deployment-changelog.md) (record, never
authority), keep `scripts/finite-status` evidence before and after (standing
rule), and update the `production` branch record per
[ADR 0006](../../docs/adr/0006-production-deploys-use-a-protected-production-branch.md).

**The one invariant everything else serves: runners roll BEFORE Core
restarts.** An upgraded Core writes a control-request vocabulary and runs
schema expectations that the previous runner generation cannot parse or honor
(`runtime_control_requests.status` gains values past `requested/running/
succeeded/failed` in Migration 0021). Reverse skew (new runners under a not-yet-
upgraded Core) is bridged and tolerated. Mixed windows are bridged but not
free: keep the whole wave inside one sitting, hours not days. Lessons encoded
here were paid for across the 2026-07/08 rollout waves and fleet-convergence
exercises; specifics live in those records under fbrain (`fbrain open …`) —
this page commits the rules, not the incident narratives (public repo).

## Source-of-truth map for this wave

| Fact | Authority |
|---|---|
| What runs where, roles | [`infra/README.md`](../README.md) |
| Per-service deploy mechanics | [`deploy-core.md`](deploy-core.md), [`deploy-finitechat-server.md`](deploy-finitechat-server.md), [`deploy-brain.md`](deploy-brain.md), [`deploy-sites.md`](deploy-sites.md) |
| Runtime image build/promotion + serial agent upgrade | [`runtime-image.md`](runtime-image.md) |
| Rollback closure artifacts | `infra/deployments/production.toml` (`rollback_policy`), the `Lat1 NixOS Closure` workflow artifact |
| Why-a-version-shipped record | [`infra/deployment-changelog.md`](../deployment-changelog.md) |
| Fleet truth at any moment | `scripts/finite-status` (read-only; exits 0/1/2 per README standing rules) |
| Control-plane preflight | `scripts/rollout_preflight.py` |
| Emergency rollback surgery for the lifecycle vocabulary | `migrations/runtime_lifecycle_reverse_remap.sql`, `migrations/runtime_upgrade_rollback_rescue.sql` |

## PRECONDITIONS

Everything below gates the START command. Do not begin with an unticked box;
the 2026-08-18 wave proved unexercised lanes fail serially on deploy night.

- **Deployed rev:** the change-set is merged to `main`; CI is green at exactly
  that revision (merge-commit SHA, not your local tree).
- **Preflight census clean:** `scripts/rollout_preflight.py` (see
  [#682](https://github.com/finitecomputer/finite-mono/pull/682)) exits 0 —
  every `runtime_control_requests.status` inside the legacy four values
  {requested, running, succeeded, failed}, zero non-terminal upgrade-kind
  requests, and you have eyeballed the long-lived-running inventory (those rows
  re-label to `launching`). Exit 1 means STOP and disposition rows first;
  Migration 0021 will fail Core startup closed otherwise.
- **Backups are current and named:** the
  [`postgres-backup-restore.md`](postgres-backup-restore.md) drill has been
  exercised within its required freshness window, and a fresh `pg_dump` of the
  Core database is captured to a named location immediately before STEP 3. The
  chat-side snapshot/restore story (Litestream + snapshot drills) is green per
  its own runbooks. Data availability outranks everything else in the wave.
- **Rollback targets recorded before touching anything:**
  - `readlink -f /run/current-system` on every touched NixOS host, copied
    somewhere off-host;
  - the previous `lat1-nixos-closure-<SHA>` artifact confirmed still
    downloadable (14-day retention — re-plan the wave if it ages out mid-day);
  - the previous runtime-artifact digest and dashboard `@sha256:` from the
    pins their authorities name (changelog map above).
- **Runner generation staged first (both Kata hosts):** new runner binaries
  applied and `FC_RUNNER_RUNTIME_ARTIFACT_ID` present EXPLICITLY in
  `/etc/finite/runner.env` on lat1 **and** lat3. There is no implicit default
  anymore; `scripts/finite-status` renders a missing pin as RED/absent — treat
  that as a halt condition. Never hand-edit `runtime_artifact_id` records or
  bypass the promotion path in [`runtime-image.md`](runtime-image.md).
- **Runtime image proven per the release ladder:** built once by the canonical
  workflow, smoke lane passed, immutable digest promoted — local → Docker →
  Kata, never skipping a rung (README release-checklist discipline).
- **Drift hygiene:** every finite-status drift exception is dispositioned
  BEFORE the wave (named in the changelog entry or fixed). Archive retired /
  broken-off records at discovery time; a silent upsert reactivating a retired
  runtime's link cost three consecutive waves their `--roll-all` passes.
- **Host/environment sanity:** builder has >50G free; `LimitNOFILE=65536`
  still declared for the long-running services (a 1024 default here produced
  a full chat outage once); `FC_RUNNER_DRAIN` is explicitly `false` (never
  unset, never left "on" from a prior incident — it pauses ALL new-agent
  creation silently).
- **Single-writer posture acknowledged:** for the whole window nobody runs
  mutating or write-dispatching one-shot CLI invocations beside resident
  processes (diagnostic containers pass `--entrypoint` per the boot-loop
  postmortem class). Second writers have historically poisoned shared durable
  state (MLS ratchet wedge; hermes inbox lease lock-out). Read-only probing
  only, via `scripts/finite-status`.
- **Secrets posture:** nothing in logs, tickets, or notes may quote environment
  dumps from containers or units (this has leaked a live credential before —
  redaction missed the key NAME, not the value). Sealed manifests are checked
  by SHA-256 only; greps near identity material stay pubkey-shaped.
- **TODO(rehearsal, once):** before the FIRST wave that could realistically
  need it, exercise `runtime_lifecycle_reverse_remap.sql` against a scratch
  copy of production state and attach the transcript to the changelog entry.
  Until that drill exists, treat STEP R2 as break-glass assisted by the
  census tool, not routine.

## STEPS

### STEP 0 — Freeze and evidence

Announce window start; one operator holds the mutation pen for the entire
wave. Save `scripts/finite-status --json` (BEFORE artifact) with timestamp and
rev. Open the changelog entry skeleton now so the record is filled in as-you-go,
not reconstructed later.

### STEP 1 — Stage and apply runners (both hosts) — BEFORE Core

Apply the staged runner generation on lat1 and lat3 (runner.env pin included).
Runners-under-old-Core is the tolerated direction; the reverse wedges
stop/restart/upgrade operations fleet-wide. Confirm via finite-status: both
hosts show pin matched/green, service active, and the standing-readiness
section reading plausibly (absent reports project as `unknown` until each
runtime sees its next control operation — acceptable inside the window).

### STEP 2 — Runtime image promotion decision

If the wave carries a new agent-runtime digest, promote it per
[`runtime-image.md`](runtime-image.md) now (it affects NEW launches
immediately and existing agents only via §4a serial upgrade, next step).
Serial rolls pace around 2 minutes per agent — budget the fleet linearly
(~30 minutes for ~19 agents) and let the recorded plan drive instead of
improvising parallelism. If a serial roll halts, RESUME THE SAME PLAN HASH;
the `.local-state/runtime-rollouts` event stream summarized by finite-status
is the progress authority. Stopped agents' ports get squatted while down —
if an identity-bound guard refuses a step, it is protecting you; do not
override it manually.

### STEP 3 — lat1 NixOS closure switch (Core + chat + dashboard + edge)

Follow [`deploy-core.md`](deploy-core.md) and
[`deploy-finitechat-server.md`](deploy-finitechat-server.md) mechanics against
the prebuilt closure from the exact rev (dashboard digest bump lands via
`modules/dashboard.nix` in the same flow). Mid-switch watches:

- If the switch exits nonzero, identify WHICH units failed before retrying
  anything (a monitoring-only failure is not a rollback trigger; a failed
  core/chat unit is). `scripts/deploy-lat2-closure-cache --activate` encodes
  this: it stops the monitoring timers across the switch and re-arms them on
  exit, treats a nonzero switch whose only failed units are monitoring-only
  as a warning, and refuses to revert the chat binary once `finitechat-server
  rollback-check` reports `fold_complete: true` / `rollback_allowed: false`
  (roll forward only).
- **Boot-loop signature to kill the wave on sight:** a gateway process
  SIGKILLed roughly every ~41 seconds, healthcheck red, while control
  requests report success. Bridge/readiness budgets: cold starts can take
  tens of seconds and readiness commonly lags "active" by 30–80 seconds —
  give every gate the generous deadline before declaring failure.
- After ANY restart of a chat-affecting unit: explicitly ensure the Hosted
  Web Device unit started again and answers its `/healthz` (restart coupling
  has silently dropped it before), per
  [`hosted-web-chat-recovery.md`](hosted-web-chat-recovery.md).

### STEP 4 — Serial agent upgrades

Continue/complete [`runtime-image.md`](runtime-image.md) §4a for existing
agents. Expect replies landed in home-chat on freshly restarted surfaces to
be the reply-route fallback doing its job after mass restarts — flag for the
VERIFY round-trip check rather than treating as delivery loss.

## VERIFY

State, not process. Each layer gets a machine check AND one human-feelable
product probe. A layer is green only when both agree.

1. **Closure identity:** `readlink -f /run/current-system` equals the exact
   built SYSTEM path for the deployed rev on every touched host (not exit
   codes, not generation numbers). Spot-check running executables resolve
   into that closure for the long-running services — a restorer race has
   resurrected stale binaries post-switch before.
2. **Serving, not just active:** poll chat/Core endpoints past `systemctl
   is-active` green until they actually answer, within the generous budgets
   above. The server contract gate reports `passed` with `source_dirty:
   false` — that combination is the only accepted proof the public URL serves
   the reviewed build. The Caddy edge forwards the services' routers verbatim;
   hit one public route per service through the edge.
3. **Dashboard byte-equality:** host-running digest equals the pinned
   `@sha256:` in `modules/dashboard.nix`. An exit-0 command proves nothing;
   compare digests.
4. **Pins:** finite-status shows the artifact pin matched/green on BOTH hosts;
   treated as RED/absent → halt per PRECONDITIONS.
5. **Control-plane census:** rerun `scripts/rollout_preflight.py` — legacy
   vocabulary preserved unless deliberately exercising the new one; no
   unexpected non-terminal rows.
6. **Launch one throwaway fresh agent** end-to-end (create → chat turn →
   reply arrives in-thread). Pin flips affect new launches only, which makes
   this the only check that exercises what you actually shipped. Keep an eye
   out for replies landing home-chat (fallback engaged ≠ lost message; still
   investigate why the route didn't resolve).
7. **The human round-trip:** a real user message through the hosted web
   device and one through another client, answered by a real agent, timed and
   recorded. On 2026-08-11 every machine check passed while hosted chat was
   dark — this row exists because of that day.
8. **Load shape:** host quiet-ish afterward (low load1, PSI ≈ 0). Sustained
   hot loops after a "successful" switch have previously meant a boot loop or
   a thundering restart pile-up.
9. **AFTER evidence + record:** save `finite-status --json` (AFTER), diff
   against BEFORE into the changelog entry: what shipped, when the roll
   finished, compatibility promises still owed, named exceptions. Close the
   wave only when the record is written.

Failure triage quick-links: chats gone dark →
[`chats-appear-missing.md`](chats-appear-missing.md) (read-only first);
hosted device unresponsive →
[`hosted-web-chat-recovery.md`](hosted-web-chat-recovery.md); general box
access → [`break-glass.md`](break-glass.md).

## ROLLBACK

Freeze further rolling first. Classify what broke using VERIFY's artifacts
before choosing a lever; multiple levers below compose in this order.

- **R1 — Services/closures (lat1):** redeploy the previous closure artifact
  (or `nixos-rebuild` to the recorded generation path) per
  [`deploy-core.md`](deploy-core.md)/[`deploy-finitechat-server.md`](deploy-finitechat-server.md).
  Chat delivery is deliberately bidirectional-format-compatible across this
  boundary: reverting the hermes ownership swap costs bounded duplicate turns
  while leases drain (TTL default 45 min; env-overridable per host) — that is
  the designed degradation, not corruption. Durable chat history is untouched.
- **R2 — Core rollback REQUIRES its data migration reversal FIRST:** once
  Migration 0021 has applied, the previous Core generation cannot write
  control requests (old vocabulary violates the restored-era constraints and
  cannot parse the new one). Execute
  `migrations/runtime_lifecycle_reverse_remap.sql` (idempotent, refusal-guarded,
  audit-logged; refuses if non-terminal upgrade-kind requests exist — finish
  or retire them first), then the upgrade-kind
  `runtime_upgrade_rollback_rescue.sql` if rolling past that change too, and
  only THEN start the previous-generation binary. Verify with the census
  (legacy-vocabulary-only view of the world) before declaring R2 done.
- **R3 — Agents/runtime pins:** restore the previous
  `FC_RUNNER_RUNTIME_ARTIFACT_ID` on each host (timer applies without
  restart). Existing agents keep their launch-time digest either way; only
  new launches change.
- **R4 — Dashboard:** revert the digest in `modules/dashboard.nix`, rebuild,
  re-verify byte-equality.
- **R-last resort — Data restore:** only via
  [`postgres-backup-restore.md`](postgres-backup-restore.md) /
  [`litestream-chat-replication.md`](litestream-chat-replication.md) with the
  coordinated empty-target proof per
  [`hosted-web-chat-recovery.md`](hosted-web-chat-recovery.md). Recovery
  authority precedes operator blindness (ADR 0001): user data availability is
  the invariant that survives even a botched wave.

Every rollback reruns the scaled-down VERIFY battery (closure identity,
serving probes, one fresh-agent launch, one human round-trip) and appends the
record to the changelog entry alongside the forward attempt.
