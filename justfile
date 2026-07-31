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

# Web-only contributor gate: dashboard unit tests, lint, and production build.
web-check:
    cd finitecomputer-v2/apps/dashboard && pnpm install --frozen-lockfile && pnpm test && pnpm run lint && pnpm run build

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
    cargo test --locked -p finitesitesd --test e2e identity_authority_can_satisfy_email_git_auth_without_sites_email_key
    cargo test --locked -p finite-brain-server owner_creates_personal_brain_by_managed_agent_email_without_trusting_navigation_npub
    cargo test --locked -p devfinity generated_yaml_contains_core_services
    nix eval --raw .#nixosConfigurations.finite-lat-1.config.system.build.toplevel.drvPath

# Static contract: active first-party callers use /v1, not the legacy /_admin API.
brain-api-route-check:
    python3 scripts/check-brain-api-routes.py

# Evaluate and build immutable system + disko outputs on finite-lat-2. The
# helper prints the exact, GC-rooted system path used for the deploy handoff.
nixos-build-lat1 rev:
    #!/usr/bin/env bash
    set -euo pipefail
    exec scripts/nix-build-lat2 {{ quote(rev) }}

# Full lat1 deploy for a committed main rev: prebuild on lat2, copy/switch
# lat1, then verify the running closure and dashboard digest by state.
[positional-arguments]
deploy-lat1 rev *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec scripts/deploy-lat1 "$@"

# Static parsing, transport, ordering, and failure-propagation contract for the
# optional existing-Runtime rollout appended to a lat1 deploy.
lat1-rollout-contract:
    bash -n scripts/deploy-lat1 scripts/rollout-lat1-runtime-artifact
    python3 -m unittest discover -s scripts/tests -p 'test_deploy_lat1_rollout.py'

# Evaluated systemd ordering plus synthetic transient/persistent endpoint
# behavior for the aggregate production healthcheck.
lat1-healthcheck-contract:
    python3 scripts/check_lat1_healthcheck_contract.py

# Static contract: Docker, Kata, and Phala share one Runtime image/build lane.
runtime-image-contract:
    python3 scripts/check_runtime_image_contract.py
    python3 -m unittest discover -s scripts/tests -p 'test_runtime_image_contract.py'

# Static production contract: Dashboard and Core must enforce the same Price.
stripe-price-contract:
    python3 scripts/check_stripe_price_contract.py

# Synthetic empty-target proof for the complete hosted Recovery Set contract.
hosted-recovery-contract:
    infra/scripts/test-hosted-web-chat-restore

# Values-free file/mode/name contract for rebuilding lat1 secret inputs.
lat1-secret-bootstrap-contract:
    python3 -m json.tool infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json >/dev/null
    python3 -m unittest scripts.tests.test_check_lat1_secret_bootstrap

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
    cd finitecomputer-v2/apps/dashboard && pnpm install --frozen-lockfile && pnpm test && pnpm run lint && pnpm run build

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
