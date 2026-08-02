# Artifact Identity and Manual Drift Audit — 2026-08-02

## Scope

This audit followed the per-service Nix source-scoping work prompted by the
August 1 rollout. It asks two questions:

1. Which services put a monorepo Git commit into a health response?
2. Which deployed-version records duplicate authoritative artifact state and
   can drift when a release or rollout requires manual copying?

The repository was checked at `85d08a486f7876e09fbd6e247e62c0c58a6130f3`.
GitHub releases were checked on 2026-08-02. This is a source audit, not a claim
about unobserved live host state.

## Decision: automatic package identity

FiniteChat is the only service that embeds a Git commit in its health payload.
Its Nix build currently injects the complete monorepo revision into the Chat
binary. As a result, an unrelated Sites-only commit changes the Chat package
path and causes an unnecessary Chat restart.

Do not add a manually maintained Chat revision pin. That would replace one
coupling bug with bookkeeping that can become stale.

Instead, Nix should derive a stable fingerprint from Chat's scoped build
inputs and expose it as Chat's artifact/source identity. The fingerprint must:

- change automatically when Chat or a real local Chat dependency changes;
- remain unchanged for unrelated monorepo changes;
- require no release-time edit or copied commit value; and
- be checked by the Chat deployment gate.

The overall deployment Git revision remains part of the NixOS closure and
rollout record. It answers “which repository revision was deployed?” The Chat
fingerprint answers “which Chat artifact is running?” These are different
facts and should not be overloaded into one field.

## Findings

### Health and build reporting

- `finitechat-server` is the only service whose health response includes a Git
  commit, branch, and dirty flag.
- Core, Sites, Brain, Identity, Agentd, and the remaining Rust services do not
  embed a Git commit in health. They use ordinary health state and/or Cargo
  package versions.
- The Agent Runtime publication workflow records and verifies the monorepo SHA
  and immutable image digest automatically in its generated build report and
  OCI metadata. That is the preferred pattern: identity comes from the build,
  not from a later handwritten record.

### Confirmed drift in `compat/matrix.toml`

`compat/matrix.toml` describes itself as hand-maintained release-time state,
but its factual entries already disagree with other repository and release
authorities:

- It lists FiniteChat releases only through `v0.1.5`; GitHub releases and tags
  exist through `finitechat/v0.1.9`.
- It records dashboard `2026-07-27.1` at digest `sha256:5b225…`, while the
  executable Nix configuration pins dashboard `2026-08-01.1` at
  `sha256:3f975…`.
- Its Runtime image section stops at `2026-07-22.1`; newer rollout evidence
  must therefore be reconciled before treating the matrix as current field
  inventory.

The compatibility policy is valuable. The duplicated factual inventory is the
part that drifts.

### Runtime promotion has repeated handoffs

Runtime publication correctly produces one immutable build report containing
the source revision and image digest. Promotion then requires an operator to
carry related values through several places:

1. register the artifact and digest in Core;
2. select the artifact ID in `/etc/finite/runner.env`;
3. use the same artifact ID in rollout commands; and
4. update `compat/matrix.toml`.

Core and the host selector represent real desired state and must remain
explicit. Recopying their facts into a handwritten compatibility inventory is
not an additional safety boundary. A future promotion command should consume
the generated report, perform the existing fail-closed checks, and emit or
validate the factual compatibility record automatically.

### Explicit pins that should remain explicit

- Digest-pinned Nix and OCI configuration is authoritative desired state, not
  redundant bookkeeping.
- Provider lanes may intentionally select different Runtime artifacts. The
  older Phala canary pin is not drift merely because Kata uses a newer image.
- `scripts/import-sync.toml` is durable import provenance and is advanced by
  `scripts/import-sync`; it is not a release-time duplicate.
- Dated run reports and post-mortems are historical evidence, not current-state
  registries.

## Follow-up boundary

The current Nix source-scoping change should include only:

- per-package source filesets;
- the automatic Chat source/artifact fingerprint;
- the updated Chat deployment check; and
- build and blast-radius evidence.

Separate follow-up work should:

1. derive or validate released-version lists from GitHub releases;
2. derive the dashboard's factual version/digest record from its authoritative
   Nix pin or published image metadata;
3. reconcile Runtime field inventory from Core/provider facts instead of a
   growing handwritten list; and
4. make Runtime promotion consume the generated publication report directly.

Human-maintained compatibility policy and explanatory notes should remain.
Machine-verifiable artifact facts should be generated or checked.
