# Migration validation evidence

Recent archive imports, an additive legacy-history supplement, and a two-source
consolidation completed on existing production Runtime implementations. A further
single-origin import also completed. Its recurring connection conflicts ceased
after owner-reported source shutdown; operator-side verification passed, with
human delivery testing explicitly deferred. This record summarizes the retained
execution evidence. It does not authorize another
cutover, introduce a migration framework, or claim current fleet-wide health.

## Publication boundary

This document records migration behavior and verification outcomes. Account and
Agent names, contact details, conversation subjects or excerpts, private file
names, archive timestamps, per-account activity counts, storage locations,
identity/provider handles, credentials, and recovery hashes remain in protected
operator records outside Git. Even a content hash or an exact activity count can
link an anonymized record back to a particular export.

The private evidence register binds each result to the accepted source cutoff,
exact owner and destination, installed implementation, authorization, source and
target inventories, conversion maps, recovery copies, and verification reports.
Reviewers needing to inspect those bindings must use an authorized private
channel; they should not attach raw evidence to this PR or its review comments.

## Completed work and evidence

| Work | Evidence checked | Outcome and limit |
| --- | --- | --- |
| Archive imports | Source inventories and hashes; archive traversal; SQLite checks on scratch copies; Git bundle verification; transcript/event accounting; native history search; approved profile readers | Completed. Every accepted source message was accounted for. One earlier converter retained explicit repeated branch context; later segmentation retained each source event once with native parent links. Source capture limitations remain part of acceptance. |
| Additive legacy-history supplement | Native export and source provenance; history and file overlap checks; preservation of all preexisting imported sessions and unrelated state; bounded memory updates; live Chat reads of the supplement | Completed. Identical files were not installed twice. Historical metadata remained identified as metadata. This was a point-in-time supplement, not continuous synchronization; later source activity and unmapped temporary attachments are outside its guarantee. |
| Two-source consolidation | Frozen legacy source recovery; comparison with the rehearsed export, facts, persona, permissions and portable files; distinct history namespaces; installed-version import; all message fields and parent links; native memory/search readers | Completed. Both histories, structured facts and portable files were installed while preserving the destination's existing Chat and identity. Unsupported source events and inactive instructions remain archived rather than replayed. |
| Subsequent connection transfers | Source-consumer fencing; target bot identity check; native configuration and pairing readers; preserved approved principal; rejection of an unrelated principal; post-transfer connection state and polling-conflict inspection | Later transfers passed these checks. A source with other active connections had only its transferred adapter disabled. Human message/reply tests were not observed at those checkpoints. An earlier, separate connection request remains blocked on source access. |
| Further single-origin import | Source message/event accounting; branch segmentation and parent links; actual installed-version import and idempotence; native profile and search readers; exact stopped-target candidate comparison; live Chat and portable-file reads | Data import completed. Existing destination Chat and identity were preserved. Explicit source DM authorization was retained; native adapter tests accepted the approved sender and rejected an unrelated sender and group traffic. After owner-reported source shutdown, polling remained free of new conflicts and current gateway/authentication checks passed. Operator-side handoff checks are complete; the human Telegram roundtrip is deferred and has not been observed. |

The earlier checkpoint that reported competing Telegram consumers is superseded
for the subsequently transferred connection: its source adapter was disconnected
before target activation, and no polling conflict was found in the inspected
post-transfer logs. A Connected label or successful identity API call alone is
not proof of end-to-end delivery. The further single-origin import is a separate
checkpoint: its earlier recurring conflicts were subsequently followed by the
post-shutdown target checks described below. Cold-start handling may discard
pending
Telegram updates; no archival guarantee is made for the handoff interval.

## Compatibility and installation checks

Preparation used the existing import API and the actual installed Hermes
implementation. Local SQLite differences were recorded privately and covered by
separate installed-target checks. Rehearsal alone was not treated as proof of a
production migration.

- Compared message content, roles, timestamps, tool links, reasoning, supported
  display metadata, and parent-session ancestry against accepted source records.
- Verified duplicate and oversized-payload rejection, repeat-import behavior,
  SQLite integrity and foreign keys, and source-specific native history reads.
- Rebuilt final candidates from the actual stopped destination backup. Compared
  preexisting session exports and protected filesystem state before installation.
- Used normal Core lifecycle operations for destination stop and resume. Verified
  the authoritative target binding, stopped compute, and absence of writers;
  no manual container stop substituted for lifecycle control.
- Limited installation to reviewed history, profile, memory, connection and new
  workspace paths. Existing Finite identity and Chat state were preserved.
- Explicitly handled a closed database's empty WAL and stale shared-memory index.
  The scoped installer was tested to reject nonempty WAL or an unexpected
  identity change before mutation.
- Verified the installed state against the candidate, then exercised native
  search, profile/file readers and real Chat after restart. Existing Chat
  identifiers and prior conversations survived alongside new replies.

Historical conversations are Hermes history and retained archives. They were
not forged into the destination's Finite Chat Rooms. Source schedules, queues,
external publishing, old identities and unrelated integrations were not
implicitly activated by importing files.

## Recovery evidence and boundaries

Source and full pre-install destination recovery copies were retained separately
from the live Runtime. Empty-directory restores compared the required regular
bytes, modes, ownership and link metadata for the applicable recovery boundary.
For the further source export, every regular file was independently reread after
restore and link records were verified. That source scratch restore used private,
inert permissions; original ownership and mode metadata remain in the archive.
This proves source bytes and links, not an executable restoration of the old host.
The destination and merged-state restores retained and compared their filesystem
metadata. Archive copies were checked by hash
across hosts. Snapshot databases were inspected through scratch copies.

Earlier imports and the additive supplement also received full post-import
backup/restore checks that included the verification conversations recorded
before capture. The most recent consolidation has a complete, independently
copied merged-state archive whose empty restore matches the installed state
immediately before resumption. It does **not** include messages written later.
The further single-origin import likewise has independently copied source,
pre-install destination and merged-state recovery sets with empty-target restore
proofs. Its merged checkpoint predates live verification replies and does not
establish that the external handoff completed.
A subsequent connection handoff has a separate restored configuration/approval
checkpoint; its earlier full-data backup must not be described as containing
that later connection state.

These are bounded recovery proofs, not evidence that every archived source
application was captured consistently or that a new scheduled-backup policy
exists. Reported optional snapshot failures and metadata limitations remain in
the private source acceptance record. Checksums do not establish a frozen source.

Rollback after resumption must first preserve and reconcile new writes. Never
restore an old Chat database over newer messages. Fence the target consumer
before returning an external connection to a source. Source volumes and recovery
copies remain retained; deletion and source retirement are separate operations.

## Further connection follow-up

The earlier checkpoint showed recurring Telegram `getUpdates` conflicts after
initial connection and reconnects. Destination process inspection found one
gateway process; the competing consumer's host was not established. Source
local-name resolution and available Tailscale inventories did not establish
source access. An automatically restarting source service in the export was a
possible explanation, not a verified diagnosis of the live competing process.

The owner later reported the old source offline. Follow-up target evidence showed
a healthy polling confirmation after the last conflict and a sustained interval
without another conflict. The running gateway had a fresh heartbeat, an event-loop
liveness indication and established Telegram TLS connections. Bot identity was
revalidated and webhook configuration was absent, consistent with polling mode.
No restart or production configuration change was needed for this follow-up.

The Runtime implementation had independently advanced since the import. The exact
destination binding and readiness were revalidated, and authorization was tested
against the current installed adapter: approved DM accepted, unrelated DM and
group traffic rejected, unrelated pairing absent. This follow-up is not a new
proof of every import compatibility edge on the updated implementation.

Operator-side handoff checks are complete at this checkpoint. The requester is
not in direct contact with the end user, so the real inbound-message/outbound-reply
test is explicitly deferred. No test message was sent to the end user. Gateway
liveness, TLS connections, an identity API response and quiet conflict logs do not
substitute for end-to-end delivery evidence. Source shutdown remains owner-reported;
no independent inspection of the source process was obtained.

## Remaining checks

- Observe a real inbound message and outbound reply from the already-approved
  Telegram account for the recent transfers. Group behavior was not exercised
  end to end; the further import has native adapter-level rejection tests only.
- For the further import, observe an approved-user roundtrip when direct user
  testing becomes available. Confirm receipt in the destination Runtime; a reply
  from another consumer must not be mistaken for destination delivery.
- Resolve the earlier source-access prerequisite before attempting that separate
  connection handoff.
- Use supported authorization flows for unrelated integrations; account login
  alone does not grant access to an external product's data.
- The canonical `scripts/finite-status` checks recorded Chat and recovery green
  around the recent cutovers, with unrelated fleet/service exceptions explicitly
  retained. The most recent Runtime was healthy and Chat replied even while the
  browser sidebar retained an older offline label. For the further import, a fresh
  dashboard view subsequently showed online; canonical Chat, recovery and rollout
  checks passed while preexisting fleet exceptions and unknown host-health results
  remained explicit. No fleet-wide green result
  or unrelated production repair is claimed.

Validation for this documentation update: checked the summary against retained
execution, restore and handoff reports; checked relative links and whitespace;
reviewed the proposed file, commit message and PR text for identifying details.
No product code, runtime image or production setting changes are part of this PR.

Related contract: [legacy Hermes migration contract](../../finitecomputer-v2/docs/legacy-hermes-migration-contract.md).
