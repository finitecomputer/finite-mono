# Finite Private: stable route and container-name migration

Status: design and preparation only. No DNS, custom-domain, container, or
production state mutation is authorized by this file.

## Decision

The stable service identity is **Finite Private**, not a model name. Use:

- container name: `finite-private`;
- preferred custom route: `inference.finite.computer`;
- current model: `deepseek-v4-flash-0731`;
- mixed-version model alias: `glm-5-2`.

Do not rename the container to `deepseek-*`. Finite Private has already served
Kimi, GLM, and DeepSeek, and a model-specific infrastructure name recreates the
same migration debt on the next model change.

## Current readers and writers

The generated `kimi-k2-6` hostname is read by Core-created Runtime specs, both
Kata Runner hosts, the Runtime/Hermes environment, local chained-limiter tools,
tests, and operator canaries. The Tinfoil container is the writer of the route;
the public shim forwards to the limiter, which reserves and settles usage with
Core before and after vLLM inference.

Tinfoil has no in-place container rename operation. A new name requires a new
container created with `--replace`, and replacing an eight-GPU container frees
the old GPUs before the replacement loads. The generated hostname therefore
changes and the old generated DNS record cannot be the compatibility boundary.

Tinfoil custom domains can be attached to a running container on relaunch, but
they require the organization feature, a registered suffix, verified DNS, and
a deployment operation. Confirm those prerequisites read-only before planning
the route migration.

The 2026-08-07 read-only `tinfoil domain list` returned no registered custom
domains. That does not prove the enterprise feature is enabled, so custom-domain
enablement and the allowed suffix remain explicit prerequisites.

## PRECONDITIONS

1. Finish and verify the DeepSeek scheduler update independently.
2. Confirm Tinfoil custom domains are enabled for the organization and that
   `inference.finite.computer` is available. If not, stop and ask Tinfoil to
   enable the feature; do not replace the generated hostname directly.
3. Inventory every issued Runtime and Runner configuration that still reads
   the historical URL. `scripts/finite-status` must report the inventory or be
   extended before migration; ad-hoc row selection is not authority.
4. Prove the exact current Tinfoil tag, host, secrets-by-name, variables,
   custom-domain state, and rollback boundary.
5. Obtain separate approvals for DNS verification, the custom-domain relaunch,
   the Runtime population rollout, and the final container replacement.

## STEPS

All three phases below are TODO because no custom domain was registered as of
the last read-only inventory and this migration has not been exercised.

### TODO: Phase 1 — introduce the stable route

1. TODO: Register and verify `inference.finite.computer` using the exact TXT and
   CNAME values returned by Tinfoil. Keep Cloudflare proxying disabled for the
   verification records if Cloudflare owns the zone.
2. TODO: Relaunch the existing `kimi-k2-6` container at the exact already-running
   tag with the verified custom domain and all three existing secret names.
3. TODO: Prove attestation, health, authentication, inference, accounting, and the
   old generated route. Both routes must work before changing any reader.

### TODO: Phase 2 — migrate readers

1. TODO: Change the repository base-URL constant to the stable custom route and
   publish one canonical Runtime image.
2. TODO: Deploy the Runner configuration with the same route for new leases.
3. TODO: Roll existing Runtimes through the normal explicit artifact rollout.
4. TODO: Require every active Runtime to report the stable route and canonical
   DeepSeek model. Preserve user-owned custom providers and mixed-version GLM
   requests.
5. TODO: Hold at least one normal observation window with zero reads of the old
   generated hostname.

### TODO: Phase 3 — replace the container identity

TODO: Only after phase 2 is complete, create `finite-private` with `--replace` using
the exact approved release, host, custom domain, variable set, and secret-name
set. Expect eight-GPU downtime. Verify the new container UUID/name, stable
custom route, attestation, full protocol/accounting gates, bounded load, and
fleet status.

## VERIFY

The migration is complete only when `scripts/finite-status --json` is green,
the stable route passes attestation/auth/inference/accounting checks, every
active Runtime reader has converged, the old generated route has zero reads for
the approved observation window, and the exact prior release/settings are
retained with a proven procedure to recreate the prior container identity.
Retain the status outputs and reader inventory.

## ROLLBACK

- Before reader migration: relaunch the old container without the custom
  domain only if the stable route itself caused the failure.
- During reader migration: point only the failed rollout cohort back to the
  still-live generated route; do not rewrite persisted user configuration.
- During final replacement: use the exact pre-replacement release and recorded
  settings to replace back to the prior container identity. If the custom route
  cannot be rebound, stop and treat it as a routing incident.

Never delete the old production record, remove DNS, or call the rename complete
until the stable route has passed end-to-end proof and every issued Runtime
reader has converged.
