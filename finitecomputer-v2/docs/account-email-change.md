# Account email change

Status: operator-assisted Core transition and read-only Sites inventory implemented;
full cross-product production qualification is still required. This is not an
Agent ownership transfer or an account merge.

## Contract

An Internal Operator changes the login email of the existing WorkOS User while
preserving that User's ID, the Core User ID, Customer Organization, Projects,
Agent Principal, human hosted Principal, Rooms, history, entitlements, and
Finite Private grants/keys. Google OAuth is supported subject to WorkOS identity
linking and native-email-change eligibility. Enterprise SSO/Directory-managed
accounts follow their IdP instead. The destination must be confirmed and must
not belong to another WorkOS or Core User, even if it appears empty.

WorkOS owns verified login identity. Core owns the account/resource association.
Sites owns its mailbox and native grants. Finite Identity owns the Agent's
public name binding. A login-email change is not an Agent name change, key
rotation, credential migration, or authorization to rewrite arbitrary grants.

No customer addresses, account IDs, private preflight reports, or credentials
belong in this document, fixtures, PRs, or repository files. Use synthetic
`before@example.test` and `after@example.test` in examples. Actual request files
and evidence references stay in the operator's private working area.

## Implemented Core boundary

POST `/api/core/v1/admin/account-email-changes/{action}` requires a verified
WorkOS session in Core's configured internal operator organization. Service,
Runner, ordinary-user, and caller-supplied identity-header credentials cannot
perform these actions. A body contains exactly:

```json
{
  "operationId": "email-change-example-1",
  "userId": "core-user-example",
  "workosUserId": "workos-user-example",
  "expectedEmail": "before@example.test",
  "newEmail": "after@example.test",
  "evidenceReference": "private-review-example-1"
}
```

| Action | Effect |
| --- | --- |
| `preview` | Repeatable-read, read-only Core inventory with collision blockers and outstanding external checks. Does not contact or mutate WorkOS. |
| `prepare` | Freshly verifies the same WorkOS subject still has the verified source email; persists exact intent and operator identity. |
| `complete` | Freshly verifies that same WorkOS subject now has the verified destination email; atomically updates only `users.normalized_email`, `users.updated_at`, and the operation receipt. |
| `cancel` | Requires WorkOS to still/again verify the source email; cancels a pending intent without changing the account. Remains usable if another account has claimed the destination. |

`completed` means **Core completed**, not that Sites, external sessions, or the
customer's real Google login passed acceptance. The API returns the external
checks on every response. It does not call WorkOS mutation/session-revocation
APIs, modify Sites, or dispatch runtime actions. An evidence reference records
an operator's external review; it is not a cryptographic attestation of that
review. No automatic cross-service coordinator is claimed.

An exact retry reuses the recorded intent. Reusing an operation ID with a
changed subject, source/destination, or evidence reference fails. Completion
without preparation fails. A cancelled operation cannot complete. A completed
change can only be reversed through a new, separately reviewed transition;
replaying a historical operation cannot overwrite later account state.

The additive operation table reserves a pending destination against other new
operations. Existing enrollment code does not consult this reservation and can
claim the destination meanwhile; completion rechecks and fails without merging.
Short Core transactions lock the users table against existing writers with a
five-second lock timeout; **no network call runs under that lock**. Ordinary
account linking, onboarding, and billing retain their existing email-conflict
guards, including under pre-change binaries. A stale session cannot implicitly
rename an account back. This preserves the single supported, explicit writer.

## Sites inventory

Run the candidate operator CLI against an existing registry or a scratch copy:

```sh
finitesitesd account-email-preflight --data PRIVATE_REGISTRY_DIRECTORY --request PRIVATE_REQUEST_JSON
```

The request has `old_email` and `new_email`. The command opens `registry.db`
read-only, enables `query_only`, and takes a consistent read transaction. It
does not initialize/migrate a registry, consume tokens, reconcile legacy state,
or create a missing database. A snapshot file must first be copied to scratch
or inspected through `scripts/snapshot-sqlite` per the repository rule.

The report contains source/destination mailbox Principal IDs, owned Project
IDs, authorized native Principal IDs, and row counts for mailbox Principals,
external Principals, shares, email keys/links, login tokens, access requests,
notifications, requester assertions, and legacy name resolutions. It never
prints key material, token hashes, messages, or mailbox values. These are counts
of retained rows, not assertions that every token or grant remains active.
Unknown schemas fail; zero rows do not prove the absence of external services.

A nonempty report needs a product-owned disposition before mutation:

- Mailbox-owned Projects and Authorized Sites Keys must preserve their durable
  relationships, rather than being recreated under an unrelated Principal.
- Shares explicitly granted to a mailbox and historical audit/notification
  records are not owner-email caches. Do not blanket replace them.
- Old mailbox tokens, viewer sessions, queued notifications and ten-minute
  requester assertions need an explicit expiry/revocation disposition.
- A destination Sites mailbox Principal may already exist independently of its
  WorkOS account. Its data is not disposable because Core reports zero agents.
- A managed Agent name that spells like either mailbox remains independent.

The runtime currently pins `fsite/v0.5.3`. Before a Sites write, identify the
actual daemon, registry schema, CLI, and hosted-adapter versions; test the
operation with those versions, not only candidate binaries. The preflight is
implemented here; a general Sites mailbox-ownership mutation is deliberately
not inferred from a Core email change.

## Requester metadata and stale sessions

Dashboard Sites requester assertions now obtain email via a fresh Core `/me`
request using the same WorkOS subject, with `cache: no-store` and a bounded
five-second request. Core independently checks the current verified WorkOS
record and its account link. The dashboard never falls back to the browser's
old email if Core fails or returns another subject. Failure omits Sites
requester context while allowing the existing chat-send path to continue.
This adds one Core lookup when creating Sites requester context for a text
send; it does not change Chat identity, Room state, or the Agent ledger.

Existing Core uses the same `/me` response, so the new dashboard works with
that pre-change API. Old dashboard sessions can still mint source-email
assertions until refreshed; deploy the reader fix, refresh/revoke sessions,
and account for outstanding assertions before authorizing a customer cutover.
Do not weaken the Agent ledger or re-run owner claim to refresh an email.

## Operator sequence and recovery boundary

1. Read-only preflight: confirm the exact destination; inspect both WorkOS Users,
   their Google/enterprise identities and Core resources; inspect hosted state,
   Sites, Brain, billing contact details, and agent connected accounts. Stop on
   collisions. A separate account with no Projects still needs a preservation
   decision; never delete it or move its email to a placeholder automatically.
2. Reproduce and qualify on synthetic existing accounts with the deployed
   version mix. Keep the same keys, IDs, history, and unrelated grants. Prove a
   real Google login returns the original WorkOS ID, not merely a mocked email.
3. Prepare an exact coordinated recovery boundary: private WorkOS identity
   metadata, Core rows/receipt, and any product-owned changed state. Retain
   restorable Chat/hosted/runtime backups; do not clone or replace them just for
   an email change. Define what to do if the provider changes but Core cannot
   commit. Revoked OAuth credentials may require interactive reauthentication;
   restoring a database row alone is not full login rollback.
4. Obtain explicit production authorization for the exact case. Run
   `scripts/finite-status` before/after any rollout. Deploy only reviewed,
   qualified components. Prepare Core intent, execute the separately authorized
   WorkOS/native verification and product-owned steps, refresh sessions, and
   complete Core. Inspect intent/current provider state after any uncertain
   response rather than blindly replaying provider mutations.
5. Accept only after Google sign-in, historical chat, a fresh message/reply,
   Connections, Brain, existing private Sites, new publishing/sharing, and billing
   continuity pass. A bot restart is conditional on an observed cache. Updating
   bot memories is housekeeping, not an authorization mechanism. Google login
   does not change the bot's Gmail/Drive credentials.

## Validation and remaining qualification

Automated tests cover actual Postgres identity/resource preservation, duplicate
and changed-operation replay, provider mismatch, an intervening enrollment
writer, cancellation after a destination collision, rollback-only dry-run,
restart, operator-only HTTP authorization and fresh verified target identity.
Sites tests compare registry bytes before/after read-only inventory and prove
missing/unknown schemas are not created or migrated. Dashboard tests exercise
stale-session/new-Core email, unavailable Core, and wrong-subject responses.

These tests are not a real Google OAuth acceptance test, full encrypted-history
recovery proof, or completed Sites migration. Those remain production gates.
The follow-up Agent transfer feature needs its own scoped grant/history and
runtime-authorization transition; it must not reuse whole-account email change
as a workaround for moving one Agent.

Provider references:
- https://workos.com/docs/authkit/email-changes
- https://workos.com/docs/authkit/identity-linking
