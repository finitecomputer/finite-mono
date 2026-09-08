# Private SimpleX connections

Private owner DMs only; groups and typing indicators are deferred. No Hermes
fork or adapter patch. The canonical Hermes pin advances from v2026.8.3 / 0.20.0
to v2026.8.31 / 0.21.0. SimpleX v7.0.2 assets are checksum-pinned through Nix.
The upstream Hermes flake brings a Home Manager input for its own module checks;
Finite does not enable a Home Manager module.

## Product and ownership

Connections → Connect creates a managed identity and renders its contact QR.
The user scans it, sends a message, then explicitly approves the exact pending
request in Connections. Hermes PairingStore owns authorization. Names are not
identity proof; the UI also shows contact ID and request age. Approval collapses
the setup UI. Unknown contacts cannot invoke the agent.

Dashboard commands use the existing authenticated Hosted Web Device → Agent
Platform Channel → agentd path. Core and Runner gain no SimpleX-specific API.
The only new product operations are connect, approve-request, and confirmed reset.
There is no code-entry or pause API. Unsupported runtimes fail rather than
silently substituting a different operation.

agentd applies `gateway.platforms.simplex` through its config-offer journal and
restarts Hermes. This can interrupt an active turn. Its fixed-operation Python
helper bootstraps and supervises the daemon; the daemon alone opens its databases.
Existing user-managed YAML or `.env` configurations are rejected for review.

## Environment and transport

- One daemon per runtime, WebSocket at `127.0.0.1:5225`, outbound public SMP/XFTP.
- The canonical toolchain closure includes the daemon; no runtime downloads.
- For managed connections the gateway wrapper supplies `SIMPLEX_WS_URL`,
  `SIMPLEX_AUTO_ACCEPT=true`, `SIMPLEX_ALLOW_ALL_USERS=false`, empty
  `SIMPLEX_ALLOWED_USERS`, and empty `SIMPLEX_GROUP_ALLOWED`.
- No provider credential is needed for SimpleX itself. Inference retains the
  platform's existing configuration and credentials.
- `FINITE_AGENTD_SIMPLEX_SCRIPT` is a local helper-path override, not browser input.

The released adapter's contact acceptance event/command differs from SimpleX v7.
Bootstrap enables daemon-native `/auto_accept on` before publishing the address;
transport acceptance does not grant agent access. The adapter can also lose a
pairing reply sent before contact readiness. Dashboard pending-request approval
removes delivered codes from the authorization flow.

SimpleX WebSocket clients compete for one event queue. A second diagnostic client
can steal Hermes messages. Therefore only initial bootstrap opens a control
WebSocket. Subsequent status uses cached address, PID, and TCP liveness. TCP
readiness does not prove recipient delivery or inference success. General outbound
send acknowledgement and gateway-outage replay remain upstream limitations.

## Persistence and confirmed disconnect

Agent Home stores `simplex/identity_chat.db`, `simplex/identity_agent.db`, their
live WAL/SHM files, attachments, and `simplex/address.json`. Hermes Home stores
pairing records, `state.db`, session transcripts, and routing metadata. Ordinary
restart/image replacement retains these. Missing/incomplete identity state fails
closed; startup never silently replaces it.

Disconnect warns before deleting SimpleX identity, contacts, approvals, requests,
rate limits, attachments, and SimpleX Hermes sessions. Phone messages, other
platforms' data, and shared agent memory remain. Reconnect requires new pairing.

The reset operation disables managed config, writes `simplex-reset.json`, and
restarts Hermes. Before starting the new gateway, the wrapper waits for the daemon
to stop. Cleanup cannot unlink a live daemon database or race the old gateway's
pairing writers. Both legacy/current pairing layouts are cleaned so migration
cannot resurrect grants. Only SimpleX keys are removed from shared rate limits.
Pinned SessionDB APIs delete SimpleX sessions and their delegate children; routing
rows and the legacy JSON mirror are cleared without removing other platforms.

The reset journal records transcript IDs before deletion. A partial filesystem
failure can therefore retry after database rows are gone. Pending reset blocks
new addresses and approvals; failed cleanup leaves SimpleX disabled but allows
other gateway platforms to start. Success is reported only after cleanup ends.
No archive or remote phone wipe is implied.

## Development and qualification

Use the existing `just dev web-design` for UI fixture work and `just dev up` /
`just dev saas-smoke` for the canonical local platform. This PR adds no native
runtime mode or feature-specific dashboard backend. The experimental native phone
trial was removed after validating the UX; its success is not production control
path proof.

- `just computer simplex-test`: offline control, exact-request authorization, and
  reset tests using pinned upstream stores. Includes cross-platform isolation,
  legacy metadata, live-daemon refusal, and interrupted transcript cleanup.
- `just computer simplex-smoke`: opt-in public-relay test with two disposable
  daemons and the released adapter; no inference or user contacts.
- PR CI checks Python formatting/lint, control tests, and Linux daemon packaging,
  alongside the existing Rust, dashboard, and Hermes bridge suites.
- Manual native trial validated QR → pending approval → private conversation and
  confirmed reset. Dashboard screenshots document that UI, not platform auth.

## Hermes upgrade qualification

Local checks use the actual old 0.20.0 and candidate 0.21.0 packages where
specified. Stand-in adapter tests remain regression coverage, not upstream
compatibility proof. Full local Python discovery ran 236 tests: 235 passed,
with the gated media test skipped. The separate media attempt is recorded below.

| Contract | Local evidence | Remaining boundary |
| --- | --- | --- |
| Persisted state | New opt-in test runs old → candidate → old → candidate, retaining three distinct routes, transcripts, approvals, pending requests, config and credentials; schema 25 → 26 integrity checks pass | Synthetic text state only; no concurrent writers or interrupted migration |
| Recovery | Stopped-writer baseline home restored into an empty target and reopened by old Hermes | Not the complete production Recovery Set or storage transport |
| Adapter lifecycle | Eight actual pinned tests pass, including cancellation/redelivery, handler failure and retry-before-ack; three new cases also pass on old Hermes | Sidecar boundary is stubbed; full process/inbox persistence still needs runtime smoke |
| Provider interpretation | Actual candidate config loader/provider resolver accepts Finite Private model, endpoint, environment-key reference and chat-completions mode | No model request or credential refresh in this check |
| Turn leases | Actual candidate store/lease registry rejects overlapping alias sessions and accepts retry after release | Full runner execution remains a canary check |
| Media lane | Nix binaries build; existing opt-in test fails before media on removed `finitechat hermes invite` command | Stale test cannot qualify media; real exchange required in canary |
| Telegram approval | Invalid code prints rejection but exits zero on both pins | Pre-existing agentd exit-code-only check can report false success; do not count a success toast as approval proof |

Reproduce the migration test with `HERMES_BASELINE_PYTHON` set to the old pinned
interpreter and run `finitechat/tests/hermes/test_pinned_hermes_upgrade_state.py`
using candidate Python. It explicitly skips without a baseline. This proves
narrow sequential downgrade compatibility, not rollback of candidate-only state;
the old runtime does not support SimpleX. Keep the pre-upgrade Recovery Set.

### Canary acceptance

Use one existing canary agent plus a fresh agent, with the canonical candidate
Runtime image. Stop the previous writer before changing images. Run
`scripts/finite-status` before and after the authorized rollout.

1. **Existing chat and restart:** continue an old conversation, verify prior
   history and routing, send a queued follow-up during a long turn, then restart
   during another turn. Confirm recovery completes without missing input,
   duplicated replies, stuck processing, or replies landing in another chat.
   Run the existing durable-home and interruption runtime smoke lanes.
2. **Inference and identity:** retain the existing Finite Private/custom config;
   request a terminal tool action and repeat after restart. Exercise two
   authenticated requesters and verify the tool runs under the correct requester.
3. **SimpleX and onboarding:** on the fresh agent, use the real dashboard QR,
   send a message, approve the exact request, and exchange replies. A second,
   unapproved contact must not invoke the agent. Confirm disconnect, reconnect
   with a fresh flow, and verify other chat history/connections remain intact.
4. **Media and other connections:** send an image/file and voice input and check
   meaningful model responses and outbound attachments. Verify existing Telegram
   chat and valid/invalid/expired approval outcomes from actual authorization
   state. Exercise Google setup and an already-connected Google tool.
5. **Shared-session contention, if used:** `/resume` the same session from two
   chats, overlap turns, and verify a visible resend notice followed by a
   successful retry and coherent history. Hermes waits five seconds by default;
   this notice is treated as a terminal response by the Finite inbox.
6. **Recovery:** restore the complete pre-upgrade Recovery Set onto an empty
   canary target and prove chat availability. Do not run old and new writers
   concurrently or equate a successful same-volume restart with recovery proof.

Keep the PR draft until the required canonical-image checks are resolved. These
checks preserve the upgrade without claiming every upstream change is covered.

## References

- https://hermes-agent.nousresearch.com/docs/user-guide/messaging/simplex
- https://github.com/NousResearch/hermes-agent/releases/tag/v2026.8.31
- https://github.com/NousResearch/hermes-agent/blob/v2026.8.31/plugins/platforms/simplex/adapter.py
- https://github.com/simplex-chat/simplex-chat/blob/v7.0.2/apps/simplex-chat/Server.hs
