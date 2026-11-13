# Finite Private serving configuration

The stable container/route identity is `finite-private`; the canonical model
label is `glm-5-3-flash`. `tinfoil-config.glm-5.3-flash.candidate.yml` pins the
checkpoint, MPK, images and serving command for usage-api admission. The
separate degraded-allowlist configuration is an explicit operational override,
not evidence of current production state.

Optional [Numes marketplace serving](numes.md) has separate credentials,
accounting and a Finite-owned concurrency cap that can be tuned without a deploy.
It is disabled unless explicitly configured.

Inspect the live measured tag and configuration with `scripts/finite-status`
and `infra/runbooks/finite-private-ops.sh status` before a deployment. Preserve
the exact previous tag, container identity, configuration and credential custody
for rollback. Do not infer deployed state from a dated README or candidate name.

Run the candidate config, protocol and quality checks before promotion. Build
limiter images through `service-images.yml`; pin immutable digests in the
satellite and publish a measured release. Both release workflow phases must
succeed; a tag alone is not deployable. Publishing and production rollout are
separate operations: publishing does not update a running instance.

Use a current Tinfoil CLI's `container update INSTANCE --tag TAG
--hold=false --mark-latest=false` or the dashboard's **Update** action for a
running instance. Review the current-to-target plan before confirming. Eight
GPUs require replacement with downtime; holding for review is unavailable and
the update cannot be canceled after the old enclave stops. Do not override
secrets, variables, security settings or volumes during a CVM-only update.
Omitted overrides preserve the selected secret names, but deployment resolves
their current values. Before confirming, independently check rotations and
credential compatibility with the secret custodian. The plan lists names and
refreshes, but does not prove that secret values match the serving enclave.

Confirm the exact new tag and verify attestation against its published
measurement before accepting readiness. The ops helper's `wait-ready` checks
HTTP readiness and successful status retrieval, so it can pass while the old
release is still serving. Then check streaming through `[DONE]`, responses,
all retained model aliases, negative authentication and usage settlement, plus
`scripts/finite-status` before and after. `mixed-version-canary` covers only
`glm-5-2`; exercise the other retained aliases explicitly. A failed replacement
needs **Deploy** on the same instance; it does not automatically roll back.

If an older operator CLI's `container relaunch` returns HTTP 404, use the current
CLI or dashboard **Update** for a running instance, or **Deploy** for a failed or
stopped instance, selecting the exact approved measured tag. Preserve its
secrets, confidential-computing settings and identity. This uses the same
production authorization boundary; do not
delete and recreate the instance to work around the obsolete CLI endpoint.
The legacy ops wrapper requires `FINITE_PRIVATE_RELAUNCH_APPROVED` to equal
the exact selected tag; that guard does not establish API compatibility.

Rollback uses the preserved measured tag and the same explicit authorization
boundary. Keep old client aliases and secrets until supported readers and
recovery sets are proven. Never copy live credentials into configs or evidence.
Rolling back to an unpatched CVM restores its vulnerabilities and incurs another
cold start. Readiness timeouts are decision points, not automatic rollback
instructions or a bound on total recovery time.

The candidate pins CVM 0.14.17, which includes the CVE-2026-98432 kernel fix.
Multi-GPU H200/Hopper enclaves also require the measured top-level setting
`security.allow-unencrypted-nvlink: true`; without it this CVM refuses to boot.
This explicitly accepts unencrypted inter-GPU NVLink traffic. GPU attestation
and CPU-to-GPU encryption remain required. Review that acceptance before
publishing or deploying the candidate; see [Tinfoil GPU security](https://docs.tinfoil.sh/containers/config-runtime#gpu-security).

Bridge-networking hosts require CVM 0.11.0 or newer. A preserved 0.10.8 tag
cannot boot there; rollback requires a compatible measured release or provider
remediation. The retained degraded-allowlist file also pins 0.10.8 and host IPC,
so it is not a usable fallback on those hosts. The candidate uses
private IPC with explicit shared memory; validate these settings before
measuring any replacement or degraded configuration.

See [satellite operations](../README.md). Outstanding admission/route
qualification is tracked in [FIN-95](https://linear.app/finitecomputer/issue/FIN-95).
