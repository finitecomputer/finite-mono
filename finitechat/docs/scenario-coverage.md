# Chat test coverage

Run from the root pinned development environment:

```sh
cargo test -p finitechat-delivery
cargo test -p finitechat-server --test http_conformance
cargo test -p finitechat-mls
cargo test -p finitechat-client
cargo test -p finitechat-server --test http_routes
cargo test -p finitechat-server --test http_persistence
```

The delivery conformance suite tests ordering and transport behavior. It
cannot prove MLS credentials, encryption, membership, or retained OpenMLS state;
those require the real MLS and client suites.

SQLite tests cover transactions, exact retries, typed rejection persistence,
concurrency, reopen and crash boundaries. HTTP tests cover authentication,
bounds, routes, opaque payload transport and durable server state.

Hermes adapter and container tests live under `integrations/hermes/tests/` and
`tests/container/`; use the current root CI recipes for their invocation.
Hosted Web Device tests cover account/Device ownership and durable room state.

For persisted-state or protocol changes, include an existing-state fixture
written by a supported older writer. An all-candidate run cannot establish
mixed-version compatibility. Recovery tests must reopen the same Recovery Set
on an empty target, with independently available decryption authority.
