# lat1/lat3 sops-nix Phased Secret Migration
Status: PROPOSED

Owner: Paul

Opened: 2026-08-12

Acceptance: `finite-lat-1` and `finite-lat-3` have one Nix-owned secret
contract, every live production secret is declared in that contract, all
production secret sources have migrated from legacy host files to `sops-nix`,
no production service consumes mutable operator-placed plaintext under
`/etc/finite` or `/etc/finite-saas` as its source of truth, and
`scripts/finite-status` is green after the final migration.

Expiry: 2026-08-26

## Scope

This run replaces the ad hoc host-file secret model with a phased
`sops-nix` migration. It deliberately avoids SecretSpec for now: the team is
small, the production estate is two NixOS boxes, and the useful contract can
live in Nix plus SOPS files.

The migration is not a big bang. The intended shape is:

1. Add a Nix secret contract that can point at either legacy files or SOPS.
2. Add `sops-nix` plumbing.
3. Migrate one low-risk secret first.
4. Expand the contract until every live secret is declared.
5. Batch-migrate remaining secrets from legacy to SOPS.
6. Remove legacy live paths.

Mixed legacy/SOPS mode is a temporary migration state, not the final design.

## Guardrails

- [ ] Treat this document as planning only until it is made ACTIVE per
      [`docs/runs/README.md`](README.md).
- [ ] Run `scripts/finite-status` before and after every production rollout.
- [ ] Never print or commit secret values, derived hashes, fingerprints, or
      password-derived evidence.
- [ ] Do not rotate production credentials as part of this migration unless
      the credential owner explicitly authorizes that separate rotation.
- [ ] Preserve chat availability. Lat1 deployments must keep the existing
      hosted recovery snapshot and rollback boundary.
- [ ] Never interpolate decrypted secret contents into Nix expressions,
      generated store files, unit text, or logs.
- [ ] Do not delete old plaintext live files until both hosts have booted the
      SOPS-backed generation and the old paths are proven unused.
- [ ] Keep the mixed migration period bounded; every still-legacy host-file
      input must remain documented in the inventory until it is migrated.

## Target Design

- [ ] `finite.secrets` is the Nix contract for SOPS-managed production secret
      consumers. During migration, absence from `finite.secrets` means the
      service still uses its existing host-file path.
- [ ] Each SOPS declaration records logical name, host scope, SOPS source,
      destination path, owner, group, mode, kind, required env names, consumer
      services, and restart/reload behavior.
- [ ] Migrated service modules consume only the centralized path, not
      hardcoded `/etc/finite/*.env` or `/etc/finite-saas/*.env` paths.
- [ ] A checker proves every SOPS-managed entry is metadata-complete without
      printing values.
- [ ] Final-state verification proves every live consumer is SOPS-managed or
      has a documented recovery-only exception.

Example shape:

```nix
finite.secrets.files.metrics-remote-write = {
  scope = [ "finite-lat-1" "finite-lat-3" ];
  sopsFile = ../secrets/shared/metrics-remote-write.env;
  destinationPath = "/run/secrets/finite/metrics-remote-write.env";
  owner = "root";
  group = "root";
  mode = "0600";
  kind = "env";
  requiredEnvNames = [
    "FINITE_METRICS_REMOTE_WRITE_USERNAME"
    "FINITE_METRICS_REMOTE_WRITE_PASSWORD"
  ];
  consumers = [ "alloy.service" ];
};
```

## Phase 0: Inventory Baseline

Progress 2026-08-12: source-only, values-free baseline captured in
[`docs/runs/lat1-lat3-sops-nix-inventory-baseline.md`](lat1-lat3-sops-nix-inventory-baseline.md).
No production SSH, live secret reads, local production Nix eval/build, or
rollout actions were performed.

- [x] Run a repo-wide static scan for every likely secret consumer and save
      only paths, variable names, and service names:

```sh
rg -n \
  'EnvironmentFile|environmentFiles|environmentFile|LoadCredential|LoadCredentialEncrypted|SetCredential|privateKeyFile|keyFile|cert|/etc/finite|/etc/finite-saas|/run/secrets|sops\.secrets|sops\.templates|BORG_|LITESTREAM_|SECRET|TOKEN|API_KEY|PASSWORD|PRIVATE_KEY' \
  infra/nixos infra/hosts scripts docs \
  -S
```

- [x] Review every hit in `infra/nixos/modules/` and both host directories.
      Classify each as live secret input, non-secret config, historical note,
      test/example, or retired reference.
- [ ] Evaluate both NixOS configs on the approved Linux builder, not on lat1
      or a laptop, and extract rendered consumers:
      `systemd.services.*.serviceConfig.EnvironmentFile`,
      `systemd.services.*.serviceConfig.LoadCredential`,
      `virtualisation.oci-containers.containers.*.environmentFiles`,
      `services.alloy.environmentFile`,
      `services.oauth2-proxy.keyFile`,
      `networking.wireguard.interfaces.*.privateKeyFile`, and Caddy TLS paths.
- [ ] Compare rendered lat1 consumers to
      [`infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json`](../../infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json).
- [ ] Create the missing lat3 values-free secret contract, covering at least
      Runner, runtime provider secrets, metrics remote-write, Identity
      operator credential, and WireGuard. The generated inventory is
      authoritative if it finds more.
- [x] Check scripts and runbooks for still-live operator paths not visible in
      Nix modules, especially deploy, rollout, recovery, Identity credential
      installers, Phala credential installers, Litestream, and Borg.
- [ ] Verify dynamic requirements that cannot be expressed as a fixed file
      list, including `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*` coverage for every
      active Core credential and Postgres role password agreement with
      `FC_CORE_DATABASE_URL`.
- [x] Produce a source-only values-free inventory baseline recording live
      items by path, kind, service consumer, required variable names, and
      initial SOPS source-layout hypothesis.
- [ ] Produce a reviewed values-free inventory recording each live item by
      path, owner, group, mode, kind, service consumer, required variable
      names, current source path, and planned SOPS source.

## Phase 1: Nix Contract and SOPS Plumbing

Progress 2026-08-12: foundation plumbing added with an SOPS-only contract and
no entries yet. Absence from `finite.secrets.files` means the existing host-file
path remains untouched. No service consumer is pointed at SOPS yet. The legacy
backend bridge was removed after review so `finite.secrets.files` only means
SOPS-owned. The host-level contract evaluates values-free on this Darwin
workstation; production closure build/eval proof on finite-lat-2 is still
required before switching any consumer.

- [x] Add `sops-nix` to `flake.nix` and import its module for
      `finite-lat-1` and `finite-lat-3`.
- [x] Add host `age` identity installation instructions:
      `/var/lib/sops-nix/finite-lat-1.agekey` and
      `/var/lib/sops-nix/finite-lat-3.agekey`, root-only.
- [x] Add bootstrap `infra/nixos/secrets/.sops.yaml` with the first operator
      recipient for human-decryptable staging.
- [ ] Add recovery and host recipients, then run
      `just infra secrets updatekeys` before any rollout depends on SOPS.
- [x] Add `infra/nixos/modules/secrets.nix` with the `finite.secrets.files`
      option schema.
- [x] Implement path resolution so `config.finite.secrets.files.<name>.path`
      returns the decrypted SOPS runtime path.
- [x] Preserve owner, group, mode, required env names, and consumer metadata
      in the SOPS-managed contract.
- [x] Add a host-generic checker that reads the Nix contract and validates
      metadata/name completeness without emitting values.
- [x] Add tests proving the checker never prints secret values.
- [ ] Build/evaluate both host configs before any service is pointed at SOPS.

## Phase 2: One-Secret SOPS Pilot

Preferred pilot: `metrics-remote-write.env`. It affects monitoring delivery,
not chat, Agent state, Core, Caddy, or recovery.

Fallback pilot: `searxng.env`, only if Search is explicitly accepted as lower
blast radius for this rollout.

Progress 2026-08-12: selected `metrics-remote-write.env` as the pilot. Alloy is
the only prepared consumer: it falls back to `/etc/finite/metrics-remote-write.env`
while the SOPS contract entry is absent, and will read
`config.finite.secrets.files."metrics-remote-write".path` after the pilot entry
is added. A stdin-only `just infra secrets ingest` helper is available for the
staging step.

Progress 2026-08-20: staged the live pilot value at
`infra/nixos/secrets/shared/metrics-remote-write.env` encrypted to the bootstrap
operator recipient only. This is intentionally not deployable yet: host
recipients are deferred, `sops.secrets` still evaluates to `{ }`, and Alloy
still reads `/etc/finite/metrics-remote-write.env` until the contract entry is
added.

- [x] Choose exactly one pilot secret.
- [x] Leave all other secrets absent from `finite.secrets.files` and on their
      existing host-file paths.
- [x] Add a stdin-only SOPS ingestion helper so operators do not hand-roll the
      encryption command.
- [x] Encrypt the pilot value into its planned SOPS file without printing it.
- [ ] Add only the pilot contract entry to `finite.secrets.files`.
- [x] Update only the pilot consumer to use
      `config.finite.secrets.files.<name>.path` when the pilot entry exists.
- [ ] Build the affected host closure on the approved Linux builder.
- [ ] Run `scripts/finite-status` before rollout.
- [ ] Deploy the affected host.
- [ ] Verify decrypted file owner, group, mode, required env names, and service
      health.
- [ ] Verify the pilot service restarts or reloads as intended when the SOPS
      material changes.
- [ ] Run `scripts/finite-status` after rollout.
- [ ] Record what broke, what was noisy, and what must change before broad
      migration.

## Phase 3: Migrate Every Live Secret

- [ ] Add each live lat1 secret to `finite.secrets.files` only as it migrates
      to SOPS.
- [ ] Add each live lat3 secret to `finite.secrets.files` only as it migrates
      to SOPS.
- [ ] Classify each inventory item as `lat1-only`, `lat3-only`, `shared`,
      `non-secret`, `retired`, or `recovery-only`.
- [ ] Confirm that shared entries are intentionally shared. Prefer per-host or
      per-service credentials where the product boundary allows it.
- [ ] Replace hardcoded secret paths in Nix modules only when that secret is
      moved to SOPS.
- [ ] Add a static guard that fails on new direct live references to
      `/etc/finite/*.env` or `/etc/finite-saas/*.env` for already-migrated
      secrets.
- [ ] Add a Nix eval gate proving every migrated rendered live consumer
      resolves through `finite.secrets`.
- [ ] Confirm no service behavior changes for secrets that remain absent from
      `finite.secrets.files`.

Initial classification hypothesis, to be checked against the generated
inventory:

- [ ] `lat1-only`: `core.env`, `dashboard.env`, `phala-runner.env`,
      `hosted-web-device.env`, `brain-authority.env`,
      `sites-viewer-session.env`, `sites.env`, `searxng.env`,
      `firecrawl.env`, Cloudflare Origin CA key/cert, Borg credentials, and
      Litestream credentials.
- [ ] `lat3-only`: `runner.env` and WireGuard private key.
- [ ] `shared`: `runtime-secrets.env`, `metrics-remote-write.env`, and
      `identity-operator.env` only if the same credential is deliberately used
      on both hosts.

## Phase 4: Batch Migration to SOPS

Migrate in small batches. Each batch must name affected services, expected
restarts, rollback boundary, and verification commands before deployment.

- [ ] Batch 1: lower-blast-radius observability/search secrets.
- [ ] Batch 2: Runner support secrets on lat3 after preserving drain/admission
      state.
- [ ] Batch 3: Runner support secrets on lat1 after preserving current
      existing-Agent lifecycle behavior.
- [ ] Batch 4: Sites/Brain/Identity support credentials that do not gate chat
      startup.
- [ ] Batch 5: dashboard/Core credentials after a lat1 hosted recovery
      snapshot.
- [ ] Batch 6: WireGuard private keys and Caddy origin cert/key after explicit
      connectivity/TLS rollback steps are written.
- [ ] Batch 7: Borg and Litestream recovery credentials after restore and
      backup-health checks are updated.
- [ ] For each batch, encrypt staged host values into SOPS without printing
      values.
- [ ] For each batch, add or update the SOPS-managed contract entries.
- [ ] For each batch, build the host closure on the approved Linux builder.
- [ ] For each batch, run `scripts/finite-status` before rollout.
- [ ] For each batch, deploy only the affected host(s).
- [ ] For each batch, verify owner, group, mode, required variable names,
      service health, and expected restart behavior.
- [ ] For each batch, run `scripts/finite-status` after rollout.
- [ ] Stop the migration if any batch creates unexpected service churn,
      ambiguous health, or a new chat/onboarding risk.

## Phase 5: Remove Host-File Sources

- [ ] Confirm every live production secret is present in `finite.secrets.files`
      or has a documented recovery-only exception.
- [ ] Confirm no active unit references old plaintext source paths.
- [ ] Remove old plaintext live files only after the SOPS-backed generation is
      active and verified on both hosts.
- [ ] Update Borg backup inputs so obsolete plaintext secret roots are not the
      recovery mechanism.
- [ ] Update disaster recovery docs: restore host state, install host age
      identity or break-glass identity, decrypt SOPS material through
      activation, then start services.
- [ ] Update `infra/nixos/README.md` so SOPS is the production secret source
      of truth.

## Acceptance Request

- [ ] Record exact Git revision and host closure paths for `finite-lat-1` and
      `finite-lat-3`.
- [ ] Record `scripts/finite-status` before and after summaries.
- [ ] Record the values-free secret contract output for both hosts.
- [ ] Record that no live service references old mutable plaintext secret
      paths as source of truth.
- [ ] Ask Paul to verify:
      `finite.computer`, `chat.finite.computer/health`,
      `brain.finite.computer/health`, `identity.finite.vip`, one Sites host,
      lat3 Runner status, and recovery/backup health.

## Parking Lot

- [ ] Per-variable SOPS templates for env files after whole-file migration is
      stable.
- [ ] Credential rotation campaign after custody migration is complete.
- [ ] Vault, cloud secret-manager, or SecretSpec only if future scale requires
      provider abstraction, leases, centralized audit trails, or live rotation.
