# Commands for controlling devfinity (local dev harness)
mod dev 'devfinity/justfile'
# Commands for controlling finite sites
mod sites 'finite-sites/justfile'

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
