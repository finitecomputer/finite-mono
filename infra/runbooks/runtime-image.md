# Building and promoting the agent runtime image

`ghcr.io/finitecomputer/agent-runtime` (mono-owned; the legacy
`finite-agent-runtime` package is frozen with the deployed pins) — the image
the lat1 finite-saas-runner will launch into Kata first and Phala as a fast
follow. Image
definitions map: `infra/images/README.md`. Rung-ladder discipline: see
[README.md](README.md) — no Kata/Phala/Tinfoil promotion without a Docker proof.

> **Runner status (post 2026-07-09 cutover):** the finite-saas-runner now
> lives on lat1 as a NixOS systemd timer (`modules/finite-saas-runner.nix`)
> but is **DORMANT** — the `phala` CLI it shells out to is not nix-packaged
> yet, and a Phala-Cloud-API / "enclavia" runner is being explored as the
> successor. Building/proving images below is independent of that; promotion
> to a live runner (step 3) waits on the runner being packaged.

## PRECONDITIONS

- The `finite-lat-2-mono` self-hosted GitHub Actions runner (on lat2, the CI
  runner box post-cutover) is registered against finite-mono — both workflows
  below queue forever without it. (Image BUILDS run in CI on lat2; the runtime
  runner that LAUNCHES the image is the dormant one on lat1.)
- The tree state you are building is on the ref you dispatch (the single
  checkout SHA pins finitechat + finite-sites + finite-brain + finite-skills
  together; that is the whole point of the mono adaptation — see the header in
  `.github/workflows/runtime-image.yml`).

## STEPS

### 1. Prove the source revision

Dispatch **Hermes runtime smoke**
(`.github/workflows/hermes-runtime-smoke.yml`) on the revision to promote. It
is test-only and builds the same canonical monorepo Agent Runtime Dockerfile as
the publication path; it cannot publish a second Hermes-only image. Run the
Docker message/restart lane and the durable-home lane as appropriate for the
cohort.

### 2. Build and publish the one runtime image

1. On that same successful revision, dispatch **Agent Runtime Image**
   (`.github/workflows/runtime-image.yml`) with
   `version=<date-based, e.g. 2026-07-08.1>`. Hermes is repository-pinned to
   `0.18.2`, the same version exercised by every smoke lane. For future
   upgrades, move the reviewed pin in the image and all smoke lanes together;
   do not add a dispatch-time override.
2. The publication workflow builds via
   `finitecomputer-v2/scripts/build_runtime_image.py` from one staged
   finite-mono checkout and root Cargo lockfile, embeds the Finite Skills
   baseline, validates the exact image, pushes `:$VERSION` +
   `:sha-<sha>`, uploads `runtime-image-report.json`, and prints the pinned
   `ghcr.io/finitecomputer/agent-runtime:<version>@sha256:...` in
   the summary. Copy that pinned ref — it is the only thing you promote.

**Recovery TODO:** the current Docker Restic smoke is not product Recovery
Readiness. It is opt-in, backs up `/data/agent` but not `/data/workspace`, uses
an operator-supplied password, and is not enabled by the v2 Runner. It proves a
component mechanism only. Restic suitability, provider-independent Recovery
Snapshots, key backup, and empty-target restore remain open post-MVP questions;
they do not block the first trusted-cohort SaaS slice.

### 3. Promote to the lat1 runner

The runner does **not** read an image tag directly. On NixOS lat1 the env is
`/etc/finite/runner.env` (secrets bootstrap — `infra/nixos/README.md`;
template of the 22 vars: `infra/hosts/lat1/systemd/runner.env.example`). The
pin is:

(Reminder: the lat1 runner is DORMANT until the phala/enclavia runner is
packaged — this step is the promote procedure for when it goes live.)

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
2. Edit `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on lat1.
   No restart needed: the timer re-invokes the runner with the new env (set
   `FC_RUNNER_DRAIN=true` first if you want in-flight launches to settle).

### 4. Record

Update `compat/matrix.toml` `[field.agent-runtime-image]` `deployed` list
with the new version — hosted agents pin at launch and do NOT auto-update;
old CVMs keep their image until relaunched, so the list grows until old
CVMs are retired.

Record the bundled Finite Skills source revision beside the image. New agents
seed that baseline once. Existing agents do not auto-update; users and agents
choose when to run `finite skills sync` against the image's tested bundle.
Core, RMP, and Runner have no desired-skills state.

### Tinfoil canary handoff

If a Tinfoil canary is used, it consumes the same published Agent Runtime
digest after the canonical smoke. Do not rebuild or publish a Hermes-only
variant from the legacy handoff scripts. Satellite digest-pin mechanics live
in `infra/tinfoil/README.md`.

## VERIFY

1. Smoke evidence and the publication report name the same monorepo SHA,
   Hermes `0.18.2`, Runtime image digest, CLIs, plugin, and bundled Finite
   Skills source.
2. After promotion: the next runner-launched CVM comes up ready within
   `FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS` and runs the new image. TODO:
   record the concrete check (runner logs via
   `journalctl -u finite-saas-runner` on lat1, or a Core/dashboard artifact
   view) at the first mono promote.
3. Runtime status and the Product Release manifest agree on the image and
   component versions; no mutable branch or second runtime package is used.
4. `compat/matrix.toml` updated.

## ROLLBACK

1. Point `FC_RUNNER_RUNTIME_ARTIFACT_ID` (and the Core artifact record)
   back at the previous version; the 20s timer picks it up.
2. Existing CVMs are unaffected either way (launch-time pin). Relaunch any
   CVM that was launched from the bad image.
3. Leave the bad tag in GHCR (immutability > tidiness) but note it in
   `compat/matrix.toml` so nobody promotes it again.

If an explicitly synced skills baseline is bad, replace the Runtime with the
known-good image while preserving `/data`, then explicitly run
`finite skills sync` again. The first slice has no revision-history rollback
command. Do not invent a Core, RMP, or Runner rollback channel.
