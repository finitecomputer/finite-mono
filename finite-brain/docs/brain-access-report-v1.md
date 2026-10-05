# Named Brain access report v1

Status: implemented for
[FIN-122](https://linear.app/finitecomputer/issue/FIN-122) (see "Implementation"
below); production enablement is separate. This contract depends on
[Core identity descriptions v1](../../finitecomputer-v2/docs/brain-identity-descriptions-v1.md).
It replaces PR #1050's naming design while retaining its useful access invariants.

## Report boundary

`GET /v1/brains/{brain_id}/access-report` is authenticated by the calling exact
key using the existing Brain HTTP authorization. Apply Brain's existing admin
rule, including its established Personal Brain semantics; never substitute a
Core account-owner check. Unauthorized requests fail before any source lookup.

`fbrain access list --brain <exact-id>` renders the report; `--json` returns the
same facts. Existing Chat agents use this CLI result through the FiniteBrain
skill. There is no new Chat message protocol or dashboard roster screen.

The full report, including shared account contact, is admin-only in v1. Ordinary
metadata visibility does not confer access to every Folder or private contact.
A later scoped member/Guest view must filter through existing Folder visibility
and an explicit audience policy before adopting this report's descriptions.

## Exact-key inventory

Build one row per canonical public key, using the union of owner/admin/member
authority, Personal Agent relationships, explicit Folder recipients, locally
known Mount participants and current-version Folder Key Grant recipients. Include
removed keys with retained current grants and keys that have never written.
Unknown identity never hides an access row or authorizes removing it.

Do not merge keys because their emails or names match. Shared runtime keys form
one key row with any supported source ambiguity. Human and agent keys stay
separate even when the same account controls them.

For each relevant Folder, distinguish:

| State | Entitlement | Current-version grant |
| --- | --- | --- |
| `ready` | Present | Present |
| `grantMissing` | Present | Absent |
| `revocationIncomplete` | Absent | Present |

Grant presence is not proof of successful decryption. Removal cannot recall
earlier plaintext. Report issuers, delegating actors, origin and recorded times
where supported; say when history is absent or outside the report's scope.

Use the same access predicates as real Folder operations. Do not introduce an
unqualified second permission algorithm. Destination-side reports cannot verify
all authority in an incoming Mount's source Brain. Include known local facts and
name that limitation instead of claiming global completeness.

## Snapshot and lookup sequence

1. Authenticate and check admin authority before reading private report data.
2. Capture access authority, current grants, participation and coverage in one
   bounded SQLite transaction. Preserve an authority fingerprint, not just the
   content sync sequence; membership, offers and grants can change independently.
3. Collect eligible exact keys. Admin-written recipients/grants alone are not
   target-key participation. Include exact-key Brain Invitation acceptance,
   Invite Token redemption, Folder Invitation acceptance, addressed Mount Offer
   controller acceptance and supported authenticated Brain action evidence.
   Automatically added Mount participants do not inherit the controller's proof.
   A read-only key lacking recorded evidence stays in the inventory, with its
   private description withheld; an account-scoped authenticated hosted flow may
   supply the explicit participation evidence without logging every report read.
4. Call the optional private Core batch client for eligible page keys and this
   exact Brain server/ID. Core additionally enforces the account/Brain sharing
   scope. No response or caller-supplied description changes access facts.
5. Recheck caller authority and the fingerprint after the external call. If the
   caller lost authority, deny. If report authority changed, restart within a
   small bounded retry budget or return a conflict; never combine stale access
   with a fresh permission claim. Bind paging cursors to the authority fingerprint
   and return 409 when the next page cannot use the same authority snapshot.

Keep #1050's qualified snapshot, limits, cursor/recheck and evidence behavior as
reference at `1e47c2c5`. Fresh implementation branches start from current main;
they need not import its naming stack, Directory listener, credential or outbound
NIP-05 rechecks. Reuse policy and regression coverage where it avoids a second
authority implementation. Previous qualification is reference evidence, not
proof for the new Core/contact path.

## Description and readable output

Each row has an independent description state: `resolved`, `unknown`,
`ambiguous`, `notShared` or `unavailable`. Resolved fields follow the Core contract:
human/agent kind, supported readable name, shared account email, responsible
account for an agent, independently established human key if available, optional
NIP-05, source and observation time/revision. Do not fabricate absent fields.

Example readable lines using synthetic descriptions:

```text
Ada · agent · responsible account: sam@example.org · member · 4 Folders ready
sam@example.org · human · admin · 4 Folders ready
npub1… · details not shared · Guest · 1 Folder ready
npub1… · identity source unavailable · removed · Private Tasks: revocation incomplete
```

Human names may be absent in Core; a shared account email is an honest readable
fallback. Agent NIP-05 names are not contact mailboxes. Cached public aliases are
separate evidence, labelled with their stored observation time and lack of a
fresh check. Do not make outbound name verification a prerequisite for reporting.

Core not configured, old protocol, timeout or outage leaves every access row
available and marks descriptions unavailable. Reject malformed batches, unknown
response keys, duplicate/conflicting entries, invalid keys and oversized output.
Keep source time separate from the Brain snapshot/check time; do not imply a
cross-database atomic observation or live human-mailbox verification.

No report read writes aliases, account associations, sharing scopes, authority,
grants, content or sync records. Existing authentication replay defenses remain;
this rule concerns product data. No durable private-contact cache in v1.

## Validation and rollout boundary

Qualify the real CLI → Brain → Core boundary with synthetic populated state:
arbitrary-domain humans and agents; replacement keys; several keys per account;
shared keys; explicit read-only Guests; Folder Invitations; Mount Offers; pending,
wrong-key and automatically added recipients; missing grants; demoted Members
and removed keys retaining grants. Test same-email keys remain separate.

Prove arbitrary-key insertion cannot disclose private contacts, an agent cannot
release its account holder's contact, revoked/absent scopes withhold, and owner
transfer requires the successor's sharing scope. Verify denial before lookup,
admin removal/access change during lookup, paging conflicts and source outages.
Compare text/JSON and before/after exports, authority and content sequences.

Use actual base binaries for new CLI/old Brain, old CLI/new Brain, new Brain/old
Core and new Core/old Brain. Prove populated schema migrations, lost-response
retry behavior and empty-target recovery using the relevant complete Recovery
Set. Run focused checks and required monorepo CI in the pinned environment.

Deploy the qualified Core capability first, then optional Brain wiring and CLI.
Describe secret names/locations, deployment digests and disable/rollback steps in
`infra/`; add missing read-only probes to `scripts/finite-status` and run it before
and after rollout. Production deployment/backfill is separately authorized.

Record a read-only report from an ordinary authorized admin session against the
designated Brain after rollout. FIN-159 / PR #1049 owns safe revocation separately;
this report does not rotate, remove or repair anything. If revocation tests need
the report, update that test dependency without importing report mutations or
coupling the two product features.

## Implementation

- Route `GET /v1/brains/{brain_id}/access-report`, version
  `finite-brain-access-report-v1`, admin-only through the existing exact-key
  rule. Each row carries `description` (`state`, optional `reason`, and for
  `resolved` rows `kind`, `displayName`, `accountEmail`, `lifecycle`,
  `responsibleAccount`, `source`) and optional `storedNip05` (`name`,
  `storedAt`). Page coverage is `descriptions` (`checked`, `notConfigured`,
  `notNeeded`, `unavailable`, `unsupported`) with Core's `checkedAt`.
- Keys without recorded participation get `notShared` with reason
  `noParticipation` and are never sent to Core. Participation evidence is
  Invite Token redemption, npub Brain Invitation acceptance, Folder Invitation
  acceptance, addressed Mount Offer acceptance, an accepted authenticated
  Brain record, or an applied Approval by its exact signer
  (`brain_approval_nonces`). Approval targets do not inherit it.
- SCHEMA_V30 adds only indexes for those reads. Grant evidence is the stored
  issuer, time and provenance; signed-audit re-verification, the Identity
  Directory lookup and outbound NIP-05 rechecks are not part of this report.
- Brain configuration: `FINITE_BRAIN_CORE_IDENTITY_URL` and
  `FINITE_BRAIN_CORE_DESCRIPTION_TOKEN` (see `finite-brain/development.md` and
  the [rollout runbook](../../infra/runbooks/brain-identity-descriptions.md)).
  A Core outage, old Core or invalid batch keeps every row and marks
  participating keys `unavailable`; malformed, partial, foreign or
  extra-key batches are refused whole.
- `fbrain access list --brain <id>` renders text and `--json`; `fbrain access
  summary` keeps the older metadata view.
