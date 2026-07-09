# Building and promoting the agent runtime image

`ghcr.io/finitecomputer/agent-runtime` (mono-owned; the legacy
`finite-agent-runtime` package is frozen with the deployed pins) — the image the lat1
finite-saas-runner launches into Phala hosted-agent CVMs. Image definitions
map: `infra/images/README.md`. Rung-ladder discipline: see
[README.md](README.md) — no Phala/Tinfoil promotion without a Docker proof.

## PRECONDITIONS

- A lat2 self-hosted runner is registered against finite-mono
  (`infra/hosts/lat2/runners.md` cutover checklist) — both workflows below
  queue forever without it.
- The tree state you are building is on the ref you dispatch (the single
  checkout SHA pins finitechat + finite-sites + finite-brain together; that
  is the whole point of the mono adaptation — see the header comment in
  `.github/workflows/runtime-image.yml`).

## STEPS

### 1. Build

1. Dispatch **Agent Runtime Image** (`.github/workflows/runtime-image.yml`):
   `version=<date-based, e.g. 2026-07-08.1>`, `hermes_agent_version`
   (default 0.18.0). Runs on `[self-hosted, finite-lat-2]`.
2. The workflow builds via
   `finitecomputer-v2/scripts/build_runtime_image.py` (staging finitechat,
   finite-sites, finite-brain from this tree), pushes `:$VERSION` +
   `:sha-<sha>`, uploads `runtime-image-report.json`, and prints the pinned
   `ghcr.io/finitecomputer/agent-runtime:<version>@sha256:...` in
   the summary. Copy that pinned ref — it is the only thing you promote.

### 2. Prove (rung-ladder, before any promote)

Run **Hermes runtime smoke** (`.github/workflows/hermes-runtime-smoke.yml`)
proof lanes: `docker_smoke` (full Docker runtime smoke with encrypted
backup/restore; publish requires `restic_backend=s3`) and/or
`phala_durable_smoke` (durable /home/node lane). Publication of the proven
image is gated on the smoke report by
`finitechat/scripts/hermes-publish-proven-image.py` — the workflow will not
publish an unproven image.

Note: that workflow builds and proves `hermes-runtime` (the
hermes agent image), not `finite-agent-runtime` itself. TODO: the runtime
image currently has no equivalent automated smoke of its own before Phala
promote — until it does, at minimum launch one canary CVM from the new
pinned digest and exercise it before promoting to product traffic.

### 3. Promote to the lat1 runner

The runner does **not** read an image tag directly. In
`/etc/finite-computer/runner.env` on lat1
(template: `infra/hosts/lat1/systemd/runner.env.example`), the pin is:

- **`FC_RUNNER_RUNTIME_ARTIFACT_ID`** (e.g.
  `finite-agent-runtime-canary-20260702-41b0c6d`) — product launches fetch
  the promoted artifact **kind, reference, and state schema from Core**
  using this ID.

So promotion is two steps:

1. Register the new pinned image as an artifact in Core.
   TODO: the exact registration mechanism (core admin API endpoint?
   dashboard?) is not documented in infra — capture it at the first
   mono-built runtime promote. (For reference, lat2's dormant runner env
   pins the reference directly via `FC_RUNNER_RUNTIME_ARTIFACT_REFERENCE` —
   lat1's live 22-var env does not have that var.)
2. Edit `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite-computer/runner.env`
   on lat1. No restart needed: the 20s timer re-invokes the runner with the
   new env (set `FC_RUNNER_DRAIN=true` first if you want in-flight launches
   to settle).

### 4. Record

Update `compat/matrix.toml` `[field.agent-runtime-image]` `deployed` list
with the new version — hosted agents pin at launch and do NOT auto-update;
old CVMs keep their image until relaunched, so the list grows until old
CVMs are retired.

### Tinfoil canary handoff

For the Tinfoil lane, the docker-smoke publish path generates the canary
artifacts (`finitechat/scripts/hermes-tinfoil-canary-artifacts.py`,
uploaded in the workflow's artifact bundle) targeting
`finitecomputer/tinfoil-agent-runtime-canary`. Satellite-repo mechanics and
the digest-pin update flow: `infra/tinfoil/README.md`.

## VERIFY

1. Workflow summary shows the pinned digest; `runtime-image-report.json`
   artifact is present.
2. After promotion: the next runner-launched CVM comes up ready within
   `FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS` and runs the new image. TODO:
   record the concrete check (runner logs via
   `journalctl -u finite-saas-runner` on lat1, or a Core/dashboard artifact
   view) at the first mono promote.
3. `compat/matrix.toml` updated.

## ROLLBACK

1. Point `FC_RUNNER_RUNTIME_ARTIFACT_ID` (and the Core artifact record)
   back at the previous version; the 20s timer picks it up.
2. Existing CVMs are unaffected either way (launch-time pin). Relaunch any
   CVM that was launched from the bad image.
3. Leave the bad tag in GHCR (immutability > tidiness) but note it in
   `compat/matrix.toml` so nobody promotes it again.
