# Finite Private specialization release handoff

This checklist records provider-neutral release evidence. It does not authorize
a production deploy, restart, or runtime mutation.

## Preconditions

- Record the source commit, workflow, attestations, Runtime image digest, and
  Core and Runner binary digests. Production hosts fetch these CI-built,
  digest-pinned artifacts; they never build locally.
- Confirm the Runtime image contains the reviewed `finite-agentd` and health
  server. Production builds must retain the compiled worker endpoint
  `https://specialization.finite.vip/v1`; the disposable Devfinity override is
  not a release input.
- Confirm the canonical model is `deepseek-v4-flash-0731` and the only accepted
  bundle is `finite-private-multimodal-v1`. This is a hard cut:
  `aeon-multimodal` is not an alias or a converged state.
- Provision the route-scoped worker credential outside the repository. Never
  print it or place it in Runtime status, Core facts, or the main-model key.
- Deploy Runner capability advertisement before enabling the Core lifecycle
  fence. Core-first is unsafe. Incapable Runners may still lease Stop/Destroy,
  but not canonical creation or Restart/Recover/Upgrade.

## Configuration surfaces

Verify the credential name, never its value, on every active surface:

| Runner class | Configuration surface |
| --- | --- |
| Kata | `infra/hosts/lat1/systemd/runner.env.example`, `infra/nixos/hosts/finite-lat-3/runner.env.example`, and the corresponding `/etc/finite/runner.env` |
| Phala | `infra/hosts/lat1/systemd/phala-runner.env.example` and `/etc/finite/phala-runner.env` |
| Docker, Apple Container, Enclavia | `finite-saas-runner` process configuration |

All use `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY`. Empty,
unsupported, or oversized values fail Runner preflight before a creation lease.
Add any newly active Runner or host to this table before release.

## Evidence sequence

Use disposable state and fake credentials for pre-production proof. Do not
create paid Agents or redeem Launch Codes. Capture redacted
`scripts/finite-status --json` output before the sequence, after each launch or
convergence batch, and after cleanup.

1. **Admission:** for every enabled Runner class, prove valid configuration
   advertises the typed capability and absent/invalid configuration advertises
   unavailable capacity before leasing creation. Prove Stop/Destroy remain
   available while canonical Restart/Recover/Upgrade require the capability.
2. **New runtimes:** launch one disposable canonical runtime through Docker,
   Kata, Phala, Apple Container, and Enclavia where enabled. Each adapter must
   accept only the existing `/healthz` result. Retain status evidence showing
   the exact effective bundle `finite-private-multimodal-v1`; do not inspect raw
   Hermes configuration or call the worker directly.
3. **Existing runtimes:** use the explicit immutable Kata Runtime Upgrade path.
   An authoritative `active_inference_profile=finite-private` host fact is
   promoted transactionally to the universal typed profile. Missing or
   conflicting facts fail closed. Desired-only, wrong-bundle, ineffective, and
   cleanup-blocked runtimes remain unready.
4. **Quarantines:** Docker and Enclavia prove creation/readiness only. Apple
   Container existing-runtime specialization replacement and Kata
   RecoverKnownGood remain disabled until they have durable rollback
   envelopes. Existing Phala
   mutation remains disabled until `provision_update`/`commit_update` rollback
   and prior encrypted-environment restoration are proven.
5. **Fleet stop condition:** stop immediately if `scripts/finite-status --json`
   reports red/unknown or any eligible runtime is absent, wrong-bundle,
   desired-only, ineffective, cleanup-blocked, or unknown. Completion requires
   every eligible runtime to record `finite-private-multimodal-v1` and the
   command to exit green.

## Verification

- Health projects only bounded, allowlisted public fields plus bundle id,
  desired, and effective. It never exposes credentials or raw Hermes config.
- Every provider uses `/healthz`; none implements a specialization-specific
  provider probe.
- Run the targeted health, agentd, Runner provider, Core in-memory/PostgreSQL,
  finite-status, runtime-image, and Hermes probe gates before the applicable
  repository-wide checks.
- Remove all disposable runtimes and retain the redacted evidence.

## Rollback

Production rollback requires read-only fleet and lease evidence, a named
known-good release, a backup/restore boundary, an abort condition, and explicit
authorization for the exact mutation.

1. Fence canonical creation and Restart/Recover/Upgrade in Core while keeping
   Stop/Destroy available.
2. Wait for outstanding lifecycle leases to complete or expire, then verify
   none remain.
3. Stop and verify every Runner service. A capacity drain alone does not
   withdraw lifecycle leases.
4. Move Core and Runner together to the same verified release. Keep Runners
   stopped until both sides match; do not roll back a Runner while canonical
   typed runtimes remain.
5. Restart the fleet and repeat admission, synthetic launch, convergence, and
   fleet-stop checks before removing the fence.

After canonical profiles are active, rollback is a forward fix unless those
profiles have been explicitly removed. N/N-1 proof must continue to show that
old or ambiguous capacity cannot claim canonical private work while capable
capacity can, without accepting an obsolete bundle identity.
