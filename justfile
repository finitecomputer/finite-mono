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

# Build an ad-hoc-signed internal macOS alpha. npm never invokes Cargo; this
# root recipe supplies the exact release daemon copied into app resources.
chat-electron-package:
    cargo build --locked --release -p finitechat-daemon
    cd finitechat/apps/electron-chat && npm ci && FINITECHAT_DAEMON_BINARY="{{justfile_directory()}}/target/release/finitechatd" npm run package:mac-alpha

# Opt-in Stripe test-mode clock E2E. Credentials come from the caller's
# environment and the harness never prints their values.
stripe-billing-clock:
    cd finitecomputer-v2/apps/dashboard && npm ci && npm run test:stripe-billing-clock
