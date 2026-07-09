# Products consume identity over HTTP

Finite products consume the Identity Authority through its HTTP Identity Contract in production. Rust crates may hold shared domain types, client helpers, and the internal engine/store used by `finite-identityd` and tests, but products should not directly own or mutate identity storage because that would blur Finite Identity's ownership of Principal Resolution and NIP-05 state.
