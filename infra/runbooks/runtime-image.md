# Building and promoting the agent runtime image

`ghcr.io/finitecomputer/agent-runtime` (mono-owned; the legacy
`finite-agent-runtime` package is frozen with the deployed pins) — the image
the lat1 finite-saas-runner will launch into Kata first and Phala as a fast
follow. Image
definitions map: `infra/images/README.md`. Rung-ladder discipline: see
[README.md](README.md) — no Kata/Phala/Tinfoil promotion without a Docker proof.

> **Runner status:** finite-saas-runner lives on lat1 as a NixOS systemd timer
> (`modules/finite-saas-runner.nix`) and advertises the Kata adapter. Phala is
> a fast follow and must consume the same artifact contract and image digest.

## PRECONDITIONS

- The `finite-lat-2-mono` self-hosted GitHub Actions runner (on lat2, the CI
  runner box post-cutover) is registered against finite-mono — both workflows
  below queue forever without it. Image builds run in CI on lat2; lat1 launches
  only the resulting digest-pinned artifact.
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
template: `infra/hosts/lat1/systemd/runner.env.example`). The pin is:

- **`FC_RUNNER_RUNTIME_ARTIFACT_ID`** (e.g.
  `finite-agent-runtime-canary-20260702-41b0c6d`) — product launches fetch
  the promoted artifact **kind, reference, and state schema from Core**
  using this ID.

So promotion is two steps:

1. Register the new pinned image as an artifact in Core.
   Use Core's service-authenticated runtime-artifact registration endpoint;
   supply the OCI kind, immutable digest reference, source revision, and state
   schema, then promote that artifact for the Kata class.
2. Edit `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on lat1.
   No restart needed: the timer re-invokes the runner with the new env (set
   `FC_RUNNER_DRAIN=true` first if you want in-flight launches to settle).

### 4. Record

Update `compat/matrix.toml` `[field.agent-runtime-image]` `deployed` list
with the new version — hosted agents pin at launch and do NOT auto-update;
existing Runtimes keep their image until replaced, so the list grows until old
compute is retired.

Record the bundled Finite Skills source revision beside the image. New agents
seed that baseline once. Existing agents do not auto-update; users and agents
choose when to run `finite skills sync` against the image's tested bundle.
Core, RMP, and Runner have no desired-skills state.

### 4a. Upgrade an existing Kata Runtime explicitly

Promotion changes only the artifact used by future launches. A normal
`restart` is a same-compute operation for Kata and never adopts the newly
promoted image. Existing compute moves only through an explicit Runtime Upgrade
request bound to one artifact id:

Runtime Upgrade first use is staged across two Core generations. The first
generation ships the new schema/parser with
`FC_CORE_ENABLE_RUNTIME_UPGRADES=false` (the Nix default) and must be live long
enough to be the known-compatible rollback target. Only a later config-only
generation sets the gate to `true`. Never enable first use in the same
generation that first introduces the `upgrade` database value.

Before activating the compatibility generation, this preflight must return no
rows; the migration deliberately fails closed instead of guessing which
already-running provider operation to cancel:

```sql
SELECT agent_runtime_id, count(*)
FROM runtime_control_requests
WHERE status IN ('requested', 'running')
GROUP BY agent_runtime_id
HAVING count(*) > 1;
```

```text
POST /api/core/v1/admin/projects/<project-id>/runtime/upgrade
Content-Type: application/json

{"targetRuntimeArtifactId":"<promoted-artifact-id>"}
```

The admin endpoint uses the same Core-side admin identity headers and service
authorization as the other `/api/core/v1/admin/*` operations. Do not invoke the
Runtime `destroy` endpoint as an upgrade step: destroy intentionally offboards
the Runtime, removes its relay credential, and revokes its Runtime-scoped Finite
Private key.

Core accepts the request only when all of these are true:

- the Runtime was created by Core with the Kata runner class;
- the target is a promoted, non-retired OCI artifact with an immutable
  `@sha256:` reference; and
- the target state schema exactly matches the mounted Runtime state schema.

The leased operation carries the resolved target artifact. The Kata adapter
pulls that digest before downtime, stops but retains the old container, starts a
candidate against the exact same host bind mounted at `/data`, and requires both
generic `/healthz` readiness and the same Agent Principal from `/contact`. Only
then does it rename the old compute to an operation-scoped rollback handle and
move the verified candidate onto the canonical Provider Runtime Handle. A
failure before or after the swap removes the candidate before restarting the
old image, so two containers never write the same `/data` concurrently.

On success, the runner completion records the actual artifact id, state schema,
loopback endpoint, and contact URL in the same Core transaction that completes
the operation. A retry that finds the exact target image on the canonical handle
verifies readiness and returns those facts without replacing compute again.

The runner persists the pre-upgrade Agent Principal before stopping the old
container. If it restarts between either provider-handle rename, it reconciles
the operation-scoped candidate/rollback topology before requiring the canonical
handle and compares the recovered target with that persisted Principal before
deleting the old compute.

Replacement compute does not copy the old image's entire `Config.Env`. It
carries only the runner-owned Runtime contract, explicitly provenance-labeled
user overrides, and credential-shaped secret values. Unowned defaults belong
to the target image and therefore change with the target release. Secret values
remain in the transient mode-0600 env file and never enter process arguments.

Verify after completion:

1. Core's admin Runtime overview reports the target artifact id and `online`.
2. `nerdctl --namespace finite inspect <source-machine-id>` reports the target
   digest and the unchanged `/var/lib/finite-saas-runner/kata/<source-machine-id>:/data`
   bind.
3. `/contact` reports the pre-upgrade Agent Principal, and existing chat,
   attachments, workspace, Sites state, and agentd ledger remain accessible.

This is an ordinary same-volume upgrade, not a Recovery Snapshot or empty-target
restore. A falsely labeled same-schema image can still mutate mounted data in an
incompatible way; the image smoke/promotion gate remains mandatory.

### Tinfoil canary handoff

If a Tinfoil canary is used, it consumes the same published Agent Runtime
digest after the canonical smoke. Do not rebuild or publish a Hermes-only
variant from the legacy handoff scripts. Satellite digest-pin mechanics live
in `infra/tinfoil/README.md`.

## VERIFY

1. Smoke evidence and the publication report name the same monorepo SHA,
   Hermes `0.18.2`, Runtime image digest, CLIs, plugin, and bundled Finite
   Skills source.
2. After promotion: the next runner-launched Kata Runtime comes up ready within
   `FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS` and runs the new image. TODO:
   verify the Core runtime row, `journalctl -u finite-saas-runner` on lat1,
   `nerdctl --namespace finite inspect`, and the Runtime `/healthz` response.
3. Runtime status and the Product Release manifest agree on the image and
   component versions; no mutable branch or second runtime package is used.
4. `compat/matrix.toml` updated.

## ROLLBACK

1. Point `FC_RUNNER_RUNTIME_ARTIFACT_ID` (and the Core artifact record)
   back at the previous version; the 20s timer picks it up.
2. Existing Runtimes are unaffected either way (launch-time pin). For a Kata
   Runtime that adopted the bad image, explicitly request an upgrade to the
   previous promoted, same-schema artifact. Never use destroy as the first leg.
3. Leave the bad tag in GHCR (immutability > tidiness) but note it in
   `compat/matrix.toml` so nobody promotes it again.

### Rolling Core back across Runtime Upgrade first use

Before first use, roll back only to the already-live compatibility generation;
it understands the new schema while the gate remains false. After an Upgrade
row exists, a binary older than the compatibility generation cannot parse that
history. Use this fail-closed rescue only when such an old-binary rollback is
unavoidable:

1. Set `FC_CORE_ENABLE_RUNTIME_UPGRADES=false` and stop the
   `finite-saas-runner.timer` and `finite-saas-runner.service` so no lease can
   move while inspecting provider topology.
2. Query `runtime_control_requests` for `kind = 'upgrade' AND status IN
   ('requested','running')`. For every result, use the compatible runner
   generation to reconcile the operation-scoped Kata candidate/rollback
   handles to one healthy canonical handle, verify `/healthz`, `/contact`, the
   expected image digest, and the single `/data` writer, then let Core record a
   terminal success or failure.
3. Verify that the active-upgrade query returns zero. The rescue script refuses
   to run otherwise; it never guesses that an active provider mutation failed.
4. From the finite-mono checkout, run:

   ```sh
   psql "$FC_CORE_DATABASE_URL" -v ON_ERROR_STOP=1 \
     -f finitecomputer-v2/crates/finite-saas-core/migrations/runtime_upgrade_rollback_rescue.sql
   ```

   The transaction audits every terminal Upgrade row, rewrites only its legacy
   parser-facing kind to `restart`, and restores the old CHECK shape. The
   original target, status, and operation id remain in the audit record.
5. Verify `runtime.upgrade.rollback_rescue` audit rows exist and no
   `kind='upgrade'` row remains. Only then activate the old Core closure. Keep
   Runtime Upgrade disabled until the compatibility generation is restored.

If an explicitly synced skills baseline is bad, replace the Runtime with the
known-good image while preserving `/data`, then explicitly run
`finite skills sync` again. The first slice has no revision-history rollback
command. Do not invent a Core, RMP, or Runner rollback channel.
