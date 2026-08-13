set shell := ["scripts/dev-shell", "-cu"]

# Commands for controlling devfinity (local dev harness)
mod dev 'devfinity/justfile'
# Commands for controlling finite sites
mod sites 'finite-sites/justfile'
# Commands for controlling finite search
mod search 'finite-search/justfile'
# Commands for validating finite skills
mod skills 'finite-skills/justfile'

# Lists just commands
default:
    just --list-submodules --list

# Cargo check across the workspace
check:
    cargo check --workspace --locked

# Formats all rust code
fmt:
    cargo fmt --all

# Runs all Rust tests with isolated devfinity-managed test infrastructure
test:
    cargo run --quiet --locked -p devfinity -- run -- cargo test --workspace --locked

# Runs the opt-in full-history Device convergence stress test.
chat-history-stress:
    cargo test --locked -p finitechat-core --lib tests::late_same_account_device_converges_topics_named_chats_and_archives_after_restart -- --ignored --exact --nocapture

# Web-only contributor gate: dashboard unit tests, lint, project-wide typecheck,
# and production build.
web-check:
    cd finitecomputer-v2/apps/dashboard && pnpm install --frozen-lockfile && pnpm test && pnpm run lint && pnpm run typecheck && pnpm run build

# Static contract: first-party Brain surfaces use only the Greenfield Brain vocabulary.
brain-language-check:
    python3 scripts/check-brain-product-language.py

# Compositional proof that one managed Agent Principal is bound once, consumed
# consistently by Chat/Sites/Brain, and never turns identity equivalence into a
# cross-product permission grant.
identity-conformance:
    cargo test --locked -p finite-identity --test authority
    cargo test --locked -p finite-saas-runner run_once_binds_canonical_agent_email_before_completion
    cargo test --locked -p finitechat-hosted-device --test http initial_hosted_chat_setup_registers_the_users_public_identity
    cargo test --locked -p finitechat-hosted-device --test http new_agent_binding_stays_unchanged_across_duplicate_selection_and_restart
    cargo test --locked -p finitesites-store sites_email_principal_supports_multiple_revocable_authorized_keys
    cargo test --locked -p finitesites-store sites_identity_reconciliation_is_additive_and_idempotent
    cargo test --locked -p finitesites-store verified_core_agent_evidence_never_resurrects_a_revoked_key
    cargo test --locked -p finitesites-engine authorized_sites_key_can_manage_same_mailbox_owned_project_without_identity_link
    cargo test --locked -p finitesitesd managed_agent_account_uses_core_service_auth_and_exact_agent_email
    cargo test --locked -p finitesitesd --test e2e mailbox_proof_registers_and_revokes_sites_key_without_changing_legacy_grant
    cargo test --locked -p finitesitesd --test e2e identity_authority_can_satisfy_email_git_auth_without_sites_email_key
    cargo test --locked -p finite-brain-server owner_creates_personal_brain_by_managed_agent_email_without_trusting_navigation_npub
    cargo test --locked -p devfinity generated_yaml_contains_core_services
    nix eval --raw .#nixosConfigurations.finite-lat-1.config.system.build.toplevel.drvPath

# Static contract: active first-party callers use /v1, not the legacy /_admin API.
brain-api-route-check:
    python3 scripts/check-brain-api-routes.py

# Build immutable system + disko outputs on the current x86_64 Linux host and
# package them as a file binary cache deploy artifact.
nixos-build-lat1-closure rev out_dir="target/lat1-nixos-closure":
    #!/usr/bin/env bash
    set -euo pipefail
    exec scripts/build-lat1-nixos-closure-artifact {{ quote(rev) }} {{ quote(out_dir) }}

# Full lat1 deploy from a CI-built closure artifact: copy/switch lat1, then
# verify the running closure and dashboard digest by state.
[positional-arguments]
deploy-lat1-closure artifact_dir *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec scripts/deploy-lat1-closure-cache "$@"

# Static parsing, transport, ordering, and failure-propagation contract for the
# optional existing-Runtime rollout appended to a lat1 deploy.
lat1-rollout-contract:
    bash -n scripts/deploy-lat1-closure-cache scripts/build-lat1-nixos-closure-artifact scripts/rollout-lat1-runtime-artifact
    python3 -m unittest discover -s scripts/tests -p 'test_deploy_lat1_rollout.py'
    python3 -m unittest scripts.tests.test_lat1_closure_artifact

# Evaluated systemd ordering plus synthetic transient/persistent endpoint
# behavior for the aggregate production healthcheck.
lat1-healthcheck-contract:
    python3 scripts/check_lat1_healthcheck_contract.py

# Digest pins, public probes, route exposure, and dashboard provisioning for
# the dedicated Ubuntu monitoring VPS.
self-hosted-monitoring-contract:
    bash -n infra/monitoring/self-hosted/install-ubuntu infra/monitoring/self-hosted/verify
    python3 scripts/check_self_hosted_monitoring_contract.py

# Anti-drift contract: both Kata Runner hosts render one shared runner-role
# module; evaluated env and unit fragments may differ only in the declared
# per-host set.
runner-host-contract:
    python3 scripts/check_runner_host_contract.py

# Canonical read-only platform status: Nix-name contract plus Aug-1 fleet math.
finite-status-contract:
    python3 scripts/check_finite_status_contract.py
    python3 -m unittest discover -s scripts/tests -p 'test_finite_status.py'
    python3 -m unittest discover -s scripts/tests -p 'test_finite_runtime_metrics.py'

# Static contract: every Identity Authority route the fsite CLI calls is on
# the service-owned public surface (public_router); the edge proxies, never
# filters. Live probe: scripts/identity-edge-contract-gate.py [--target URL].
identity-edge-contract:
    python3 scripts/identity-edge-contract-gate.py --static
    python3 -m unittest discover -s scripts/tests -p 'test_identity_edge_contract_gate.py'

# Static contract: Docker, Kata, and Phala share one Runtime image/build lane.
runtime-image-contract:
    python3 scripts/check_runtime_image_contract.py
    python3 -m unittest discover -s scripts/tests -p 'test_runtime_image_contract.py'

# Measured eight-H200 DeepSeek serving identity and scheduler contract.
finite-private-deepseek-contract:
    python3 scripts/check_finite_private_deepseek_candidate.py
    python3 -m unittest discover -s scripts/tests -p 'test_finite_private_deepseek_candidate.py'
    python3 -m unittest discover -s scripts/tests -p 'test_finite_private_ops.py'
    python3 -m unittest discover -s scripts/tests -p 'test_check_deepseek_v4_0731_quality.py'

# Promotion-time form: the model image and MPK must already be immutable.
finite-private-deepseek-release-contract:
    python3 scripts/check_finite_private_deepseek_candidate.py --release-ready

# Static production contract: Dashboard and Core must enforce the same Price.
stripe-price-contract:
    python3 scripts/check_stripe_price_contract.py

# Synthetic empty-target proof for the complete hosted Recovery Set contract.
hosted-recovery-contract:
    python3 -m unittest discover -s scripts/tests -p 'test_snapshot_sqlite.py'
    infra/scripts/test-hosted-web-chat-restore

# Synthetic offline litestream replicate -> restore -> integrity proof.
litestream-recovery-contract:
    bash -n infra/scripts/test-litestream-restore
    infra/scripts/test-litestream-restore

# Values-free file/mode/name contract for rebuilding lat1 secret inputs.
lat1-secret-bootstrap-contract:
    python3 -m json.tool infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json >/dev/null
    python3 -m unittest scripts.tests.test_check_lat1_secret_bootstrap

# Values-free NixOS secret contract shape. Host coverage gates stay separate
# until rendered lat2 eval is wired into the migration.
nixos-secrets-contract:
    python3 scripts/check_nixos_secrets_contract.py
    python3 -m unittest scripts.tests.test_nixos_secrets_contract
    python3 -m unittest scripts.tests.test_nixos_sops_ingest
    python3 -m unittest scripts.tests.test_nixos_sops_operator_key
    python3 -m unittest scripts.tests.test_nixos_sops_updatekeys

# Create or inspect the local operator age key used by SOPS. Prints only the
# public age recipient; the private key stays in SOPS_AGE_KEY_FILE or
# ~/.config/sops/age/keys.txt.
[positional-arguments]
nixos-sops-operator-key *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec python3 scripts/nixos_sops_operator_key.py "$@"

# Encrypt one NixOS SOPS secret from stdin into infra/nixos/secrets.
# Example:
#   ssh root@finite-lat-1 'sudo cat /etc/finite/metrics-remote-write.env' \
#     | just nixos-sops-ingest shared metrics-remote-write.env --logical-name metrics-remote-write --required-env-name FINITE_METRICS_REMOTE_WRITE_USERNAME --required-env-name FINITE_METRICS_REMOTE_WRITE_PASSWORD --consumer alloy.service --restart-unit alloy.service
[positional-arguments]
nixos-sops-ingest *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec python3 scripts/nixos_sops_ingest.py "$@"

# Update SOPS file recipient metadata after .sops.yaml recipient changes.
[positional-arguments]
nixos-sops-updatekeys *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec python3 scripts/nixos_sops_updatekeys.py "$@"

# Synthetic deletion, watermark, active-job, and stale-lease safety contract
# for finite-lat-2's operator-installed runner guardrails.
lat2-runner-guardrails-contract:
    bash -n infra/hosts/lat2/configure-runner-linger infra/hosts/lat2/restart-idle-runner infra/hosts/lat2/runner-maintenance
    python3 -m unittest scripts.tests.test_lat2_runner_guardrails

# Disposable Docker-backed real Hermes/managed-skill/fbrain/Brain/Product Client matrix.
brain-product-matrix:
    bash scripts/tests/test_devfinity_brain_readiness.sh
    node finite-brain/crates/finite-brain-server/src/product-client.test.js
    cargo test --locked -p finite-brain-server brain_update_notification_carries_the_authoritative_cursor
    cargo test --locked -p finite-brain-cli supervisor_discovers_every_opened_brain_working_tree
    FINITE_BRAIN_COLLABORATION_SMOKE_REPORT="$PWD/target/brain-product-matrix/organization-collaboration.json" cargo test --locked -p finite-brain-cli --test fbrain_process_acceptance built_fbrain_process_two_independent_homes_open_restricted_collaboration -- --nocapture
    scripts/check-brain-collaboration-smoke-report.py "$PWD/target/brain-product-matrix/organization-collaboration.json"
    scripts/devfinity-brain-product-matrix

# Focused protocol/process proof for the Hosted Web + Electron Device alpha.
chat-device-parity:
    cargo test --locked -p finitechat-core --test electron_device_parity
    cargo test --locked -p finitechat-hosted-device --test http
    cargo test --locked -p finitechat-daemon
    cd finitechat/apps/electron-chat && pnpm install --frozen-lockfile && pnpm run check

# Reproducible local/CI gate for every surface changed by Electron parity.
chat-electron-check:
    cargo test --locked -p finitechat-daemon
    cargo test --locked -p finitechat-core --test electron_device_parity
    cargo test --locked -p finitechat-hosted-device
    cd finitechat/apps/electron-chat && pnpm install --frozen-lockfile && pnpm run check
    cd finitecomputer-v2/apps/dashboard && pnpm install --frozen-lockfile && pnpm test && pnpm run lint && pnpm run typecheck && pnpm run build

# Build the macOS Electron app. It is ad-hoc signed by default; release callers
# supply FINITECHAT_CODESIGN_IDENTITY (and optionally a temporary keychain) for
# Developer ID signing. pnpm never invokes Cargo; this recipe supplies the exact
# release daemon copied into app resources.
chat-electron-package:
    cargo build --locked --release -p finitechat-daemon
    cd finitechat/apps/electron-chat && pnpm install --frozen-lockfile && FINITECHAT_DAEMON_BINARY="{{ justfile_directory() }}/target/release/finitechatd" pnpm run package:mac

# Regenerate the native iOS bridge/project and prove the unsigned Release
# configuration Xcode Cloud will archive.
ios-cloud-preflight:
    finitechat/scripts/ios-xcode-cloud-preflight.sh

# Opt-in Stripe test-mode clock E2E. Credentials come from the caller's
# environment and the harness never prints their values.
stripe-billing-clock:
    cd finitecomputer-v2/apps/dashboard && pnpm install --frozen-lockfile && pnpm run test:stripe-billing-clock
