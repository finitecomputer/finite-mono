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
