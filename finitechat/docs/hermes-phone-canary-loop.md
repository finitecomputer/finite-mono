# Fresh Hermes Phone Canary Loop

Status: Finite Chat local-phone and remote-Docker canary runbook. The hosted
runtime provider ladder now lives in
`../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`.

## Problem Statement

The core quality loop is:

> Paul asks for a fresh Hermes-backed Finite Chat agent, receives an invite URL
> opens the production iOS app on his physical phone, joins the room, and has a
> real multi-turn conversation with Hermes.

This must be a named product gate, not an improvised operator sequence. Local
phone, remote Docker, and hosted provider canaries should use the same
promotion rule: do not hand a fresh invite to a human until the lower layer has
already proven agent-side admission and runtime readiness with
machine-readable evidence.

## What Went Wrong

- We treated multiple different proofs as interchangeable. Adapter echo tests,
  CLI admission smokes, simulator UI tests, Docker restore tests, and human
  physical-phone tests answer different questions.
- `scripts/hermes-real-gateway-demo.sh` is a low-level manual runner, not a
  product canary. It can expose an invite before the sidecar and Hermes gateway
  have proven that they can admit a join.
- We let a server compatibility failure turn into a new test shape. The product
  canary server is `https://chat.finite.computer` for local phone, remote
  Docker, and hosted provider canaries. If production is behind the app/server
  contract, deploy the server; do not replace the release gate with a Mac LAN
  server.
- Physical-phone reachability was not modeled as a first-class constraint.
  `127.0.0.1` is useful for a simulator and impossible for the phone because
  it points at the phone itself. A phone product canary must use the hosted
  server. Mac LAN URLs, loopback, and tunnels are lower-level diagnostics only.
- Script discovery depended on relative worktree layout and whichever Hermes
  checkout/profile happened to be found. A canary must record the Hermes
  package/version, plugin path, plugin hash, finitechat commit, and dirty flag.
- Sidecar and plugin compatibility was assumed from startup. The current
  sidecar contract is `/healthz`, `/readyz`, `/v1/hermes/inbound`, and
  `/v1/hermes/{action}`. A run that talks to any other bridge route is a
  provenance/version failure, not a flaky chat failure.
- Invite delivery was optimized for the first terminal start. That is the wrong
  UX. The running agent should expose the stable invite after launch, so the
  dashboard or operator can fetch it whenever the human is ready.
- We used logs and screenshots as the main oracle. They are useful for
  debugging, but every gate should write JSON with the inputs, versions,
  process ids, URLs, room id, invite id, and pass/fail reason.
- We let active product state and fresh canary state blur together. A fresh
  canary must use a new agent state root or a deliberate restored state root,
  and must say which one happened.
- Git hygiene got harder because runtime work, app work, docs, and provider
  work were spread across worktrees. Experimental work should stay in a
  task-specific worktree and publish only source changes that passed the gate.

## Non-Negotiable Rules

- No echo handler counts as real Hermes.
- No CLI join counts as the physical-phone product proof. CLI join is only an
  owner-side admission preflight.
- No phone, remote Docker, or hosted provider product canary may replace
  `https://chat.finite.computer` with a local, LAN, loopback, or tunnel server.
  Local servers are lower-level diagnostics, not product acceptance.
- No human invite is handed out before the sidecar is ready, the gateway is
  alive, the current plugin provenance is recorded, and a throwaway admission
  probe has passed.
- No hosted provider deployment starts until local phone and remote Docker
  evidence are green for the same source/image lineage.
- Phala/provider-specific acceptance belongs to
  `../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`.
- No generated state, logs, secrets, SQLite files, or `target/` reports are
  committed.
- No work happens in a dirty default checkout when a task-specific worktree is
  available.

## Evidence Schema

Every canary layer should write a report with this shape, even if some fields
are null for that layer:

```json
{
  "status": "passed",
  "layer": "local-phone | remote-docker",
  "source": {
    "repo": "finitecomputer/finitechat",
    "branch": "codex/hermes-sidecar-hardening",
    "commit": "git sha",
    "dirty": false
  },
  "runtime": {
    "finitechat_version": "...",
    "hermes_agent_version": "0.17.0",
    "plugin_name": "finitechat",
    "plugin_hash": "...",
    "image_ref": null,
    "image_digest": null
  },
  "server": {
    "url": "https://chat.finite.computer",
    "phone_reachable": true
  },
  "agent": {
    "state_root": "...",
    "restored": false,
    "npub": "npub...",
    "device_id": "agent-short-id",
    "sidecar_url": "http://127.0.0.1:...",
    "healthz": {"ready": true},
    "readyz": {"ready": true}
  },
  "invite": {
    "room_id": "room-...",
    "invite_id": "invite-...",
    "url": "finite://join?..."
  },
  "admission_probe": {
    "state": "joined",
    "elapsed_ms": 0
  },
  "phone": {
    "device_name": "Paulphone Air",
    "installed_bundle_id": "computer.finite.finitechat",
    "installed": true,
    "human_chat_event_ids": ["..."]
  },
  "steps": []
}
```

If a gate fails, the report should still exist with `status: "failed"` and a
single normalized `failure` string plus log paths. A missing report is itself a
failed gate.

## Layer 1: Local Mac To Physical Phone

Purpose: prove product behavior with the fastest feedback loop and the fewest
deployment variables.

Recommended defaults:

- Finite Chat server: `https://chat.finite.computer`.
- Hermes: `hermes-agent==0.17.0` unless this doc is updated with a newer
  supported version.
- Agent state: fresh timestamped root under
  `target/hermes-phone-canary/local/<run-id>`.
- Phone app: normal `computer.finite.finitechat` build installed on the
  physical device. No launch automation flags for join or send.

Required preflight:

1. `git status --short --branch` is recorded in the report.
2. `cargo build -p finitechat-cli -p finitechat-rmp`
   succeeds, or the report records `--skip-build` with binary provenance.
3. The phone server URL is `https://chat.finite.computer`.
4. The app installs on the target phone.
5. The sidecar answers `/healthz` and `/readyz`.
6. The Hermes gateway process is still alive after loading the plugin.
7. The gateway log records the current `finitechat` plugin, not a stale
   or built-in bridge.
8. A throwaway CLI admission probe joins with the current invite.

Only after those preflights pass should the harness print the human handoff:

```text
Invite URL: finite://join?...
Report: target/hermes-phone-canary/local/<run-id>/report.json
```

Human acceptance:

- Paul joins from the phone app without CLI flags.
- The app reaches the room composer.
- Paul sends at least two messages.
- Each local send appears optimistically before server acceptance.
- The app shows a visible working/thinking marker while Hermes is handling a
  turn.
- Hermes replies to both messages with real model output.
- The report is updated with observed phone chat event ids or explicit
  operator notes that the human proof passed.

Existing lower-level checks that should stay green before the phone handoff:

```sh
cargo run -q -p finitechat-rmp -- test ios-simulator --json
scripts/hermes-adapter-regression-report.py
scripts/hermes-sidecar-smoke.sh
scripts/hermes-real-gateway-admission-smoke.py
```

The local one-command phone canary is:

```sh
scripts/hermes-phone-canary.py --install-phone-app --keep-running
```

On success it writes `target/hermes-phone-canary/local/<run-id>/report.json`,
prints the invite URL, and leaves a `stop.sh` script in the same run directory.
It must fail before printing a human invite if the hosted server, app install,
sidecar, gateway, throwaway admission probe, or real Hermes model-response
smoke fails.

`scripts/hermes-real-gateway-admission-smoke.py` is the closest existing local
preflight because it starts `finitechat hermes serve`, runs
`hermes gateway run --replace`, and proves invite admission through a normal
throwaway client. It is still not the phone canary because it uses a CLI user
and local loopback server by default.

## Layer 2: Remote Docker Runtime

Purpose: prove the real Linux runtime shape before provider deployment. This
should run on finite-lat-2 or another x86 Docker host, ideally through the
self-hosted GitHub runner path so published packages are tied to the same proof.

Required preflight:

1. The local phone report for the source branch is green.
2. The Docker image is built from `containers/agent/Dockerfile`.
3. The image starts the same entrypoint intended for hosted providers:
   `/opt/agent-entrypoint.sh` -> `/opt/run_hermes_gateway.sh`.
4. The runtime reports `FINITE_AGENT_RUNTIME real_hermes_gateway=true`.
5. The container health endpoint reports the agent npub.
6. The Docker smoke proves gateway admission before backup and after restore.

Lower-level restore smoke:

```sh
scripts/hermes-sidecar-docker-smoke.sh
```

Human-handoff remote Docker canary:

```sh
scripts/hermes-remote-docker-canary.py --keep-running
```

By default this uses `ssh://finite-lat-2`, builds the real runtime image on
that remote Docker daemon, starts the container against
`https://chat.finite.computer`, proves invite admission and a real Hermes
model reply, stops the container so the entrypoint writes a restic backup,
wipes the agent volume, restores into a fresh volume, proves the same user can
still chat, proves a fresh user can still join, and only then prints a human
invite URL. The restored container is left running only with
`--keep-running`; otherwise the script cleans up the remote container and
volumes after writing the report.

For a remote Docker canary, the wrapper should produce the same report shape as
the local phone canary plus image metadata:

- image id and digest;
- runtime env names used;
- server URL used by the agent;
- restic backend, repository, tag, and snapshot id when restore is enabled;
- before/after admission probe results;
- optional human phone chat event ids if Paul tests against this remote agent.

Promotion rule: a remote Docker run can hand an invite to Paul only if the
same container has already passed the admission probe and the report includes
the exact image id/digest. If the image is rebuilt after the probe, the probe
must be rerun.

## Layer 3: Hosted Runtime Provider

Provider-specific runtime promotion belongs to
`../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`. That v2 matrix owns
the local Docker, remote Docker, and Phala acceptance rules for the real hosted
agent image. Finite Chat should feed that matrix a proven app/protocol/plugin
commit, not maintain a separate provider ladder here.

## Invite API Requirement

The runtime should expose a small local/admin API that returns the stable invite
after launch. Hitting it twice should keep the same invite and room.

Minimum response:

```json
{
  "room_id": "room-...",
  "invite_id": "invite-...",
  "url": "finite://join?...",
  "agent_npub": "npub..."
}
```

This is an operator/dashboard surface. It is not a replacement for admission
authorization. The endpoint must never expose private keys, provider keys,
restic passwords, or plaintext chat contents.

## Git Hygiene

- Work in this worktree for Finite Chat changes:
  `finite-chat-darkmatter-worktrees/hermes-sidecar-hardening`.
- Use a separate `finitecomputer-v2` worktree for hosted runtime/deploy changes.
- Keep `.state/`, `target/`, SQLite stores, app containers, and downloaded CI
  artifacts out of git.
- Before starting a new experiment, record:

```sh
git status --short --branch
git worktree list
```

- Before dispatching CI or publishing images, use the branch publication gate
  so source changes are explicit and generated/sensitive paths are blocked:

```sh
scripts/hermes-branch-publication-readiness.py \
  --branch codex/hermes-sidecar-hardening
```

## Implementation Backlog

1. Add one command for the local phone canary, preferably under
   `finitechat-rmp`, that implements the Layer 1 preflight and evidence schema.
2. Make `scripts/hermes-real-gateway-demo.sh` fail closed or clearly label it
   as a manual local runner so it is not reused as the phone product gate.
3. Add the runtime invite API to the local runner and Docker/provider runtime.
4. Add a remote Docker wrapper that can run the real image on finite-lat-2,
   fetch the invite after the admission probe, and write the same report
   schema.
5. Teach the hardening audit to require the local-phone and remote-Docker
   reports before accepting a hosted provider canary handoff.
6. Keep the app-side canary assertions in Rust/RMP as much as possible:
   optimistic send projection, working/thinking markers, room admission state,
   and multi-turn transcript projection should be tested through the same core
   state used by CLI and iOS.
