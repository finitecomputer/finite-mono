set shell := ["scripts/dev-shell", "-cu"]

# Domain command modules.
mod brain 'finite-brain/justfile'
mod chat 'finitechat/justfile'
mod computer 'finitecomputer-v2/justfile'
mod dashboard 'finitecomputer-v2/apps/dashboard/justfile'
mod dev 'devfinity/justfile'
mod identity 'finite-identity/justfile'
mod infra 'infra/justfile'
mod monitoring 'infra/monitoring/justfile'
mod nixos 'infra/nixos/justfile'
mod runtime-images 'finitecomputer-v2/deploy/finite-computer/images/justfile'
mod sites 'finite-sites/justfile'
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

# Formats all Python code with the repo-pinned ruff (flake.nix `pyToolPkgs.ruff`
# — the exact attr CI's hermes-bridge-ci shell checks with). Never invoke
# `nix run nixpkgs#ruff` or a host ruff: format output differs between
# versions and that is how CI formatting gates go red.
fmt-py:
    nix develop .#default -c ruff format .

# Lints all Python code with the repo-pinned ruff.
lint-py:
    nix develop .#default -c ruff check .

# Prints the pinned toolchain versions (rust-toolchain.toml + flake ruff).
toolchain-versions:
    @echo "rust: $$(cargo --version)"
    @echo "ruff: $$(nix develop .#default -c ruff --version)"

# Runs all Rust tests with isolated devfinity-managed test infrastructure
test:
    cargo run --quiet --locked -p devfinity -- run -- cargo test --workspace --locked

# Stable root wrappers for module-owned recipes.
brain-api-route-check:
    just brain brain-api-route-check

brain-language-check:
    just brain brain-language-check

brain-product-matrix:
    just brain brain-product-matrix

chat-device-parity:
    just chat chat-device-parity

chat-electron-check:
    just chat chat-electron-check

chat-electron-package:
    just chat chat-electron-package

chat-history-stress:
    just chat chat-history-stress

chat-reliability-fast report="finitechat/target/hermes-adapter-regressions/report.json":
    just chat chat-reliability-fast {{ quote(report) }}

[positional-arguments]
deploy-lat1-closure artifact_dir *args:
    just nixos deploy-lat1-closure "$@"

[positional-arguments]
deploy-lat3-closure artifact_dir *args:
    just nixos deploy-lat3-closure "$@"

[positional-arguments]
deploy-lat4-closure artifact_dir *args:
    just nixos deploy-lat4-closure "$@"

[positional-arguments]
install-lat4-from-artifact artifact_dir target_host *args:
    just nixos install-lat4-from-artifact "$artifact_dir" "$target_host" "$@"

finite-private-deepseek-contract:
    just computer finite-private-deepseek-contract

finite-private-deepseek-release-contract:
    just computer finite-private-deepseek-release-contract

finite-private-glm53-contract:
    just computer finite-private-glm53-contract

finite-private-glm53-release-contract:
    just computer finite-private-glm53-release-contract

finite-status-contract:
    just infra finite-status-contract

hosted-recovery-contract:
    just infra hosted-recovery-contract

identity-conformance:
    just identity identity-conformance

identity-edge-contract:
    just identity identity-edge-contract

lat1-healthcheck-contract:
    just nixos lat1-healthcheck-contract

lat1-rollout-contract:
    just nixos lat1-rollout-contract

lat1-secret-bootstrap-contract:
    just nixos lat1-secret-bootstrap-contract

lat2-runner-guardrails-contract:
    just nixos lat2-runner-guardrails-contract

lat3-runner-rollout-contract:
    just nixos lat3-runner-rollout-contract

lat4-runner-rollout-contract:
    just nixos lat4-runner-rollout-contract

litestream-recovery-contract:
    just infra litestream-recovery-contract

monitoring-nixos-contract:
    just monitoring monitoring-nixos-contract

nixos-build-lat1-closure rev out_dir="target/lat1-nixos-closure":
    just nixos nixos-build-lat1-closure {{ quote(rev) }} {{ quote(out_dir) }}

nixos-build-lat3-closure rev out_dir="target/lat3-nixos-closure":
    just nixos nixos-build-lat3-closure {{ quote(rev) }} {{ quote(out_dir) }}

nixos-build-lat4-closure rev out_dir="target/lat4-nixos-closure":
    just nixos nixos-build-lat4-closure {{ quote(rev) }} {{ quote(out_dir) }}

runbook-facts-contract:
    just infra runbook-facts-contract

production-deploy-contract:
    just infra production-deploy-contract

runner-host-contract:
    just nixos runner-host-contract

runtime-image-contract:
    just runtime-images runtime-image-contract

stripe-billing-clock:
    just dashboard stripe-billing-clock

stripe-price-contract:
    just dashboard stripe-price-contract

web-check:
    just dashboard web-check
