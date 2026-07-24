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

# Runs all rust tests
test:
    cargo test --workspace --locked

# Web-only contributor gate: dashboard unit tests, lint, and production build.
web-check:
    cd finitecomputer-v2/apps/dashboard && npm ci && npm test && npm run lint && npm run build

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

# Focused protocol/process proof for the Hosted Web + Electron Device alpha.
chat-device-parity:
    cargo test --locked -p finitechat-core --test electron_device_parity
    cargo test --locked -p finitechat-hosted-device --test http device_link
    cargo test --locked -p finitechat-daemon
    cd finitechat/apps/electron-chat && npm ci && npm run check

# Reproducible local/CI gate for every surface changed by Electron parity.
chat-electron-check:
    cargo test --locked -p finitechat-daemon
    cargo test --locked -p finitechat-core --test electron_device_parity
    cargo test --locked -p finitechat-hosted-device
    cd finitechat/apps/electron-chat && npm ci && npm run check
    cd finitecomputer-v2/apps/dashboard && npm ci && npm test && npm run lint && npm run build

# Build the macOS Electron app. It is ad-hoc signed by default; release callers
# supply FINITECHAT_CODESIGN_IDENTITY (and optionally a temporary keychain) for
# Developer ID signing. npm never invokes Cargo; this recipe supplies the exact
# release daemon copied into app resources.
chat-electron-package:
    cargo build --locked --release -p finitechat-daemon
    cd finitechat/apps/electron-chat && npm ci && FINITECHAT_DAEMON_BINARY="{{justfile_directory()}}/target/release/finitechatd" npm run package:mac

# Opt-in Stripe test-mode clock E2E. Credentials come from the caller's
# environment and the harness never prints their values.
stripe-billing-clock:
    cd finitecomputer-v2/apps/dashboard && npm ci && npm run test:stripe-billing-clock
