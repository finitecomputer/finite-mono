# Finite Private limiter: switch ownership to mono

This is the planned downtime procedure for the first Finite Private enclave
release whose limiter image is built from `finite-mono`. It also advances the
model container from upstream GLM 5.2 v0.0.14 to v0.0.17. Preparation and image
proof may happen ahead of time. **Creating the satellite release, relaunching,
or switching production requires fresh explicit approval.**

The ownership switch does not migrate usage state. Core remains the system of
record for grants, reservations, settlements, and API keys; the same sealed
service key connects the replacement limiter to the same Core API.

## Known source and image boundary

| Item | Current production | Target |
|---|---|---|
| Limiter source | legacy `finitecomputer`, commit `81f40e8` | this repo, `cafe85246bce88201c23a46ec7b33c8e28cc25e4` |
| Limiter image | `ghcr.io/finitecomputer/finite-private-limiter:2026-07-02.glm52.health.1@sha256:f977b238439ff4caa3f416bf1ec8f16ed383640d7417262d26ed4388c8624d5c` | `ghcr.io/finitecomputer/private-limiter:2026-07-21.1@sha256:5d57ecf462fcb105eae2160dd01493efd825532fb61ee286098bdc1b485ec84b` |
| GLM image | upstream v0.0.14, `sha256:8cc690cf5b1c26b0bc14894a7ca27890386b536930b69172678560220572648b` | upstream v0.0.17 (`84b2e80`), `sha256:0a73ccd09e52d63ef101ac2911e54760b58ca6e0596cadfd219e096d54b1a396` |
| CVM | `0.10.4` | `0.10.8` |
| Satellite | `finitecomputer/confidential-kimi-k2-6` | same repo and outer deployment |

Expected interruption is about 35 minutes based on prior 2,064–2,098 second
multi-GPU model starts. Announce a wider window and do not claim zero downtime.

## Preconditions

1. The mono change is merged to `main`, the exact 40-character source SHA is
   recorded, and all workspace/limiter tests pass at that SHA.
2. `Service Images` has built `private-limiter` from that exact SHA. Record the
   immutable tag and digest; prove the image at the Docker rung before Tinfoil.
3. Compare the mono limiter source to legacy commit `81f40e8`. The only
   intended divergence is mono build/package wiring; retain fresh internal
   accounting request IDs, streaming `[DONE]` settlement, timeout settlement,
   settle retry, `/live`, `/health`, `/ready`, and `/metrics`. Keep the watchdog
   disabled for this rollout.
4. Record that no equivalent non-production eight-GPU target exists for a
   representative `--max-num-seqs 32` load proof. The bounded 32-call check is
   therefore an explicit, quota-consuming test inside the approved downtime
   window below, against a dedicated synthetic canary grant. This is not a
   reason to waive the latency gate.
5. Confirm the mono candidate still pins limiter digest
   `sha256:5d57ecf462fcb105eae2160dd01493efd825532fb61ee286098bdc1b485ec84b`
   and review a local diff against both the currently deployed satellite config
   and upstream v0.0.17. Do not create or update a satellite branch before the
   approval boundary below.
6. Confirm the topology remains `glm-5-2:8001` (private) →
   `finite-private-limiter:8002` → public shim; `shim.upstream-port` is 8002;
   only the limiter joins the `core-api` allowlist network.
7. Confirm sealed secrets are present by name only:
   `FINITE_USAGE_API_SERVICE_KEY`, `VLLM_INTERNAL_API_KEY`, and `VLLM_API_KEY`.
   Confirm the two vLLM-facing key values still agree through the secret
   management surface without exposing them in logs.
8. Capture the current container/deployment ID, current measured release tag,
   current config, and current `tinfoil-deployment.json`/`tinfoil.hash`. Name
   that measured tag as the rollback target before proceeding.
9. Schedule and communicate the downtime window. Verify an operator can access
   Tinfoil, Core read-only accounting evidence, and the canary secret file.

## Prepare the measured target (approval boundary)

Do not perform this section until approval includes creating the new satellite
release.

1. Create a satellite branch from its current production commit, copy the exact
   reviewed mono candidate, and verify its diff matches the local preflight
   evidence.
2. Merge the reviewed satellite config and create a new, unique release tag.
3. Wait for the Tinfoil release workflow to produce
   `tinfoil-deployment.json` and `tinfoil.hash` for that exact tag.
4. Verify the measured artifacts reference both expected image digests, CVM
   0.10.8, the existing model revision/MPK, port 8002 as the shim target, and no
   placeholder digest. A Git tag without measured artifacts is not deployable.
5. Record target tag, target deployment hash, mono source SHA, limiter digest,
   model digest, operator, and scheduled window in the change record.

## Switch during the approved window

1. Immediately before downtime, run the read-only current gate:

   ```bash
   infra/runbooks/finite-private-ops.sh gate
   ```

   Also capture a known settled Core canary reservation as the before-state.
2. Set the relaunch guard to the exact approved measured tag and relaunch:

   ```bash
   export FINITE_PRIVATE_RELAUNCH_APPROVED='<approved-measured-tag>'
   infra/runbooks/finite-private-ops.sh relaunch '<approved-measured-tag>'
   ```

3. Run `wait-ready`. Preserve non-200 response bodies. During model load,
   public 502 or upstream-pending readiness is expected; a limiter `/live` 200
   proves the shim has reached the limiter, while `/health` 200 proves both
   vLLM and Core are ready.
4. Once ready, run the scripted protocol and bounded-load canaries. The load
   command enforces p99 first byte below 90 seconds, leaving 30 seconds of
   headroom under the limiter's 120-second first-byte timeout; override that
   threshold only in the reviewed change record.

   ```bash
   infra/runbooks/finite-private-ops.sh gate
   infra/runbooks/finite-private-ops.sh stream-canary
   infra/runbooks/finite-private-ops.sh responses-canary
   infra/runbooks/finite-private-ops.sh repeated-id-canary
   infra/runbooks/finite-private-ops.sh load-canary
   ```

   Preserve all command output. Compare the dedicated canary grant's Core
   reservation rows before and after these commands and confirm:

   - `/live`, `/health`, and `/ready` return 200;
   - `/live` reports `defaultModel=glm-5-2`, the intended dependency URLs,
     required secret presence, and the intended timeout budgets;
   - an invalid Finite key returns 401 before inference;
   - valid chat and responses calls succeed through port 8002;
   - the two calls printed by `repeated-id-canary` create two distinct Core
     reservation IDs despite one caller `x-request-id`;
   - the successful streaming reservation settles promptly after `[DONE]`;
   - Core has no canary reservation stuck in `reserved`; and
   - user-visible denial copy and reset timestamps still come from Core (the
     exhausted-grant denial path remains a synthetic integration test unless
     the change record separately authorizes exhausting a dedicated canary).
5. Observe deep readiness, error rate, first-byte latency, settlement latency,
   and reserved-row age through the initial canary period. End the downtime
   notice only after accounting evidence and inference both pass.

## Abort and rollback

Abort before relaunch if the target is missing measured assets, a digest does
not match, the placeholder remains, the secret names/topology changed, or the
first-byte gate fails.

After relaunch, roll back immediately if the target cannot reach deep readiness
within the agreed startup budget, invalid keys reach vLLM, valid canaries fail,
streaming reservations remain stuck, Core reserve/settle is unhealthy, or
load-canary p99 first byte is 90 seconds or higher. Do not bypass the limiter to restore
service unless a separate break-glass decision explicitly accepts unmetered
access.

Rollback is one relaunch to the exact prior known-good measured tag captured in
preflight:

```bash
export FINITE_PRIVATE_RELAUNCH_APPROVED='<prior-known-good-measured-tag>'
infra/runbooks/finite-private-ops.sh relaunch '<prior-known-good-measured-tag>'
infra/runbooks/finite-private-ops.sh wait-ready
infra/runbooks/finite-private-ops.sh gate
```

The rollback reuses the unchanged Core state and sealed secret names. Expect a
second model-load downtime period. After rollback, verify a fresh reservation
settles and inspect any target-era `reserved` rows before closing the incident;
prefer user-favorable correction if a failed target attempt left stale
estimates.
