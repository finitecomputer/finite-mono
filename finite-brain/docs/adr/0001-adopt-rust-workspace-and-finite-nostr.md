# Brain crate ownership

Brain crates are members of the root Cargo workspace. Core owns domain and
crypto contracts; store owns SQLite state and transactions; server owns HTTP
routes; app is the service binary; CLI owns Working Trees and client operations.
`finite-nostr` supplies reusable Nostr primitives without Brain product policy.
See [development](../../development.md) for the current crate map.
