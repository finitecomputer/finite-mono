# Deployment changelog

Status: **RECORD — NOT DEPLOYMENT AUTHORITY**

This file holds only what the sources of truth below cannot express: why a
version shipped, when a fleet roll completed, and which compatibility promises
are still live. It replaced the hand-maintained `compat/matrix.toml` on
2026-08-21 (ownership audit O7): nothing ever read that ledger, a whole PR
category existed to keep it in sync, and it had already drifted from the pins
that actually run (it listed `finitechat` at v0.1.5 while `finitechat/v0.1.9`
was tagged, and the dashboard digest comment in Nix lagged it by one version).

Newest first. Never record a secret value.

## Where the facts live

| Surface | Source of truth | How to read it |
|---|---|---|
| Dashboard image | `infra/nixos/modules/dashboard.nix` (`image = …@sha256:…`) | `git log -- infra/nixos/modules/dashboard.nix`; on lat1 `podman inspect finite-saas-dashboard --format '{{.ImageDigest}}'` |
| Agent Runtime image for **new** launches | Core's promoted runtime-artifact record plus `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on each Kata host (lat1, lat3) | `scripts/finite-status` reports the pin per host; promotion per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) |
| Agent Runtime image for **existing** Agents | Core's per-Runtime record — Agents pin at launch and never auto-update | `scripts/finite-status`; serial upgrades per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) §4a |
| CLI and Electron releases (`finitechat`, `fsite`, `fbrain`) | component-scoped release tags and the rolling alias releases (`finitechat-latest`, `fsite-latest`, `fbrain-latest`) | `gh release list --repo finitecomputer/finite-mono`; `git tag -l 'finitechat/*' 'fsite/*' 'fbrain/*'` |
| Server binaries on lat1 (Core, chat, Hosted Device, Sites, Brain, Identity) | the NixOS closure built from `infra/nixos/` at the deployed revision | `readlink -f /run/current-system` on the host; `scripts/finite-status` |
| Finite Private (Tinfoil) | [`runbooks/finite-private-deepseek-production-update.md`](runbooks/finite-private-deepseek-production-update.md) and [`tinfoil/model-inventory.md`](tinfoil/model-inventory.md) | `just finite-private-deepseek-contract` |
| Phala canary Runtime | `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `infra/nixos/modules/finite-saas-phala-runner.nix` | the unit environment is the pin; [`runbooks/phala-confidential-runner.md`](runbooks/phala-confidential-runner.md) |

Pending, not-yet-deployed work is tracked in
[`deployment-queue.md`](deployment-queue.md); once a queue row closes, anything
the sources above cannot carry lands here.

## Standing promises

- The finitechat server keeps accepting every fielded CLI and Electron client
  until a deprecation is announced. The Electron app is a distinct, revocable
  Device on the user's existing Finite Chat account.
- Hosted Agents pin their Runtime image at launch and do **not** auto-update.
  Kata launches the immutable digest through a promoted Core artifact;
  existing Agents are rolled serially through Core's guarded same-volume
  upgrade path, which preserves the Agent Principal and the durable `/data`
  mount.
- Installers point only at the finite-mono rolling alias releases (hard cut
  2026-07-08). Releases installed before the cut (`finitechat` v0.1.0–v0.1.3,
  `fsite` v0.3.1, `fbrain` v0.1.2–v0.1.3) came from the legacy repo URL; no
  live users depend on it.
- The historical `glm-5-2` request alias remains for mixed-version clients;
  `deepseek-v4-flash-0731` is the canonical model label everywhere else.
- Runtime artifact ids promoted before 2026-08-05 (`2026-07-10.2` through
  `2026-07-22.1`) live only in Core's runtime-artifact table.

## Entries

### 2026-08-27 — fbrain `v0.5.0` + Agent Runtime `2026-08-27.2` (same-day fast follow)

- Brain sync went incremental (#699): `fbrain sync`/`open` now reconcile
  against the cached export and pull only sync records instead of downloading
  the full encrypted export — unbricking every brain whose export exceeds the
  10 MB response limit (currently `finitecomputer`). Bumped to 0.5.0
  everywhere; `fbrain/v0.5.0` published and the rolling alias verified
  (installed binary reports 0.5.0).
- Agent Runtime `2026-08-27.2`
  (`…@sha256:7f6d9ab354c40bcddb19ba6ef37769d4a864f87acadf19fad3d22a1d4d6f8368`,
  source `bfe082a1`, carries the 0.5.0 CLI in the agent baseline) registered
  and promoted in Core BEFORE host pins moved; canary then `--roll-all` per
  host — **22/22 lat1, 30/30 lat3 verified** against `.2`.
- Process note: the v0.5.0 tag briefly landed twice on an unmerged bump ref;
  both premature builds were cancelled within a minute and nothing published.
  Guard now required for any release tagging: GitHub `mergeStateStatus`
  CLEAN, PR state MERGED, and the bump read back from main — in that order.

### 2026-08-27 — Platform wave: lifecycle Core `0020–0022`, Agent Runtime `2026-08-27.1`, hermes delivery hardening

- Deployed rev `b9254c81` to both NixOS closures (lat1 rebooted into kernel
  6.18.39 mid-activation — see notes). Agent Runtime `2026-08-27.1`
  (`ghcr.io/finitecomputer/agent-runtime:2026-08-27.1@sha256:c7130042…edf11`,
  source `847fd818`, Hermes 0.20.0, Finite Skills tree `5d7f5618d4…`) was
  registered and promoted in Core, then pinned per host.
- Fleet upgraded via the reviewed prepare/execute wrapper: canary then
  `--roll-all` per host — **22/22 lat1, 30/30 lat3 verified**; the only
  non-target record is the known artifact-less `smoke` row (zero drift
  exceptions).
- Shipped: canonical runtime-control lifecycle vocabulary + forward-only
  offboarding phase machine (Core migrations `0020–0022`, applied cleanly —
  post-migration status census matches pre-wave sums exactly), standing
  readiness health reports, hermes delivery moved onto the Rust sidecar's
  leased inbox (45-min lease TTL default; unresolvable reply threads warn and
  fall back Home instead of being consumed silently), finitechat `/readyz`
  semantic readiness probes (#678) now answering on production.
- CLI releases cut and rolling aliases sha256-verified: `finitechat/v0.2.0`,
  `fsite/v0.5.1`, `fbrain/v0.4.0`.
- Operational notes folded into runbooks: promote into Core BEFORE flipping
  host pins (unregistered pin ⇒ runner cycles fail closed, HTTP 404); a closure
  with a kernel bump reboots during activation (expect the dark window);
  recovery-snapshot gate can transiently cancel a queued job — immediate
  systemd retry succeeds.
- Rollback anchors captured pre-wave: pins
  `finite-agent-runtime-2026-08-20.1`; `lat3-nixos-closure-6fcea1bb…` artifact +
  prior lat1 system generation path; named `pg_dump`
  `finite_core_pre_platform_rollout_2026-08-27T0140Z.dump`; reverse-vocabulary
  rescue gated by `scripts/rollout_preflight.py`.

### 2026-08-20 — Agent Runtime `2026-08-20.1` rolled to the full fleet

- Normalizes the legacy `glm-5-2` model name to `deepseek-v4-flash-0731` at the
  Hermes launcher boundary (PR #585).
- Makes the Hermes adapter durable across restarts: reply routes persist in an
  adapter-owned SQLite (no more replies landing in home-chat after a restart),
  and inbox events ack only after the turn completes, with persisted dedup
  (PRs #589, #591; issues #588, #574).
- Rolled 2026-08-20 to the full fleet via a clean `--roll-all` on both hosts
  (lat3 29/29, lat1 22/22). The Sites Canary 0715 drift record was retired the
  same day through `runtime-offboard-retired-exact` (PR #586); Smoke Studio is
  the only known non-target record left.
- Hermes 0.20.0; source revision `0758088b8a8c78ef69fd72110ead0444336b2b2d`.

### 2026-08-20 — `fbrain` 0.4.0 bumped in source

- llm-wiki papercuts: duplicate-member 400, account-email resolution,
  supervise ticks (PR #581); courtesy email plus a self-describing
  `deliveryStatus` for account-backed invitations (PR #587). Server binary in
  lockstep at 0.4.0.
- The `fbrain/v0.4.0` release tag is the release record; check
  `gh release list` before treating 0.4.0 as fielded.

### 2026-08-19 — Dashboard `2026-08-20.1` pinned

- Stop-button confirmation (#583). Digest in `infra/nixos/modules/dashboard.nix`.
- Dashboard contract at this pin: `finite.computer`, WorkOS + Stripe customer
  mode; Finite Private usage, reset, and reset-time controls; hosted Device
  list; the Electron bridge is capability-gated and otherwise preserves hosted
  chat; revoked Electron devices can re-link without changing the WorkOS user
  identity; Skills visible to signed-in users; Brain navigation disabled while
  the Identity Authority cutover remains deferred.

### 2026-08-19 — Agent Runtime `2026-08-18.1` rolled to the full fleet

- 51 Agents on lat1 + lat3 upgraded and verified; the only non-target records
  were the known drift set (Sites Canary 0715, Sol 2).

### 2026-08-18 — Agent Runtime `2026-08-18.1`

- finitechat single-writer store lease with read-only one-shot CLI paths
  (PR #572).
- agentd bridge-ready deadline raised 30s → 180s (PR #571) — the root cause of
  the 2026-08-18 boot loop on history-heavy rooms.
- Durable-smoke observability plus a finite-private-keyed model smoke
  (PR #570); the durable smoke passed with principal/room continuity across a
  restart.

### 2026-08-18 — finitechat-server Litestream retention

- Snapshot/retention pruning disabled (PR #564). Unbounded replica growth is
  accepted; L0 post-compaction cleanup is still enforced. Chat SQLite remains
  replicated by Litestream; the server listens on loopback `:8788` behind
  `chat.finite.computer`.

### 2026-08-17 — `fbrain` v0.3.0

- The ADR-0046 release: principal grants with provenance; the blessed
  member/admin invite CLI with signed approval artifacts
  (`fbrain approvals list/approve/deny`); chat approval cards anchored to
  messages via `metadata.approve`; cohort Folder invitations via
  folder-scoped plans; the restore drill; SQLite WAL with point lookups and
  denormalized capacity counters; backon-owned retry timing. Server binary in
  lockstep at 0.3.0.

### 2026-08-13 — Finite Private: DeepSeek V4 Flash 0731 scheduler 64/512 → 128/2048

- Measured scheduler promotion on the existing eight-H200 service. The
  checkpoint, MPK, runtime and limiter images, secrets, route, parser, context,
  numerical format, and DP8+EP topology are unchanged.
- Satellite repo `finitecomputer/confidential-kimi-k2-6`, source commit
  `0ef8c6c07dfd56e11d936aba416e24a51e06399a`, release tag
  `v2026-08-13-deepseek-v4-flash-0731-128-2048-1`.
- Candidate-config SHA-256
  `22a3b8030aeb2a47dab8547690cf125880f630d3163bcb713534fb43bffa8907`;
  deployment asset SHA-256
  `83d4d2eb23b052fafecd8a9ec2875ad0aa577842a6ffdd64812914de576463e4`;
  Tinfoil hash asset SHA-256
  `b0322ad6b2bb89f7971002c61868a9b4e53301e6d75a0762849fe06b0f0ee56b`.
- Rollback: commit `e337db3606d67c53387113700362adec7b4dfdf7`, tag
  `v2026-08-05-deepseek-v4-flash-0731-retry-2-3`.
- Procedure and gates: `runbooks/finite-private-deepseek-production-update.md`.

### 2026-08-11 — Agent Runtime `2026-08-11.1` and `fbrain` v0.2.2

- Ships fbrain 0.2.2 (PR #481): the daemon-supervise inotify feedback-loop fix
  behind the fleet-wide ~1.3 cores/VM burn since the Aug 7 roll. Fleet-roll
  target for lat1 + lat3 on 2026-08-11.

### 2026-08-07 — Agent Runtime `2026-08-07.1` and `2026-08-07.2`

- `2026-08-07.1` brought Hermes 0.20.0 (GitHub-tarball channel) and fbrain
  0.2.1. `2026-08-07.2` restored the manifest-driven plugin data files the 0.20
  wheel build drops (PR #460).

### 2026-08-06 — `fbrain` v0.2.1

- Keeps new non-Markdown asset bytes out of Folder Objects, syncs Markdown
  Asset Source Notes under `raw/`, preserves legacy inline Asset ciphertext as
  unsupported, and retains incremental-write compatibility with older servers
  through a bounded-bootstrap fallback.
- Adds email-invitation delivery reporting, a non-rewriting V21 migration that
  permits parallel Brain and Folder invitations, and an opt-in
  disposable-stack rate-limit override; production defaults unchanged.
  Product Client readable-asset portability remains deferred. Ships the
  finite-brain server binary.

### 2026-08-05 — Agent Runtime `2026-08-05.1`

- The lat3 convergence target.

### 2026-08-01 — `fsite` v0.5.0

- Typed mailbox/NIP-05/npub targets, the mailbox-principal reconciliation and
  persistent-sharing flows; also ships `finitesitesd-linux` for the server
  deploy.

### 2026-07-23 — `finitechat` v0.1.5

- Bounded microphone recording through the existing attachment path (CLI and
  Electron). CLI/server protocol unchanged.

### 2026-07-22 — `finitechat` v0.1.4 (first Electron release)

- Developer ID-signed and Apple-notarized direct-download app for Apple
  Silicon (`finitechat-electron-macos-aarch64.zip` under the `finitechat-latest`
  alias), with revoked-device recovery. CLI/server protocol unchanged.
