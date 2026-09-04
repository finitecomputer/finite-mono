# Spike: web chat → hermes tui_gateway (bypassing finitechat)

Goal: feel out pointing the dashboard's web chat directly at a local hermes
agent's `tui_gateway` JSON-RPC WebSocket (`tui_gateway/ws.py`) instead of the
finitechat hosted-device path.

## Current web chat data path (what we're bypassing)

    browser
      → dashboard Next.js API routes
         src/app/api/chat/machines/[machineId]/hosted-device/{state,updates,actions}/route.ts
      → src/lib/hosted-web-chat.ts        (orchestration: bindings, bootstrap, recovery)
      → src/lib/hosted-web-device.ts      (HTTP transport to the hosted device)
      → FC_HOSTED_WEB_DEVICE_URL (finitechat-hosted-device, 127.0.0.1:38918 in devfinity)
      → finitechat (server 18787) → runner → Apple Container agent runtime (hermes inside)

The renderer is `@finite/chat-ui` (finitechat/packages/finitechat-chat-ui),
fed a presentation model (`src/model.ts`: rooms/chats/messages + seq cursors,
"JSON presentation model emitted by finitechat-core"). Hosted Web and local
Device renderers deliberately share the model, not the transport — so an
alternative transport only needs to produce this model.

## The experiment path (verified working 2026-09-04)

    browser chat UI
      → ws://127.0.0.1:9119/api/ws?token=<token>   (hermes serve, tui_gateway JSON-RPC)

### Running the agent gateway

    # headless gateway (this is what remote clients use; `dashboard` subcommand
    # is the same server + browser UI). NOTE: pinned v2026.8.3 has no `web`
    # subcommand despite the upstream docs page mentioning `main`.
    nix run .#hermes-agent -- serve              # 127.0.0.1:9119

Auth: `/api/ws` requires `?token=<session token>`. Set your own at launch:

    HERMES_DASHBOARD_SESSION_TOKEN=$(cat .local-state/hermes-session-token) \
      nix run .#hermes-agent -- serve

The token for the currently-running instance is at
`.local-state/hermes-session-token` (mode 600, gitignored); server log at
`.local-state/hermes-serve.log`.

Verified round-trip (hermes venv python has `websockets`):

    /nix/store/lq8c19c5vfw3nwfvk6lkf5m76rkmaxmb-hermes-agent-env/bin/python
    # → connect ws://127.0.0.1:9119/api/ws?token=…
    # RECV {"method":"event","params":{"type":"gateway.ready",...}}
    # SEND {"jsonrpc":"2.0","id":1,"method":"session.list"}
    # RECV {"id":1,"result":{"sessions":[...real session history...]}}

Model provider: hermes resolves from its own config (`~/.hermes` config.yaml /
env / auth.json). It boots fine keyless — prompting just fails until a
provider key exists. A Finite Private key for local inference is cached by
devfinity at `.local-state/devfinity/runs/default/finite-private-upstream.key`
after `just dev inference-key` (one-time interactive paste).

### JSON-RPC surface relevant to a chat client

Requests: `session.create`, `session.list`, `session.history`,
`prompt.submit` (rewind/edit = same method with `truncate_before_row_id`),
`session.interrupt`, `approval.respond`, `clarify.respond`, `command.dispatch`
(e.g. `/model <name>`), `model.options`, `terminal.resize`.

Events: `message.delta` (streaming tokens; coalesced server-side at ~30fps),
`message.complete`, `tool.start` / `tool.progress` / `tool.complete`,
`approval.request`, `clarify.request`, `gateway.ready`, session lifecycle.

Mapping to the chat-ui model: one gateway session ≈ one AppChatSummary;
`session.history` rows + `message.delta`/`message.complete` events ≈ message
list; seq cursors have no direct analogue (gateway rows carry `row_id`).

## Dashboard with hot reload (running now, keyless)

The design fixture is self-contained: an in-process fixture Core + fixture
hosted device + real `next dev`, serving THIS worktree's source with hot
reload. No stack, no keys:

    cd finitecomputer-v2/apps/dashboard
    ../../scripts/with-dev-env pnpm run design
    # → http://127.0.0.1:13002/dashboard/machines/runtime_web_design/chat

(scenarios: `pnpm run design:state healthy|unavailable|recovering`.)

The full devfinity stack (`just dev up`) also runs its dashboard as
`pnpm run dev` on 13002 — but every profile requires a Finite Private key
(`just dev inference-key`) and it watches the shared main checkout, so
iterate here instead. All Rust services are already built and nix-cached;
the next full `up` after `inference-key` skips straight to booting.

One-time per worktree: `direnv allow .` at the worktree root, or
`scripts/with-dev-env` silently no-ops.

## Architecture (decided 2026-09-04, second pass)

**Browser → gateway, direct.** No Next API routes, no server-side client, no
cache: the browser opens `ws://127.0.0.1:9119/api/ws?token=…` itself
(WebSockets have no CORS gate; the token rides the query string exactly like
hermes' own web UI). Connection info is inlined from
`NEXT_PUBLIC_HERMES_GATEWAY_WS_URL` / `NEXT_PUBLIC_HERMES_GATEWAY_TOKEN` —
for a real deployment the per-viewer token is the ONE injection point left
to design. Everything lives in
`src/components/hermes-gateway-chat.tsx` (hook + section, one file) with a
one-line mount in agent-sidebar.tsx, so the eventual cutover is one file
plus one line.

**Protocol facts (all verified live against v2026.8.3):**

- `projects.tree` is hermes' own authoritative sidebar RPC: projects →
  repos → lanes with preview sessions + `scoped_session_ids`. Topics =
  projects is not an invention — it IS the desktop's sidebar model. It also
  includes a zero-session "discovery tier" (every repo hermes has ever seen
  a cwd for — ~200 on this machine); the chat sidebar filters to
  `sessionCount > 0`.
- Sessions join projects by `cwd` at creation time (my test session created
  with the gateway's cwd landed in the "finite-mono" project). `cwd` is a
  `session.create` param — new chats choose their topic by choosing cwd.
- Drafts are invisible: `session.create` makes a gateway-memory draft with
  NO DB row; it appears in `session.list`/`projects.tree` only after its
  first `prompt.submit`. Drafts also die with the connection that made
  them. Parity note: the existing web chat shows empty new chats; hermes
  hides them until they say something.
- Lists are pull, turns are push: `session.list`/`projects.tree` are
  queries; `message.delta`/`message.complete`/`tool.*`/approvals stream as
  events. There is no session-list-change push — refresh the list after
  actions that change it.
- `prompt.submit` returns `{"status":"streaming"}` immediately; the reply
  arrives as events on the same socket.
- Titles: `session.list` showed an empty title for the test session (the
  create-time title had not materialized at list time; hermes auto-titles
  from the first turn, and `session.title` sets explicitly). UI falls back
  title → preview.

## Parity probe: side-by-side sidebar (landed 2026-09-04)

The sidebar now renders both transports next to each other:

    TOPICS                ← finitechat hosted-device (fixture data)
      General / Design review
    HERMES · API/WS       ← tui_gateway session.list over /api/ws
      Friendly greeting   (real hermes session history)

Pieces:

- `src/lib/hermes-gateway.ts` — one-shot JSON-RPC over a short-lived
  WebSocket (Node global WebSocket; no new deps). `hermesSessionsTopic()`
  folds `session.list` into `HostedChatTopic`/`HostedChatSummary` so the
  renderer shape is identical across transports.
- `src/app/api/chat/gateway/sessions/route.ts` — GET returns the topic JSON
  (502 + readable error when the gateway is down or the token is wrong).
- `components/agent-sidebar.tsx` — `HermesGatewaySection`: read-only rows
  styled with the same `finite-chat__*` classes, reload button, error line.
- `scripts/web-design-fixture.ts` — allowlists HERMES_GATEWAY_WS_URL /
  HERMES_GATEWAY_TOKEN through to the dashboard child.

Boot: `HERMES_GATEWAY_TOKEN=$(cat .local-state/hermes-session-token) \
../../scripts/with-dev-env pnpm run design` from the dashboard dir (token
also rides ?token= like hermes' own web UI).

### Feature-by-feature backlog (existing chat behavior → gateway equivalent)

| Existing (finitechat) | Gateway protocol mapping | Status |
|---|---|---|
| Chat list | `session.list` + `projects.tree` | ✅ read-only, grouped by project |
| Open chat → transcript | `session.history` (rows by `row_id`) | next |
| Send message | `prompt.submit` | — |
| Streaming reply | `message.delta` (coalesced ~30fps) + `message.complete` | needs live subscription |
| New chat | `session.create` (+ `session.activate`) | — |
| Rename chat | `session.title` | — |
| Archive | no direct analogue — closest is `session.close` | gap to name |
| Unread counts | no analogue (server-side read state is finitechat MLS) | gap to name |
| Rewind/edit | `prompt.submit` + `truncate_before_row_id` | — |
| Approvals | `approval.request` / `approval.respond` events | — |
| Topics/rooms | `projects.tree` (user projects + auto-discovery by cwd) | ✅ projects ARE topics (desktop sidebar model) |

Hermes pin: v2026.8.3. Checked v2026.8.3..v2026.8.31 — gateway changes are
fixes (compute-host interrupt forwarding, clarify relay, per-profile PID
isolation) and wire-neutral refactors; nothing that eases ws integration.
No bump needed for the spike.

## Integration point for the deeper swap

`src/lib/hosted-web-device.ts` is the seam: everything above it
(hosted-web-chat.ts orchestration + API routes) is finitechat-specific
bookkeeping (agent bindings, recovery); everything the renderer needs is the
presentation model. For the spike, add a parallel minimal path — e.g. a
`/api/chat/gateway/*` route pair (state + updates) backed by a server-side
ws client to `ws://127.0.0.1:9119/api/ws` that folds gateway events into
chat-ui model snapshots — and a chat page variant that uses it. Browser →
gateway direct is also possible (WebSocket has no CORS gate; the token rides
the query string exactly as hermes' own web UI does).
