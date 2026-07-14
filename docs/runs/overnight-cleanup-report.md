# Overnight Cleanup Report

Status: IN PROGRESS

Run: [`overnight-cleanup.md`](overnight-cleanup.md)

Branch: `overnight-cleanup-2026-07-13`

Base: `ec7bb9031b138919c2696d221d631d54c8060dee`

## 1. Stale chat pane on switch — SHIPPED

What changed: The dashboard now follows a rejected selection-only mutation
response with a fresh HTTP state snapshot. The reconciliation snapshot may
replace an unchanged equal-revision state, while the existing generation and
mutation-sequence checks still prevent stale responses from rolling state
back. The browser fixture now matches the daemon's selection-only `OpenChat`
revision behavior and covers a stream update landing while `OpenChat` is in
flight.

Verification evidence:

- Dashboard unit tests: 174 passed.
- Dashboard lint: passed.
- Dashboard browser suite: 2 passed, including the forced `OpenChat`/SSE race.

Two-minute morning verification:

1. In the local stack browser, open an Agent with at least two Chats under one
   Topic.
2. Send or receive activity so the stream is live, then switch rapidly between
   the two Chats.
3. Confirm the highlighted Chat, header, and transcript all change to the last
   clicked Chat without another click or message.

Paul action: none beyond the morning browser verification.

## 2. Disaster recovery drill — BLOCKED

What ran locally: A disposable synthetic Recovery Set containing 17 opaque
Account, Device, Room, Topic, Chat, message, attachment, Project, Runtime, and
Agent identifiers was built from two valid SQLite databases and a Postgres 16
custom-format dump. `restore-hosted-web-chat-snapshot` installed the verified
set atomically onto an empty target in 306 ms. The installed databases and dump
were readable and their manifest reverified. The source remained unchanged.

Negative evidence: six attempts were refused before target mutation: missing
isolated-mode acknowledgement, unsupported format, hash-modified artifact,
missing database, corrupt SQLite with a recomputed manifest, and non-empty
target. The non-empty target's sentinel remained intact.

Read-only remote evidence on 2026-07-14 UTC:

- Both production snapshot/off-site timers were active; both health services'
  latest results were successful.
- The latest service-consistent snapshot was 293 seconds old and its manifest
  verified.
- The recovery credential directory was mode `0700`; its three named files
  were root-owned mode `0600`.
- The off-site success stamp was 764 seconds old.
- The three newest Borg archives listed successfully. The selected candidate
  is `finite-lat-1-hosted-web-chat-2026-07-14T02:37:00`; an archives-only Borg
  check of the newest archive passed.

Why blocked: No empty external isolated target was named or proven to have
public ingress, email, webhook, push, billing, and other side effects disabled.
There is also no observed fence proving the retained Agent Runtime cannot
contact both source and restored stacks. Extracting the production archive on
finite-lat-1 would write to the live production host; extracting it onto this
Mac would handle the complete production Recovery Set outside a named isolated
target. Both violate the drill boundary. Wrong-key and truncated-Borg-archive
proofs therefore remain unrun, as do service startup, identifier comparison,
attachment decryption, Runtime reconnect, and a fresh Agent turn.

Two-minute morning verification:

1. Review this entry and confirm the selected archive and timings are
   acceptable as preflight evidence only.
2. Confirm that no restore target or traffic switch was created by this run.

Paul action: Name/provision the empty isolated target, confirm its outbound
side-effect fence, name the retained Runtime and its source/restored-stack
network fence, and confirm independent Borg passphrase/key-export custody.
Then the remote drill can resume without redoing the local verifier work.
