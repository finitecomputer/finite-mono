# Direct transport

The agent's own room log, which web chat reaches without MLS. The decision is
[ADR 0013](adr/0013-web-chat-uses-the-agents-direct-room-log.md); the rollout
is `infra/runbooks/direct-chat-cutover.md`.

`finitechat hermes serve` selects it with `FINITECHAT_CHAT_TRANSPORT=direct`
(the default is `mls`). Hermes keeps its loopback contracts unchanged:
`/v1/hermes/*`, `/v1/agentd/*`, `/healthz` and `/readyz`. `/readyz` adds
`transport` and `direct_fold`.

## Room log

The log lives in `$FINITECHAT_HOME/direct/room-log.sqlite3` (WAL,
`synchronous=FULL`). Encrypted attachment blobs live in `direct/blobs/<ciphertext
sha256>`. These are the records a per-person export carries.

| Table | Meaning |
|---|---|
| `direct_meta` | `schema_version`, `origin` (`fold` or `fresh`), `created_at`, `owner_account_id` (the person this log renders for), `fold_cursors` (room → MLS cursor at the fold) |
| `direct_rooms` | `room_id`, `display_name`, `picture`, `read_seq` (the person's read mark), `next_seq` |
| `direct_members` | room members' account ids |
| `direct_events` | `room_id`, `seq`, `message_id`, `sender_account_id`, `sender_device_id`, `timestamp_unix_seconds`, `plaintext`, `origin` (`fold` or `direct`), `idempotency_key` |
| `direct_profiles` | cached `AppProfileSummary` JSON |

**What an event is.** `plaintext` is a JSON `DecryptedApplicationEventV1`: the
same bytes an MLS Device stores after decryption. Folded events keep their
original `seq`, `message_id`, sender and timestamp. New events continue the
room's sequence and get `direct-<hash>` message ids.

**What derives from the events.** Topics are ConversationCreate/Update/Archive.
Chats are SegmentStart plus the namespaced rename/archive/placement events.
Messages are `HermesMessagePayloadV1`, with edits by `edit_of` and attachments
as blob references. Runtime command requests and results ride in the same log,
so they share its ordering and history.

**How it is projected.** `finitechat_core::DirectRoomEngine` projects the events
with the Device's own projection, and renders the hosted device's `AppState`
for any `AppView`. `finitechat_core::room_digest` hashes what a person sees.
Hashes cover topic and chat ids, titles, archive state, placement and every
transcript row. No text is included.

## The fold

The fold runs on the first direct start with no log:
1. It runs `StartRuntime` and syncs until the server has nothing newer.
2. It refuses if any room is still behind the server.
3. It copies every stored message and event (unioned by room, seq and message
   id), each room's members and read mark, and the profiles.

It writes to a staging file and renames it into place. The MLS store is only
read.

**Who the owner is.** The log renders for the room's single human member. A
room shared by several people needs exactly one of them in
`FINITECHAT_OWNER_NPUBS`; otherwise the fold refuses.

**A fresh agent** (no MLS store) gets one room and a Home topic. The owner is
`FINITECHAT_OWNER_NPUBS`; failing that, the first verified signer claims the
room.

**Fold status.** `direct/fold-status.json` records the outcome and the post-fold
digests, so operators can compare them with `finitechat-hosted-device digest`
from the host's `/data` mount.
- A failed fold records `origin: "pending"` with the error, and that start
  serves MLS chat.
- A folded log whose MLS cursor later advanced (a rollback ran MLS chat again)
  is refused the same way. Moving `direct/` aside folds afresh.

## App API

The sidecar serves these loopback routes. Hermes `serve` exposes them as
`/api/plugins/finitechat/v1/app/...` behind native auth.

| Route | |
|---|---|
| `GET /v1/app/info` | `{transport, version, agent_id, agent_account_id, rooms, canonical_room_id, migration}`, unsigned |
| `GET /v1/app/state?<AppView>` | the person's `AppState` |
| `GET /v1/app/updates?<AppView>` | SSE: `event: state`, `id: <rev>`, sent at once, then on every change or every 30 s |
| `POST /v1/app/actions` | an `AppAction` → `AppState`. Covered: open, create topic, start chat, move, rename, archive, sends, load older, mark read, typing |
| `POST /v1/app/new-chat` | `{room_id, topic_id, reason?, intent_key}`; idempotent per intent |
| `POST /v1/app/attachments` | multipart (≤8 files, 32 MiB each, 64 MiB total) → `AppState` |
| `GET /v1/app/attachments/{room}/{message}/{attachment}` | bytes, with `nosniff` and `sandbox` |
| `POST /v1/app/runtime-commands` | the hosted device's runtime-command request → `{request_id, status, body, error}` |
| `GET /v1/app/digest?room_id` | the room's digest |

**Authorization.** Every route except `info` requires
`x-finite-chat-authorization`: base64url (no padding) of a kind-27235 Nostr
event, with empty content, created within ±120 s, and these tags:
- `u` = `finitechat-direct://<FINITE_AGENT_ID><path-and-query>`
- `method`
- `payload` = SHA-256 of the body, for POST only
- `nonce`

The signer must be the log's owner. A signature is accepted once. Text sends
also take the verified requester headers (`x-finite-requester-email`,
`x-finite-sites-requester-assertion`). The hosted device mints these events
with `POST /v1/chat/direct-authorization`.

## Streaming

`FINITECHAT_DIRECT_STREAMING=1` does three things:
- The adapter advertises message editing.
- The config reconciler turns Hermes streaming on.
- The sidecar keeps `running` edits of an earlier message as in-memory drafts,
  which the person's view collapses onto that message.

A streamed reply costs two log rows: its first part and its final edit.

## Operator commands

| Command | |
|---|---|
| `finitechat hermes direct-digest` | the agent's digests |
| `finitechat app digest` | one Device's digests |
| `finitechat-hosted-device digest` | every hosted person's digests |
| `scripts/direct_chat_fold_audit.py` | compares hosted digests with fold-status files |
