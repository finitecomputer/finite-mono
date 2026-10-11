# Finite Private serving configuration

The stable container/route identity is `finite-private`; the canonical model
label is `glm-5-3-flash`. `tinfoil-config.glm-5.3-flash.candidate.yml` pins the
checkpoint, MPK, images and serving command for usage-api admission. The
separate degraded-allowlist configuration is an explicit operational override,
not evidence of current production state.

Inspect the live measured tag and configuration with `scripts/finite-status`
and `infra/runbooks/finite-private-ops.sh status` before a deployment. Preserve
the exact previous tag, container identity, configuration and credential custody
for rollback. Do not infer deployed state from a dated README or candidate name.

Run the candidate config, protocol and quality checks before promotion. Build
limiter images through `service-images.yml`; pin immutable digests in the
satellite and publish a measured release. Authorized relaunch requires
`FINITE_PRIVATE_RELAUNCH_APPROVED` to equal the exact selected tag. Confirm
readiness, streaming, mixed-version model aliases, negative authentication and
usage settlement afterward, plus `scripts/finite-status` before and after.

If the pinned CLI's `container relaunch` returns HTTP 404, use the Tinfoil
dashboard's **Deploy** action on the existing instance, selecting the exact
approved measured tag. Preserve its secrets, confidential-computing settings
and identity. This uses the same production authorization boundary; do not
delete and recreate the instance to work around the obsolete CLI endpoint.

Rollback uses the preserved measured tag and the same explicit authorization
boundary. Keep old client aliases and secrets until supported readers and
recovery sets are proven. Never copy live credentials into configs or evidence.

Bridge-networking hosts require CVM 0.11.0 or newer. A preserved 0.10.8 tag
cannot boot there; rollback requires a compatible measured release or provider
remediation. The retained degraded-allowlist file also pins 0.10.8 and host IPC,
so it is not a usable fallback on those hosts. The 0.14.13 candidate uses
private IPC with explicit shared memory; validate these settings before
measuring any replacement or degraded configuration.

See [satellite operations](../README.md). Outstanding admission/route
qualification is tracked in [FIN-95](https://linear.app/finitecomputer/issue/FIN-95).
