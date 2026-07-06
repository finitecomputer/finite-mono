# Agent Invite Flow — Execution Plan

Companion to ADR 0006. Status ledger lives at the bottom; commit per phase.

## Goal

`hermes-agent` (latest upstream) + the `finitechat` binary +
the `finitechat` plugin = drop-in agent you can start chatting with
from the Finite Chat app by scanning a QR and typing a PIN. Proven by an
end-to-end test where Hermes runs in a Linux container (Apple `container`)
and the user side runs on the host. Then a perf pass so the flow and the
message path have no pathological behavior and a best-in-class latency
story.

## Current gaps (audited 2026-06-12)

- The Python plugin (`integrations/hermes/finitechat/adapter.py`)
  shells to `finitechat hermes <action> --json` — **those subcommands do
  not exist**; they died with the deleted engine. The CLI today is a thin
  per-route HTTP client with no MLS device.
- `finitechat-hermes` (Rust DTO crate) survives and is the bridge contract.
- No invite/QR/PIN code anywhere. Link sessions are the closest lifecycle
  model.
- Local hermes-agent fork is 417 commits behind NousResearch upstream;
  upstream plugin interface needs re-verification (research in flight).
- Apple `container` 1.0.0 pkg downloaded to /tmp, awaiting sudo install;
  machine is on macOS 26.3 (full support).
- Client room records carry no server address (ADR 0005 item 1 pending).

## Phases

1. **Docs** — ADR 0006 + this plan. ✔
2. **Proto + server: invite sessions.** DTOs and limits in
   finitechat-proto/http; durable invite-session state + the six routes in
   finitechat-server (check → persist → apply, op-logged, snapshot-covered);
   restart + idempotency + limit tests alongside the link-session suites.
3. **Client: addressing, invite codes, join/accept.**
   - ADR 0005 item 1: `server_url: Option<String>` on room records
     (serde-defaulted), sync/fanout ticks group rooms by server.
   - Invite code v1 encode/parse (`finite://join?...`) with strict version
     gate; PIN + pin_proof primitives (hmac).
   - `create_room_invite`, `accept_pending_joins` (inviter), and
     `join_via_invite` (joiner: join request → status poll → welcome claim
     → agent-npub verification).
   - Tests: full invite round trip over HTTP, wrong PIN rejected, expired
     window rejected, server-side KeyPackage substitution rejected,
     unknown-version invite rejected.
4. **CLI: `hermes` subcommand family.** Persistent encrypted device store
   under `FINITECHAT_HOME`; subcommands `init`, `invite`, `pin`, `poll`,
   `send`, `edit`, `activity` speaking the finitechat-hermes JSON contract
   on stdin/stdout; QR as terminal Unicode; npub bech32 display. Process
   tests in the Python suite (binary-level, like test_process_binary_smoke).
5. **Hermes plugin refresh.** Reconcile adapter.py/plugin.yaml with latest
   upstream plugin practice (research agent report); surface the invite as
   an onboarding affordance; extend tests/hermes unit suite for the new
   actions.
6. **Container harness.** Install `container`, `container system start`;
   Linux image with hermes-agent (latest) + plugin + a Linux build of the
   binary; host runs finitechat-server; guest→host over the vmnet gateway;
   scripted bring-up/teardown modeled on finitecomputer's msb scripts; the
   end-to-end test drives: agent invites → host client joins via URL + PIN
   → message round trip through Hermes → restart survival.
7. **Perf.** Measure invite→joined and message round-trip; kill pathological
   polling (long-poll `wait_ms` on sync/inbox/invite routes is the lead
   candidate); perf_baseline additions; perf-log + architecture report
   updates.

## Status

- [x] Phase 1 — docs
- [x] Phase 2 — proto + server invite sessions
- [x] Phase 3 — client addressing + invite codes + join/accept
- [x] Phase 4 — CLI hermes subcommands
- [x] Phase 5 — hermes plugin refresh
- [ ] Phase 6 — container harness + e2e
- [x] Phase 7 — perf pass (container e2e live run pending runtime install)
