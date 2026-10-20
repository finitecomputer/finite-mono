# ADR 0013: Web chat uses the agent's direct room log

Status: proposed 2026-10-13 (FIN-182); accepted when the cutover ships

Web chat stops using the Hosted Web Device as a server-held MLS client.
Instead, the person's chat device moves into the agent, and the encryption
between the two is removed.
- **The log.** Each agent keeps one plaintext room log. It uses the same
  application-event format and is projected by the same finitechat-core
  `ChatProjectionState`, so topics, chats, order, archive state and transcripts
  come out as they did on the Device.
- **How the dashboard reaches it.** Through the existing hosted-Hermes ingress
  and the `finitechat` Hermes dashboard plugin.
- **Contracts.** The wire contract and storage are in
  [direct-transport.md](../direct-transport.md).

This supersedes [ADR 0011](0011-hosted-web-chat-uses-a-revocable-device.md)
and [ADR 0012](0012-hosted-agent-room-binding.md) for web chat. The Hosted Web
Device stays the custodian of each person's account key. It still signs Brain
and Sites identity assertions, and now also signs every direct chat request.

## Why

The MLS path chained four authorities for one conversation:
- the hosted device's decrypted store;
- the chat server's ciphertext;
- the agent's decrypted store;
- Hermes' sessions.

Its failure modes (room quarantine, behind-server cursors, sync stalls, and an
agent that exits when the chat bridge cannot reach the server) produced most
chat incidents. Yet it bought no confidentiality from Finite: the operator
runs both ends.

The direct log keeps one chat authority per agent, on the agent's durable home
beside Hermes' own state.

## Consequences

- **Authenticity is preserved.** Every viewer request carries a kind-27235 Nostr
  event signed with the person's existing account key, which the hosted device
  holds. It is bound to the agent, method, path, body and a nonce, and the bridge
  accepts it once. Code inside the agent container therefore still cannot speak
  as its owner, as MLS sender authentication guaranteed before.
- **History moves without leaving the agent.** On the first direct start the
  sidecar folds the agent's own MLS store, after a final sync, and only when
  every room it serves has synced to the server head (no quarantined room, none
  behind). The MLS store is only read, so the previous image still works on it.
  - The agent's copy differs from the person's in one invisible way: Home's
    `created_seq`, because Home is created before the agent joins.
  - Digests prove the rest per person.
- **One mode per agent, decided by the agent.** The dashboard uses direct chat
  only when the agent reports it. Otherwise it uses the hosted device.
  - A failed fold leaves the agent serving MLS chat.
  - So does a folded log that MLS has since moved past, such as after a
    rollback. That log is refused rather than served stale.
- **History is unavailable while the agent is down.** It lives with the agent,
  so it can't be viewed while the agent is down or restarting.
- **Unread state starts over.** Read state was per Device, so it restarts at the
  fold.
- **Attachments.** Those sent before the fold remain encrypted blobs on the
  chat server, decrypted on demand; new uploads are stored, encrypted, in the
  agent's home. Retiring the chat server first requires copying the older blobs
  locally (FIN-59).
- **Streaming.** No longer constrained by an append-only log: running edits can
  be ephemeral drafts. It is opt-in.
