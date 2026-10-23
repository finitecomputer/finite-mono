# Brain-scoped operator attribution

V3 adds a bounded operator catch-up for existing roster keys that predate the
normal participation hooks. It adds no user action or access authority. Core
owns the attribution; Brain still owns roles, current grants and participation.
The [v1/v2 contract](brain-identity-descriptions-v1.md) remains in force for
legacy requests. Deployment follows the [identity runbook](../../infra/runbooks/brain-identity-descriptions.md).

## Scope and disclosure

An operator record names one exact configured Brain server, Brain ID and full
public key. It never enters the global account/key association table. A lookup
for a different Brain cannot use it. Four record kinds are supported:

- `managed_agent` binds an existing linked account and WorkOS subject to the
  current, unambiguous runtime key/project evidence. Name, lifecycle and owner
  contact come from current Core facts.
- `account_chat_key` binds the existing linked account and subject to an exact
  hosted key. The operator must first prove the current hosted identity selector
  and its matching `hosted-web` store row; presence in an old store is insufficient.
- `historical_identity` and `invitation_key` carry a dated, printable ASCII
  annotation, at most 120 bytes. Neither claims a current controller, owner email,
  retirement or account association. They cannot cover a current global human
  association or runtime pin.

The reader rechecks bindings on every request. An unlinked or changed account,
conflicting global human association, changed project owner, ambiguous pin or
unsupported runtime lifecycle withholds operator contact and reports a
contact-free conflict. A valid binding with unusable current contact is
`unknown` with `suppressed/contactUnavailable`; no saved contact is substituted.

For participating keys, a normal resolved result wins. Normal `unknown` or
`ambiguous` also suppresses the operator description. Only normal `notShared`
can use a valid scoped record. Binding diagnostics are evaluated even when
normal evidence wins. Operator descriptions never include owner human-key
hints; normal hints are restricted to the participating subset.

## Private v3 protocol

The existing private description endpoint accepts
`finite-core-brain-descriptions-v3`. Its dedicated request adds required
`participatingPublicKeysHex` to the existing scope fields and full page
`publicKeysHex`. The subset must be unique and contained in the page. Limits
remain 100 keys, 64 KiB request, 256 KiB response and the existing lookup budget.

One read-only PostgreSQL `REPEATABLE READ` transaction finds exact active scoped
records, then loads account/pin facts only for the union of those keys and the
participating subset. Keys outside that union return bare `notShared` without
an account/pin lookup. There is no atomic snapshot across Brain and Core;
Brain retains its authority-change recheck around the lookup.

V3 preserves all legacy description states. A resolved operator result has
`source.kind: operatorScopedRecord` and matching `operatorRecord` revision and
evidence time. A used annotation instead has bare legacy `notShared`, an
`operatorAnnotation`, and `operatorRecord.state: used`. Exactly one representation
is permitted. Suppressed/conflict metadata supplies neither operator contact
nor annotation. A normal description can coexist with either diagnostic.

Brain rejects a malformed, partial, extra-key or contradictory batch as a whole.
It forwards the optional annotation and diagnostic outside `description` in the
access-report row. The shared [28-row wire fixture](contracts/brain-identity-descriptions-v3.example.json)
contains synthetic data only.

## Negotiation and old viewers

Brain tries v3 first. Only HTTP 400 with the exact bounded JSON `error` value
`unsupported descriptions version` or `invalid descriptions request body`
allows reconstruction of a legacy request. That request contains only the
participating subset. An empty subset sends no legacy request. V2 can retry v1
only for the version error. At most three attempts share the same deadline;
other statuses, malformed responses and transport errors fail closed.

New Core uses distinct semantic errors for missing/invalid subsets and
unexpected version-specific fields, preventing those mistakes from triggering
legacy reconstruction. V1/v2 responses retain their previous shape and never
read or expose operator records.

Coverage retains its existing states and adds optional `coreProtocolVersion`
and `coreNegotiation` (`direct`, `afterSchemaRejection`, or
`afterVersionRejection`). The version is present only after a validated answer.
On direct v3 failure every attempted page key is unavailable. On legacy success
or failure, nonparticipants retain legacy `notShared/noParticipation` and only
the existing trusted Directory alias policy.

Old fbrain JSON preserves unknown fields; old plugin/dashboard code can ignore
the annotation while keeping the roster. Successful legacy rendering does not
qualify an apply: required admin viewers must support the dated labels first.
Current plugin and dashboard show annotations in the existing Identity cell,
never as public-name aliases or Owner contacts. No screen, column or action is
added. A merged old CLI's first-page version field cannot certify later pages;
qualify every page separately when a roster spans multiple pages.

## Operator transactions and rollback

`finite-saas-core brain-scoped-records --manifest PATH` requires OS root and
defaults to the existing rollback-only dry-run store. It validates the exact
configured server and the operator's existing linked email/WorkOS subject.
Preview executes the binding checks and writes inside a transaction, then rolls
back; it is not a DDL migration and requires the deployed additive schema.

Execution additionally requires `--execute --expected-manifest-sha256 DIGEST`.
Use the semantic digest returned by preview, not a byte hash of whitespace in
the JSON file. Freeze both the file bytes and private evidence separately.
Surrogate row IDs allocated by preview are disposable; review natural keys,
kind, scope and digest. A committed receipt records the actual IDs and complete
afterimages. Manifests and receipts contain private attribution and stay out of
git, routine logs and public review fixtures.

Each apply is one Brain and 1–100 records, atomically. It rejects nonidentical
active records, records identical rows as no-ops, and records its operation
receipt in the same transaction. An exact operation-ID/digest retry returns
the receipt; a changed digest under that ID fails. New operations require a
current application window and non-future evidence time. Evidence freshness
and completeness beyond that are operator preconditions, not inferred by the CLI.

A separate `retire` manifest names `targetOperationId`, with no record list.
It retires only unchanged rows created by that apply, comparing every afterimage
and revision atomically. Existing/no-op rows and later normal observations are
preserved. Any drift refuses the entire retirement. Keep operation receipts and
retired rows for audit. Do not restore a database to undo labels.

Schema 0042 is additive. Both new tables participate in the normal Core backup.
Rehearsal must restore them onto an empty target and prove the reader, replay and
retirement afterward. An older binary must reopen populated tables without
altering them; a source-only compatibility argument is insufficient.

All Core operator CLI invocations use the deployment's existing global CLI
coordination lock and never overlap a Core restart. Scoped preview, apply,
exact replay and retirement connect to the already installed schema without
running startup migrations; missing schema fails closed. That operational lock
is not a new global PostgreSQL advisory lock; the transaction itself locks
affected accounts, evidence and records.

## Verification signals

`scripts/finite-status --brain-identity` reports active/retired scoped counts
and current-binding conflicts without contacts or keys. Missing coverage is
explicit; it cannot be treated as zero conflicts.

`--brain-roster ID` checks the private v3 endpoint with exact audited pages,
their participation subsets and a current audited admin. Its
`evidence.coreDescriptionProtocol` reports served version and checked/used/
conflict counts. It does not downgrade. The credential and contact payload
are discarded. This proves Core only; separately require a real admin report
with checked v3 coverage and the rendered roster. Preserve role/grant/identity
snapshots, Chat old-history/reload/new-message checks and normal health gates.
