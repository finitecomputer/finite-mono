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
