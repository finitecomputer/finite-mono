# Portable v1 Hardening Readiness

Status: current readiness record for `finitecomputer/finite-brain#14` and
`finitecomputer/finite-brain#22`

This document records the Portable v1 hardening pass for the Rust rewrite. It is
not a production runbook. It is the evidence map for what the current core,
store, server, development Smoke UI, and Product Client prove before the
staging PR.

## Compatibility Fixture Coverage

| Area | Evidence |
| --- | --- |
| Canonical payload serialization | `finite-brain-core::tests::hashes_canonical_spec_vectors`; signed revision, tombstone, and admin payload validation tests |
| Encrypted envelope vector | `finite-brain-core::tests::encrypts_and_opens_folder_object_with_aad`; `rejects_wrong_folder_object_aad`; `rejects_wrong_folder_key_version` |
| Signed records | `validates_signed_create_update_and_move_revisions`; `validates_signed_tombstone`; `validates_signed_admin_access_change` |
| Hash inputs | `rejects_revision_ciphertext_hash_mismatch`; server route coverage for bad ciphertext hash |
| Duplicate sync submit | `finite-brain-store::tests::sync_duplicate_event_returns_existing_sequence`; `finite-brain-server::tests::object_write_duplicate_retry_returns_original_sequence` |
| Base revision conflicts | `sync_rejects_stale_base_revision_and_existing_create`; server object write conflict test coverage |
| Bootstrap/pull visibility | `sync_projection_survives_restart_and_can_rebuild`; `encrypted_vault_export_filters_payloads_grants_and_access_state`; server sync bootstrap/pull route tests |

## Explicit Limits

| Boundary | Current behavior | Evidence |
| --- | --- | --- |
| HTTP auth skew | Portable v1 default is 60 seconds. Tests may pin the clock explicitly. | `server_state_defaults_to_portable_v1_auth_skew`; stale auth route coverage |
| Request body size | Axum `DefaultBodyLimit` caps extracted request bodies at `1 MiB`. | `protected_create_rejects_oversized_request_body` |
| Sync pull page size | Store clamps client `limit` to `MAX_PULL_LIMIT = 1000`. | `sync_pull_caps_large_client_limits` |
| Retention floor | Cursors below the retained floor return `410 Gone` and require rebootstrap. | `sync_cursor_expiry_requires_rebootstrap`; server route coverage |
| Retry/idempotency | Duplicate sync writes are keyed by event id and return the original sequence. Invitation/share accepts are retry-safe only for the same target npub. | duplicate sync, Vault Invitation, Share Link, and Shared Folder tests |
| Protected route rate limits | Authenticated routes are counted per signer, method, and path. Portable v1 defaults to 120 requests per 60 seconds. Deployments can override the in-process bounds through `ServerState::with_rate_limit`. | `protected_routes_enforce_configured_rate_limits` |

## Backup And Restore

The Rust v1 implementation stores authoritative state in SQLite. A complete
development backup is the SQLite database file plus any future external object
store paths once those exist.

Restore requirements:

- Open the restored SQLite database.
- Verify Vault metadata and sync sequence monotonicity.
- Verify current encrypted projection.
- If projection rows are missing or stale, rebuild them from the append log.

Evidence:

- `sync_projection_survives_restart_and_can_rebuild`
- `sqlite_backup_copy_restores_append_log_and_can_rebuild_projection`

## Security Review

| Topic | Current status |
| --- | --- |
| Replay resistance | HTTP auth binds `u`, `method`, optional body payload hash, signature, and 60-second timestamp. The server rejects a reused auth event id within the configured auth window. State-changing sync records are additionally idempotent or conflict-checked by signed event id and base revision. |
| Signer mismatch | Core validates revision, tombstone, and access-change signer fields. Server route tests cover signer mismatch on object writes. |
| Nonce uniqueness | Core encryption generates a fresh 12-byte AES-GCM nonce with `OsRng`. Deterministic nonce helper is public only for fixtures/tests and named as such. |
| NIP-07 trust boundary | Browser/provider signing and NIP-44 encryption remain trusted-client responsibilities. The Product Client owns signer discovery, auth signing, Folder Key Grant opening, and local plaintext indexes. The development Smoke UI never holds production keys and only accepts pasted signed payloads. |
| Smoke UI plaintext/XSS | Smoke UI is development-only and can display encrypted payloads, route errors, and pasted payload bodies. It must not be deployed as a product client or used with production secrets. |
| CORS/cookies | Rust server relies on Nostr auth headers only. It accepts `Authorization`, `X-Nostr-Authorization`, and `X-FiniteBrain-Nostr`. It does not trust cookies for auth. Browser CORS is allowlist-driven and defaults to the configured public base URL origin. |
| Server plaintext search | Server returns `400` for plaintext search because search/indexing must run client-side over decrypted accessible content only. |
| Payload leakage | Secure object routes store encrypted payload JSON and server-visible metadata only. Encrypted export withholds inaccessible object payloads as opaque entries. |

## Error Contract Review

| Case | Stable response | Evidence |
| --- | --- | --- |
| Missing/invalid Nostr auth | `403`, `valid Nostr authorization is required` | `protected_create_rejects_missing_auth` |
| Replayed Nostr auth event | `403`, `replayed Nostr authorization event` | `protected_routes_reject_replayed_auth_events` |
| Stale auth event | `403`, `stale Nostr event timestamp` | `protected_create_rejects_stale_wrong_method_wrong_url_and_wrong_payload_auth` |
| Wrong auth method/url/payload | `403`, method/url/payload mismatch messages | same route test |
| Protected route rate limit exceeded | `429`, `protected route rate limit exceeded` | `protected_routes_enforce_configured_rate_limits` |
| Disallowed CORS preflight origin | `403`, `CORS origin is not allowed` | `cors_preflight_is_allowlist_driven` |
| Oversized body | `413 Payload Too Large` | `protected_create_rejects_oversized_request_body` |
| Vault access missing | `403`, `vault access required` | `metadata_requires_vault_membership` |
| Missing folder/object | `404` for missing current object/folder/vault paths | secure object route tests |
| Missing required grant | `400`, `missing required grant` | store grant transaction tests |
| Setup incomplete | Metadata exposes `setupIncomplete`; finish setup rejects non-empty folders | setup repair/reject tests |
| Conflict | `409`, `sync conflict: ...` | store and server stale base tests |
| Rebootstrap | `410`, `rebootstrap required` | store and server retention tests |
| Invalid payload/malformed record | `400`, invalid JSON/record/hash/signature messages | core/server validation tests |
| Expired or revoked invite/share | unavailable link error, no accept | pending/revoked/expired link tests |

## Dependency Audit

| Dependency | Scope | Why it exists |
| --- | --- | --- |
| `aes-gcm` | core | AES-256-GCM Folder Object encryption/decryption with authenticated AAD. |
| `axum` | app/server | Minimal HTTP server, routing, JSON extraction, body limits, and dev static responses. |
| `base64` | core/server | NIP-19/auth payload handling and encrypted envelope nonce/ciphertext encoding. |
| `finite-nostr` | core/server | Reusable Finite Nostr primitives shared with other repos. |
| `nostr` | core/server | Protocol event types, signatures, tags, NIP-98 kind constants, and key generation in tests. |
| `rusqlite` with `bundled` | store | SQLite persistence from day one with deterministic local builds. |
| `serde` / `serde_json` | core/server/store | Stable DTOs, canonical payload parsing, route JSON, and stored opaque payload JSON. |
| `sha2` | core/server/store | SHA-256 for ciphertext hashes, deterministic ids, auth payload hashes, and backup/export checks. |
| `time` | server/store | RFC3339 server/store timestamps and link timestamp validation. |
| `tokio` | app/server tests | Async runtime for the development HTTP app and route tests. |
| `tower` | server tests | Route-level `oneshot` tests without binding ports. |
| `tempfile` | store/server tests | Isolated SQLite backup/restore and Smoke UI route tests. |
| `unicode-normalization` | core | Portable path/name NFC normalization. |

## End-To-End Smoke Matrix

| Flow | Evidence |
| --- | --- |
| Vault bootstrap | core bootstrap tests; server `POST /_admin/vaults`; Smoke UI create vault control |
| Folder creation | store transactional folder tests; server restricted Folder route; Smoke UI create Folder control |
| Encrypted object write/read/sync pull | core crypto tests; server object create/update/move/delete/pull test; Smoke UI object/sync controls |
| Access grant/removal/rotation | store grant/removal rotation tests; server admin route test |
| Vault Invitation accept | store and server singleton npub-bound invitation tests; Smoke UI invitation controls |
| Share Link accept | store and server singleton npub-scoped Share Link tests; Smoke UI Share Link controls |
| Mounted Folder projection | shared Folder connection store/server tests; Smoke UI mount/connection controls |
| Export/import | encrypted export route/store tests; core OKF export/import/search tests; Product Client OKF import execution tests |
| Locked/setup-needed states | setup incomplete repair tests; Smoke UI setup/error state chips |
| Local development UI | `smoke_ui_serves_static_assets_and_sqlite_flow_works` verifies static assets and SQLite-backed route flow; Product Client route smoke covers `/client`, `/client/config.json`, `/client/app.js`, and `/client/app.css` |

## Residual Pre-Production Risks

- The replay cache and protected-route rate limiter are in-process server
  state. A horizontally scaled production deployment needs a shared edge policy
  or sticky process assumptions documented before public traffic.
- The Smoke UI is intentionally not a production signer/client.
- CORS is allowlist-driven. Staging/production must set the public base URL and
  any separate Product Client origin explicitly rather than using wildcard
  origins.
- OKF import execution remains Product Client owned. The server exposes
  encrypted export and rejects plaintext server search/import.
- Hard-cut v1 does not include legacy route compatibility or a migration bridge
  from the old prototype runtime.
