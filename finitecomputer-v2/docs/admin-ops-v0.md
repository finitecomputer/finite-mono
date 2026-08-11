# Admin Ops v0

Status: active v2 product contract.

Date: 2026-07-03.

## Problem Statement

Finite Private key issuance and limit management required shell access to the
prod box (`finite-saas-core finite-private-friend-key-issue` and friends
against `FC_CORE_DATABASE_URL`), and there was no admin view of provisioned
agent runtimes. Operators need to see every provisioned box, restart or
recover any of them, and manage Finite Private (issue friend keys, rotate
keys, reset burst windows) from the dashboard.

The hard requirement is that admin-ness is enforced by Core, not by the
dashboard. Core validates the standard WorkOS access-token JWT and requires
the configured internal operator `org_id` for every administrator route. Every
mutating admin action writes an audit event with the operator's verified email
as actor.

## Product Flow

1. An Account Auth member of Finite's configured operator organization opens
   `/dashboard/admin`. Non-admins get a 404.
2. The page has three tabs: **Users**, **Invites**, and **Finite Private**.
   Users groups `GET /api/core/v1/admin/runtimes` by owner account and enriches
   each existing account card with the matching grant, usage, assignable 1x/5x
   profile, and all account/project/runtime-scoped keys from the account-centric
   Finite Private admin state. It never asks the operator to correlate separate
   user, grant, and key lists.
3. Row actions Restart and Recover (with a confirm step) create the same
   runtime control requests the owner-scoped buttons create; the runner
   leases and completes them through the unchanged
   `/api/core/v1/runtime-control-requests/*` machinery.
4. The Invites tab owns Launch Code batches. The Finite Private tab is the
   standalone friends-and-family test-key minting surface plus summary metrics.
   Existing account grant/key controls live only on Users cards.
5. Issue and rotate return the raw key exactly once. The dashboard shows it
   once with a copy button and a "you will not see this again" note. The raw
   key lives only in the action response and the page's in-memory state; Core
   stores only the hash and never logs the raw value.
6. Every dashboard server action forwards the WorkOS access token. Core
   independently validates its signature, issuer, client id, expiry, subject,
   verified user record, and exact operator `org_id` before doing anything.

## Route Table

All routes require `require_admin_identity` (validated WorkOS access token with
the exact configured operator organization):

| Method | Route | Action |
| --- | --- | --- |
| GET | `/api/core/v1/admin/runtimes` | Provisioned-boxes overview |
| GET/POST | `/api/core/v1/admin/launch-code-batches` | List metadata or issue one named exact-size batch |
| POST | `/api/core/v1/admin/launch-code-batches/{batch_id}/revoke` | Revoke remaining unredeemed codes |
| POST | `/api/core/v1/admin/projects/{project_id}/runtime/restart` | Restart any project's runtime (owner check skipped) |
| POST | `/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat` | Recover any project's runtime |
| POST | `/api/core/v1/admin/finite-private/friend-keys` | Approve grant for an email and issue a key; returns raw key once |
| POST | `/api/core/v1/admin/finite-private/keys/{key_id}/rotate` | Rotate a key; returns new raw key once |
| POST | `/api/core/v1/admin/finite-private/keys/{key_id}/revoke` | Revoke a key |
| POST | `/api/core/v1/admin/finite-private/grants/{grant_id}/window-reset` | Reset the current burst window |
| POST | `/api/core/v1/admin/finite-private/grants/{grant_id}/limit-profile` | Assign the durable 1x or 5x profile without resetting usage |

The existing admin-authorized
`GET /api/core/v1/finite-private/admin-state` response adds `accounts` and
`profiles` while retaining the flat `grants` and `apiKeys` fields for an
additive mixed-version rollout. Each account includes its verified email,
grant, keys, and exact project/runtime bindings.

## Source Of Truth

Core owns:

- the `FC_WORKOS_OPERATOR_ORG_ID` predicate and all admin authorization decisions
- runtime control requests, whichever surface created them
- Finite Private grant/key state and burst window accounting
- the account-to-grant-to-key correlation and assignable limit-profile catalog
- the admin audit log (`finite_private_admin_audit_events`), which now also
  records runtime admin actions with the admin's email as `actor`

The dashboard owns only the UI gate and adapter code. Its gate is a
convenience: bypassing it still cannot mutate Core, because Core validates the
JWT and operator organization on every call.

The CLI subcommands in `finite-saas-core` remain as the break-glass path and
their help text points at the dashboard admin page.

### `--dry-run`

`--dry-run` runs the real operation against real Core state inside a
transaction that is always rolled back. It therefore needs
`FC_CORE_DATABASE_URL`, exactly like a committing run, and reports what the
command would do to the rows that are actually there.

It previously swapped in an empty in-memory store and never contacted the
database. That made the preview wrong in both directions: the revoke and
window-reset commands failed with "not found" for every input including valid
production IDs, and `reconcile-imports` reported a creation for every record
because nothing existed to update. Treat dry-run output from before
2026-07-24 as unreliable.

A dry run never commits, so it is safe to repeat. It does not run schema
migrations.

## Operator organization

Core requires `FC_WORKOS_OPERATOR_ORG_ID` at startup. The value names Finite's
internal WorkOS organization and is never persisted or inferred as a Core
Customer Organization. Missing, absent-from-token, or different `org_id`
claims fail closed. Administrator authorization never checks role slugs.

## Raw Key Handling

- Core generates raw keys server-side (`fpk_live_` + 64 hex chars) for admin
  issue and rotate, returns them once in the response body, and stores only
  the SHA-based hash.
- Core never logs raw keys; the admin-state and audit endpoints never contain
  them (asserted in tests).
- The dashboard keeps the raw key in `useActionState` memory only, shows it
  once with a copy button and a one-time warning, and never writes it to a
  URL, cookie, or log.

## Weekly Limits Are Future Work

Weekly limits are computed from a rolling window over reservations, not from
a stored counter. There is therefore no weekly reset lever in Admin Ops v0 —
only the burst window reset, matching the `finite-private-window-reset` CLI.
A weekly override/reset needs its own design (probably an explicit
adjustment ledger over reservations) before it can exist anywhere.

## Evaluation Design

Admin Ops v0 is accepted when:

- Core tests prove `require_admin_identity` rejects missing/invalid JWTs,
  unverified or unknown users, missing/different operator organizations, and
  every service credential, while accepting the configured operator org.
- Core tests prove each admin endpoint works for admins and is rejected for
  non-admins.
- Core tests prove profile assignment preserves usage/window state and that
  admin state correlates accounts, grants, keys, projects, and runtimes.
- Core tests prove admin restart/recover skip the owner check but create the
  same control request shape the runner leases and completes through the
  existing endpoints.
- Core tests prove friend-key issue mirrors the CLI (grant approve + key
  issue), the raw key is returned once and never appears in stored state,
  rotate returns a new raw key while the old raw key stops validating, and
  window reset clears only the burst window (weekly rolling usage is
  untouched).
- Core tests prove every mutating admin action records an audit event with
  the admin's email as actor.
- The devfinity-managed `just test` Postgres harness covers the new store
  methods (overview read, admin restart lease round trip, friend key
  issue/rotate/revoke, window reset, audit persistence) against Postgres.
- Dashboard tests cover the admin gate helper, heartbeat-age formatting,
  one-time key display, exact project correlation, owner grouping, curated
  1x/5x profile ordering, and the three-tab single-card information architecture.
- Gates pass: `cargo fmt --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `just test`, and dashboard `npm ci`, `npm run lint`,
  `npm test`, `npm run build`.

## Open Decisions

- Whether Admin Ops should also expose stop/Runtime Retirement (the UI starts
  with restart/recover only). Purge User Data is explicitly not a routine Admin
  Ops control.
- A designed weekly-limit override mechanism.
