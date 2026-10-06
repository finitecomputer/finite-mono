# Brain identity descriptions v1

Status: implemented in Core and the dashboard for FIN-122 (see
"Implementation" below); production enablement is separate. This document
defines the replacement for PR #1050's Directory naming integration.
Product scope, priorities and delivery tracking remain in
[FIN-122](https://linear.app/finitecomputer/issue/FIN-122).

## Ownership of facts

Core describes an exact public key. Brain decides that key's access. A name,
email, account relationship or agent ownership never creates Brain membership,
an admin role, Folder entitlement or a Folder Key Grant.

| Fact | Authoritative source | Meaning |
| --- | --- | --- |
| Account contact | Core `users.normalized_email`, linked through verified WorkOS identity | Current account email, in any domain; not perpetual mailbox verification |
| Agent name | Core `projects.display_name` | Hosted project's name |
| Agent key | Authenticated `agent_runtimes.health_reporting_npub`, consistent active project/runtime link | Exact recorded runtime principal; not inferred from a slug |
| Responsible account | Current `projects.owner_user_id` | Account responsible for the hosted agent; not proof of legal ownership or decryption |
| Human key | New Core association observed through the trusted hosted identity path | Existing human key associated with a stable account ID |
| NIP-05 | Published name-to-key binding or separately recorded binding evidence | Optional public name; not necessarily a mailbox or owner email. Core v1 returns none: `projects.agent_email` is reserved before publication and is not binding evidence. Brain shows its own dated aliases separately |

Do not use `agent_creation_requests.owner_chat_account_id` as a current reverse
registry. It is launch-time input. Do not query the operator runtime inventory
as the product API, scan sealed Chat bindings, or derive account IDs from email.

Human names are optional: use an existing exact-key profile only with its stated
source. Without a supported human display name, render the shared account email
as the readable fallback. Never manufacture a person's name from an email slug.

## Trusted hosted observation

The dashboard backend already verifies an account and can call Hosted Device
`identifyMember`. That operation loads the existing identity and Chat store;
missing or inconsistent state fails without creating either.

Add a narrowly authenticated Core ingestion operation for this trusted backend.
Its account context must be verified through Core's existing WorkOS account
authority. The backend obtains the key from `identifyMember` for that subject,
not from browser input. Bind the observation to subject, exact key, issuer,
audience, freshness and a request nonce. Reject mismatches and replays; retries
with the same operation ID return the original outcome. A trusted service
observation is described as such, not as an independent cryptographic ownership
certificate. It must not introduce a generic signing operation or broaden the
existing hosted signer's allowed origins.

Add an indexed account/principal association containing stable Core user ID,
canonical key, source, observation time, active status and evidence revision.
Keep retired associations as history. Conflicting active claims produce an
explicit conflict; they do not replace a key or choose the latest claim. Email
changes update contact information for the same account, not the association.

The hook runs after a successful ordinary hosted Brain action. Its bounded
failure/retry path must not block Chat startup, onboarding, signing, normal Brain
access or the original action's successful result. A report read does not invoke
this writer. Older components continue without registration.

Ingest route:
`POST /api/core/internal/v1/brain-account-observations`. Require both the current
WorkOS bearer from server-only `AccountAuthContext.accessToken` and a dedicated
hosted-observation service credential. Core uses `require_verified_identity`;
caller-supplied account identity/email headers remain forbidden. The credential
identifies the trusted dashboard issuer, rather than authorizing a browser to
submit observations directly.

The versioned body contains an operation ID, configured Brain server identity,
exact Brain ID, observation time, optional observed human public key, exact
participating public key and action kind (`humanHostedAction` or
`ownedAgentHostedAction`). The trusted backend derives keys from the Hosted Device
response or exact Core-owned agent pin and verifies the Brain action's response.
For a human action, participating key must equal the observed human key. For an
owned-agent action, Core additionally verifies the unique current project owner
matches the bearer account. No actor, email or ownership field from the browser
can supply this evidence; a model message is not a receipt.

Start with a 60-second observation window plus 30 seconds of clock skew. Atomically
store the association when present, scoped sharing and operation outcome. An
exact same-account/same-payload operation retry is idempotent; different payload,
issuer or account reuse is refused. Return `recorded` or `unchanged`; conflicting
active key associations return 409 without overwriting them. Keep the stored
operation receipt free of bearer/service credentials. This is one trusted-service
operation, with no challenge route, borrowed Brain proof URL or new signing API.

## Private contact disclosure

An admin can add an arbitrary key to a Brain. That action does not authorize
reverse discovery of private account information. Under v1, autonomous agent
participation also does not authorize publishing its account holder's contact;
v2 accepts that risk (see "Disclosure without a sharing scope").

The disclosure scope, its revocation and the successor-scope rule below apply to
v1 requests only. A v2 request applies no scope, so revoking one does not
withhold contact from a v2 answer, and v2 has no per-account opt-out. Under v2
the description credential describes any linked key Brain sends for any Brain
ID; Brain's participation rule is the only gate.

Core owns one disclosure scope per stable account, exact Brain server identity
and exact Brain ID. A trusted account-authenticated hosted action establishes it
automatically after the backend verifies actual success against that Brain.
Issuing a signature, accepting a browser claim of success, or receiving an agent
message about its owner is insufficient.

The user-facing action must explain that the account's contact and responsibility
for its participating agents are visible to this Brain's administrators. This is
one account/Brain scope, with issuer, time, revocation and revision; it requires
no manual roster labels or per-agent confirmation. It covers that account's own
established key associations and its currently owned participating agent keys.
An account-authorized operation with its exact owned agent may qualify without
adding the human as a Brain member.

Brain owns caller authorization and exact-key participation eligibility. Core
owns the disclosure check. A scoped read credential never bypasses either rule.
Check current responsible account at lookup time: a former owner's scope cannot
release a successor's contact after transfer. A revoked scope withholds private
fields immediately on the next lookup. Historical participation may identify a
removed key with retained grants while its account scope remains valid.

For keys outside disclosure scope, return the same `notShared` state regardless
of whether an internal account match exists. Do not expose account-existence
information through different private errors. Existing public NIP-05 evidence
can still be shown separately, with its source and observation time.

## Disclosure without a sharing scope (v2, FIN-166)

Version `finite-core-brain-identity-descriptions-v2` keeps the v1 request and
response shape and drops the per-Brain sharing scope. Brain sends only keys
that themselves have recorded participation in that Brain, so participation
alone decides which keys Core describes. The linked-account, conflict,
ambiguity, lifecycle and current-owner rules are unchanged. Core keeps serving
v1 with the v1 policy, so an older Brain discloses nothing new. Brain asks v2
first and asks v1 only when Core answers exactly 400
`{"error": "unsupported descriptions version"}`.

Accepted risk (decision recorded in FIN-166, 2026-10-05): any participation by
an Agent key in a Brain releases the Agent's name and its owner's account email
to that Brain's admins. That includes participation the owner did not request:
a non-owner's request in chat, an "Agent instruction" in an invite email,
injected content, or the Agent's automatic Folder Key writes. Participation by
a hosted human key releases that human's email. An admin still cannot learn who
owns a key only by adding it to their Brain.

Owner human keys: v2 lists in `responsibleAccount.humanPublicKeysHex` only the
owner's keys that are in the same request. Brain also drops listed keys it did
not ask about, whichever version answered.

Ownership transfer: v2 releases the current owner because an Agent key acted in
the past. That is sound only while a project's owner never changes. Core has no
writer that changes `projects.owner_user_id`, and a test fails if one is added.
A transfer writer must first stop participation recorded before the transfer
from releasing the new owner.

## Private batch boundary

Route: `POST /api/core/internal/v1/brain-identity-descriptions` on a
service-owned private router. Route tests freeze the path and protocol. Caddy must not maintain a second per-route allowlist.

Request: protocol version, configured Brain server identity, exact Brain ID,
calling admin key for audit, and canonical full target public keys supplied by
the authorized Brain report. Accept no email, prefix, name, wildcard or fuzzy
search. Start with at most 100 distinct keys and a 64 KiB request body; reject
oversized requests without partial disclosure. Bound database fanout and return
size; set a two-second lookup budget. Publish the final measured limits in tests.

Return exactly one result per requested key with its echoed canonical key:

- `resolved`: kind (`human` or `agent`), supported display name, optional account
  email, optional responsible-account description and independently known human
  key, optional NIP-05, source kinds, observation times and source revision.
- `unknown`: a permitted subject has no supported association.
- `ambiguous`: conflicting authoritative associations; no selected owner/contact.
- `notShared`: private description is not available for this Brain's audience.

Endpoint failure or unsupported protocol is a source-level `unavailable` result
in Brain. Do not turn outages into `unknown`, permission denial or empty rosters.
No response includes keys' secrets, wrapped Folder keys, runtime endpoints,
fleet controls, billing facts or a global account list.

Use a dedicated read-only Brain credential, distinct from the trusted hosted
ingest credential. Bind each to its service and allowed server instance. Use the
private service network or authenticated TLS. No operator/fleet token goes to
Brain; no service token goes to `fbrain`, browsers or Agent Runtimes. Document
credential names and locations only. Redact contact details from routine logs.

## Source and time rules

Query all exact runtime-pin matches. Require active project/runtime links to
agree with `runtime.project_id`. Deduplicate runtime incarnations only under an
explicit lifecycle rule. Different projects or accounts sharing a key are
ambiguous even if names match. Stopped/offline runtimes retain key evidence.
An unambiguous retired agent remains described as `retired`, using its recorded
project/responsible-account facts with their source time; it is not represented
as an active runtime. Null pins, inconsistent links and unresolved relocation
records do not justify guessing. Include lifecycle state in resolved agent rows.

Join current account contact and project owner in one bounded source snapshot.
Return source observation time/revision separately from Brain's access snapshot.
There is no atomic transaction across Core Postgres and Brain SQLite. No durable
private-contact cache is needed in v1; refresh queries the source again.

## Compatibility and recovery gates

All new tables/indexes are additive. Record actual migration numbers when written
and list every writer/reader. No Chat key, sealed binding, history file or Brain
grant schema is rewritten. New readers tolerate absent observations/scopes.

Qualification must cover real base binaries with the new schema, old dashboard
with new Core, new hosted hook with old/unavailable Core, and mixed Brain/Core
versions. Test arbitrary-domain email, forged subject/key/email, replay and lost
responses, missing/half-present Device state with no mint, duplicate pins, email
change, stopped/retired/relocated agents and ownership transfer.

Restore the complete relevant Recovery Set onto an empty target: Core Postgres
including associations/scopes, Brain SQLite including authority/grants, and the
unchanged existing hosted identity/history recovery material. A restored index
is not a restored signing key. Rollback disables the optional lookup/hook and
returns to proven older binaries while preserving additive metadata.

Legacy coverage must be measured. An explicit load-only association backfill may
be proposed with a dry run, conflict report, backup and rollback; it never mints
keys or invents disclosure scopes. Production backfill and deployment require
separate authorization. Missing legacy/native evidence remains explicit until
an authenticated flow establishes it. Universal identification is not claimed.

## Implementation

Core (`finite-saas-core`):

- Migration `0038_brain_identity_descriptions.sql` adds
  `account_brain_principals`, `account_brain_sharing_scopes`,
  `brain_account_observation_receipts` and the partial index
  `agent_runtimes_health_reporting_npub`. Writer: the observation route only.
  Readers: the description route only. No existing row is rewritten.
- Both routes are served only by `api::brain_identity_router` on the optional
  listener `FC_CORE_BRAIN_IDENTITY_BIND`, never by the account or runtime
  routers. Configuration: `FC_CORE_BRAIN_IDENTITY_BIND` (loopback or private
  address with an explicit port; wildcard and public addresses are refused),
  `FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER` (the one canonical Brain origin),
  `FC_CORE_BRAIN_OBSERVATION_TOKEN` and `FC_CORE_BRAIN_DESCRIPTION_TOKEN`.
  All four or none. Each token must differ from the other and from every
  existing Core credential. Partial or invalid configuration, an address
  used by a mandatory listener, or a bind failure disables only this
  feature, with a warning naming variables but not values. Mandatory
  listeners bind first.
- Brain server identity is exactly `https://host[:port]` in lowercase with no
  path or trailing slash; local development may use
  `http://127.0.0.1:<port>` or `http://localhost:<port>`. Anything else is
  refused, not normalised.
- Observation: header `x-finite-brain-observation-credential` plus the
  account's WorkOS bearer in `Authorization`. Core resolves an existing
  linked account by verified WorkOS id, read-only; an unknown or pending
  account gets 403 `account_not_linked` and nothing is enrolled. One
  transaction, serialized per account by a row lock on that account, stores the
  association, scope and receipt. Errors: 400 `observation_expired`, 409
  `operation_id_reused`, `key_associated_elsewhere`,
  `key_conflicts_with_agent_record`, 403 `agent_not_owned`.
- Descriptions: header `x-finite-brain-description-credential` only. One
  REPEATABLE READ, read-only transaction with a 2-second SQL statement
  timeout. Source fields are length-checked in SQL (email 254 bytes, display
  name 200 bytes); oversized or control-character values make the row
  `unknown` and are never truncated. Responses over 256 KiB fail with 503.
  Logs carry the Brain id, the requesting admin key and counts, never contact.
- Disclosure: `resolved` needs a linked responsible account (the current
  project owner for agents) and, under v1 only, an active scope for it.
  `ambiguous` is stated only when every implicated account is linked and,
  under v1, shared with this Brain. The response `version` echoes the request.
  Missing projects or foreign active links are `notShared`. An inactive
  sibling without completed retirement makes the agent `unknown`.
  `responsibleAccount.humanPublicKeysHex` lists at most 8 of the owner's
  associated keys (under v2, only keys in the request) and excludes any key
  that is also pinned as an agent.

Dashboard: `src/lib/brain-identity-observation.ts`, called through `after()`
from `POST /api/brain/invitations/accept` once the Brain server returns an
acceptance by the exact hosted key, and from `POST /api/brain/approvals/approve`
once the Brain server applies a delegation-grant approval signed for that
exact Brain (how existing admins qualify). Both cards tell the user that the
action lets the Brain's admins see their account email and that they are
responsible for their agents there, and send `shareAccountContact: true` with
that text. A request without it (a tab loaded before the text existed) still
joins or approves but records no sharing. It loads the key with Hosted Device
`identifyMember` (no mint), retries a lost response once with the same
operation id, and never changes the join result. Configuration:
`FC_CORE_BRAIN_IDENTITY_URL` and `FC_CORE_BRAIN_OBSERVATION_TOKEN`; the Brain
server identity it sends is `FC_BRAIN_PUBLIC_ORIGIN` (falling back to the
upstream origin), which must equal `FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER`.

Not in v1: scope revocation has no product writer (an operator can set
`revoked_at`), there is no owner-transfer writer, and existing hosted humans
are described only after their next qualifying action.
