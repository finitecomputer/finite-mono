# Payload Generations: Self-Converging Agent Runtimes

Status: draft for review. Owner: paul. Branch: `codex/payload-generations`.

## Problem

The runtime image fuses the layer that boots (kernel-facing OS, entrypoint) with
the layer we iterate on (Hermes, CLIs, skills, agentd). Because an agent cannot
replace its own body, every runtime change is external surgery — an operator
drives per-agent VM stop/replace/restart from the host. The consequences are
documented in the 2026-08-01 and 2026-08-05 rollout postmortems: stop-timeout
false failures, outside-in probes that misread guest state, host-specific
drift, operator-serialized fleet convergence, and a hard block on opaque
providers (Phala). Separately, managed skills only move when the image moves,
so skill docs drift behind weekly service deploys (the ADR 0002 trade-off did
not hold in practice).

## Decisions already made (owner-confirmed 2026-08-06)

1. **Scope**: full generation mechanism, with the skills channel as the first
   independently mergeable milestone.
2. **Shell boundary**: a new minimal binary, `finite-shell`, containing only
   what is absolutely necessary. Everything else — including `finite-agentd` —
   lives in the payload. No prototype shortcuts; this is the shipping design.
3. **Distribution**: payloads and skills bundles are tarballs referenced by new
   kinds in Core's existing `runtime_artifacts` inventory (sha256 by digest
   now, detached ed25519 signatures on manifests from the start; key rotation
   deferred).
4. **Flip semantics**: talk to Hermes in its own vocabulary (gateway
   restart/replace). Bounded quiesce wait, then proceed — fleet freshness is
   prioritized over never interrupting a turn. Health gate is cheap (no
   inference roundtrip per flip; a fleet-wide inference check would DDoS our
   own inference). Canary channel testing is the real verification; rollback
   is automatic on gate failure and operator-initiated otherwise.
5. **Service directory**: real Core endpoint now, not faked. Endpoints,
   client version expectations, and channel heads are one signed document.

Future idea, explicitly not now: the shell being able to pull an "emergency
repair agent" payload that spends tokens to fix a broken agent in place.

## Architecture

### Layers

```
┌────────────────────────────────────────────────────────┐
│ OCI image ("shell image") — changes rarely             │
│   finite-shell (PID 1) + seed payload tarball          │
├────────────────────────────────────────────────────────┤
│ /data (durable volume)                                 │
│   /data/generations/<version>/   payload generations   │
│   /data/generations/current  →   symlink (atomic flip) │
│   /data/agent/...                existing agent state  │
│   /data/agent/managed-skills/finite/current  skills    │
│   /data/shell/                   shell state + status  │
└────────────────────────────────────────────────────────┘
```

**Payload** (one tested set, one manifest): `finite-agentd`, `finitechat`,
`fsite`, `fbrain`, the Hermes venv, the Hermes finitechat plugin, the `/opt`
scripts' successors, and a skills *seed*. Manifest: version label, per-file
sha256, total digest, minimum shell version, ed25519 signature.

**Skills** ride their own faster channel (see Milestone 1) because content
cadence ≠ binary cadence; the payload's skills seed is only the first-boot /
empty-target fallback.

### finite-shell responsibilities (exhaustive — anything not listed is payload)

- Verify `/data` is mounted and writable; refuse to start otherwise.
- First boot: unpack the in-image seed payload into `/data/generations/` and
  point `current` at it.
- Fetch payload tarballs (HTTPS by digest from the artifact reference), verify
  sha256 + manifest signature (pubkey baked into the shell), unpack to a
  staging generation directory.
- Flip: bounded quiesce request to agentd over a local unix socket
  (`/data/shell/agentd.sock`), graceful agentd shutdown, atomic `current`
  symlink swap, start new generation's agentd.
- Health-gate the flip (agentd ready + bridge ready file + loopback health,
  bounded); on failure, swap back, restart previous generation, mark the
  staged generation bad, report.
- Keep the previous generation on disk; prune older than N−1.
- Supervise exactly one child (the active generation's `finite-agentd`) with
  restart backoff.
- Serve `/healthz` and `/contact` itself (moving them out of the payload's
  Python health server) so a broken payload is still observable and reports
  its generation state. `/healthz` gains `payload_version`,
  `payload_digest`, `shell_version`, `last_flip` fields.
- Poll the service directory for its channel's head at a jittered interval;
  also accept an immediate check/flip instruction from agentd (delivered via
  the Agent Platform Channel as new `agent.payload.*` command families).

The shell has no LLM involvement, no shell-out surface, no config beyond the
Core bootstrap URL, its channel name (in `/data/shell/channel`, set at hatch,
changed only via a typed platform command), and the baked-in release pubkey.

### Core changes

- `runtime_artifacts.kind` gains `payload_bundle` and `skills_bundle`
  alongside `oci_image`. Existing promotion/immutability semantics apply
  unchanged.
- New `release_channels` table: `(channel_name, artifact_kind) → artifact_id`,
  updated by the same operator verbs as promotion. `stable` and `canary` to
  start. Per-agent channel default `stable` recorded at hatch.
- **Service directory endpoint**: `GET /api/core/v1/service-directory` —
  unauthenticated read-only, ed25519-signed document containing: schema
  version, per-service base URLs (core, chat, sites, brain, identity,
  finite-private), per-service `min`/`current` client versions, and channel
  heads for each artifact kind. Endpoints, versions, and channels are one
  advertisement. Nothing per-agent or secret is in it; the agent's own channel
  membership lives on the agent.
- New resolver crate `finite-service-directory` (+ thin Python accessor for
  the Hermes plugin): fetch, verify, cache to
  `/data/agent/service-directory.json`, expose typed lookups. Existing env
  vars become explicit overrides (devfinity/tests), not the primary path.
  Full migration of the ~22 endpoint env vars is follow-up work, not this
  train.

### What is deliberately unchanged

- The Kata production path, rollout wrapper, and lifecycle probe. Production
  adoption of the shell image is a later, separate "one last rollout" (the
  #378 transition pattern) and is out of scope here.
- `check_runtime_image_contract.py` still enforces one canonical image; it is
  updated to know the shell layout, not bypassed.
- User-owned skills and Hermes user config: never touched by any of this.
- Agent identity: every flip and rollback asserts the npub is unchanged.

## Milestones

Each milestone is a mergeable PR train with its own CI coverage. Later
milestones depend on earlier ones; earlier ones ship value alone.

### M1 — Skills channel (drift fix, mergeable alone)

- CI packs `finite-skills/` into a versioned tarball + signed manifest;
  registers it in Core as a `skills_bundle` artifact; operator promotes and
  sets the channel head.
- `finite skills sync` keeps its entire validate/lock/atomic-exchange
  machinery and gains a remote source: fetch by digest, verify, then the
  existing staging + `renameat2`/exchange path. Baked-in `/runtime` bundle
  remains the first-boot seed and offline fallback.
- New `agent.skills.sync` command family on the Agent Platform Channel +
  a periodic check in agentd.
- devfinity: `publish-skills` helper + smoke (`DEVFINITY_SKILLS_SYNC_SMOKE`)
  that publishes a modified bundle and asserts the agent converges and a
  corrupt bundle is refused with the old baseline intact.
- **Spike first**: prove `renameat2(RENAME_EXCHANGE)` works on the Apple
  Container virtiofs `/data` bind. `finite.py` already fails closed
  (`EXDEV/ENOSYS/EOPNOTSUPP → SyncError`) if not; the fallback is symlink-swap
  layout (as generations use) instead of directory exchange.
- Supersedes ADR 0002 → new ADR 0006 (in this train).

### M2 — Service directory

- Core: document schema, signing, endpoint, channel-head inclusion,
  `release_channels` table + operator verbs.
- `finite-service-directory` resolver crate + Python accessor; agentd
  fetch/refresh/cache; M1's skills fetch switches to resolving through it.
- devfinity serves a real directory from local Core; env overrides preserved.

### M3 — finite-shell and generations

- New crate `finite-shell`; Dockerfile restructured: builder stage also emits
  the payload tarball + manifest; final image = shell + seed payload.
  `agent-entrypoint.sh` shrinks to `exec finite-shell`. Payload build script
  shared between CI publish and local devfinity.
- Generation layout, staging, verify, flip, health gate, rollback, prune;
  agentd quiesce protocol over the unix socket (agentd asks Hermes for a
  gateway-style graceful restart; bounded wait, then proceed).
- `/healthz`+`/contact` move into the shell; payload's Python health server
  retires. Runner/devfinity readiness probes unchanged (same port, same
  paths, richer body).
- agentd gains `agent.payload.status|check|stage|flip|rollback|set-channel`
  command families, forwarding to the shell socket. All recorded in agentd's
  existing ledger.

### M4 — Channel pull + acceptance ("the local upgrade simulator")

- Shell polls its channel head and self-converges (stage at any time, flip
  after quiesce).
- `DEVFINITY_PAYLOAD_UPGRADE_SMOKE=1`: hatch → publish payload v2 to canary →
  agent converges unattended → npub identical, chat replies, Brain intact →
  publish a deliberately broken v3 → shell auto-rolls back to v2 and reports
  the bad generation. This test failing-then-passing is the acceptance
  criterion for the whole plan, and is the "local upgrade simulator" the
  2026-07-16 rollout audit asked for.
- Payload version telemetry surfaced end to end: shell `/healthz` → runner →
  Core → `finite-status` fleet view. Fleet policy check: any agent below
  N−1 is flagged for repair (the "gen fence").

## Risks and mitigations

- **virtiofs atomicity** (M1 spike): fail-closed already; symlink fallback
  designed.
- **Two supervisors** (shell → agentd → children): responsibilities are
  disjoint by construction — shell knows only "one child + generations";
  agentd keeps its existing tree. Neither restarts the other's children.
- **Broken generation passes the cheap gate**: canary channel soak is the
  real defense (decision 4); operator rollback via `agent.payload.rollback`
  is the backstop; the crash-loop the gate misses still surfaces through
  shell restart-backoff telemetry.
- **Image contract drift**: `check_runtime_image_contract.py` updated in M3;
  the stale `finitechat/containers/agent/Dockerfile` is deleted in the same
  train.
- **Signing key** is a single ed25519 key for now (CI signs manifests, Core
  signs the directory); rotation and key ceremony are explicitly future work
  before any non-Finite-operated fleet exists.

## Out of scope (tracked, not forgotten)

Production/Kata adoption rollout; floor-version *enforcement* in services
(directory advertises it; nothing refuses yet); bridge-in-agent
(hosted-device single-user mode); full endpoint env-var migration; emergency
repair agent; key rotation; user-hardware distribution of the shell image.

## Follow-ups from the 2026-08-07 one-image rollout postmortem

The four silent-failure classes map onto this architecture as follows: the
port squat (Class 4) and poisoned-metadata/stop-path classes (1/3) cannot
occur during payload flips (no provider surgery), and two hardening changes
landed in this branch — telemetry reports are identity-gated against a pinned
Agent Principal (a squatted port can never wear another agent's name in the
fleet view), and the convergence fence reports `stale` when a runtime's last
report is older than 15 minutes (a dead guest must never keep reading as
converged). Two remain open as designed follow-ups:

- **Gateway-freshness signal (Class 2, wedged-but-connected).** Process
  liveness is not turn liveness. agentd should surface "last inbound message
  vs last gateway activity" so the shell's healthz — and therefore the fleet
  view — carries the wedge detector that today requires an SSH sweep.
- **Provider liveness is telemetry, not provider metadata (Classes 1/3).**
  The four-verb provider contract implicitly trusts the provider to notice
  crashes; containerd's own records lied twice on 2026-08-07. The out-of-band
  liveness definition is telemetry staleness, and any future automated
  "restart presumed-dead compute" loop must fence duplicate writers first
  (the 2026-08-01 lesson).

## Scope, honestly stated (post-review framing, 2026-08-08)

This project is **payload convergence**, not "no more rollouts." Routine
runtime releases stop requiring provider surgery; shell-image, OS, provider,
and disaster-recovery rollouts remain, and the probe/stop-path machinery
stays funded as shell-update infrastructure. Coverage against the 2026-08-07
failure classes:

| Failure class | Effect of this architecture |
|---|---|
| Zombie VM (Class 1) | Not repaired (shell dies with the guest); detected via telemetry staleness → `stale` fence |
| Wedged-but-connected gateway (Class 2) | Not yet detected; gateway-freshness signal is the top follow-up |
| Poisoned containerd record (Class 3) | Avoided during payload releases; still gates provider verbs |
| Port squat (Class 4) | Structurally eliminated for updates; telemetry identity-pinned |

## Gates for the transition rollout (must hold before "one last rollout")

1. **Recovery Set with proven empty-target restore.** The transition is the
   moment of maximum danger — every agent re-imaged; its failure story must
   not be manual forensics. (Also the Aug-1 postmortem's standing follow-up.)
2. **Autonomy ships off.** Transition with poll interval 0 fleet-wide;
   manual channel-driven flips first; enable polling cohort by cohort.
3. **Fault-injection suite in finite-shell tests**: power loss at each
   journal boundary, full disk, directory replay, corrupt state, rollback
   failure (several landed with the hardening batch; complete the set).
4. **Release data-contract policy**: expand/contract migrations with an
   N/N−1 compatibility window (the gen fence defines the window); no
   irreversible remote effects before stable promotion. Checklist, not
   machinery.
5. **Key ceremony** (separate artifact/directory keys, offline root,
   revocation path) before any non-Finite-operated fleet; `key_id` landed
   now so rotation is diagnosable.

Deliberately deferred as premature at current fleet scale: progressive-wave
promotion, automatic head demotion (the fleet view's bad-list visibility +
a human is today's circuit breaker), non-root payload processes, external
user-journey sentinels as product.

## Deferred from the 2026-08-10 re-reviews (tracked, not forgotten)

Accepted as real but deliberately NOT pre-merge at ~50-agent scale (both
reviewers agreed these are follow-ups). Do not let them evaporate:

- **Wedged-but-connected gateway signal (Class 2) — top follow-up.** agentd
  must expose last-inbound vs last-gateway-turn so a wedged-but-"running"
  gateway is visible in healthz → telemetry → fleet view. This is the
  difference between preventing the last incident's *mechanics* and its
  *symptoms*.
- **Digest-based fencing.** The convergence fence compares `version_label`,
  which is not unique; compare artifact-id/digest against the immutable
  channel head / N−1 identities, and bad-list by digest. (Digest is already
  plumbed to Core as evidence.)
- **Flip-gate depth.** Gate parses agentd process states / a loopback probe so
  an immediately-broken Hermes fails the flip (distinct from the deferred
  Class-2 wedge).
- **Control socket becomes a real boundary** only when payloads run non-root
  (USER directive / SO_PEERCRED); today it is groundwork.
- **Corrupt-state recovery mode.** state.json corruption after a commit
  currently degrades to first-boot; enter a diagnosable recovery mode or
  reconstruct from independently retained verified generation metadata.
- **Python accessor verification.** finite_service_directory.py trusts the
  shell-written cache; verify signature/key_id/age on read, or expose
  resolved verified values through agentd.
- Smaller: skills verify→adopt TOCTOU (pass the verified digest into the
  apply step), live re-hash of generations at flip/boot, bad-list Vec cap,
  telemetry registry entry pruning.
- **Transition-readiness (post-merge, gated):** production payload/skills
  publishing workflow (bundle storage + channel advance, not just the seed);
  the three acceptance smokes as required jobs (blocked on a Linux/Docker
  equivalent — the Apple-Container path is developer-machine-only); a Kata
  generations proof.
