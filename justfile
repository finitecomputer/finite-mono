mod sites 'finite-sites/justfile'

default:
    just --list-submodules --list

metadata:
    cargo metadata --format-version 1 --no-deps >/dev/null

check:
    cargo check --workspace --locked

fmt:
    cargo fmt --all

test:
    cargo test --workspace --locked

# `just dev-up` or `just dev-up --headless`
dev-up *args:
    cargo run -p devfinity -- up {{ args }}

dev-cleanup:
    cargo run -p devfinity -- cleanup
