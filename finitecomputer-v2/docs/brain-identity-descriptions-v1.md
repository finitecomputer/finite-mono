# Brain identity descriptions v1

Status: draft implementation contract for FIN-122. This document defines the
replacement for PR #1050's Directory naming integration. The endpoint, schema,
hosted hook and rollout described here are not implemented by this commit.
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
| NIP-05 | Published name-to-key binding or separately recorded binding evidence | Optional public name; not necessarily a mailbox or owner email |

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

## Private contact disclosure

An admin can add an arbitrary key to a Brain. That action does not authorize
reverse discovery of private account information. Autonomous agent participation
also does not authorize publishing its account holder's contact.

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

## Private batch boundary

Proposed route: `POST /api/core/internal/v1/brain-identity-descriptions` on a
service-owned private router. Freeze the path and protocol in route tests before
runtime implementation. Caddy must not maintain a second per-route allowlist.

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
