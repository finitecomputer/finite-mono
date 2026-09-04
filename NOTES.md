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

## Integration point for the swap

`src/lib/hosted-web-device.ts` is the seam: everything above it
(hosted-web-chat.ts orchestration + API routes) is finitechat-specific
bookkeeping (agent bindings, recovery); everything the renderer needs is the
presentation model. For the spike, add a parallel minimal path — e.g. a
`/api/chat/gateway/*` route pair (state + updates) backed by a server-side
ws client to `ws://127.0.0.1:9119/api/ws` that folds gateway events into
chat-ui model snapshots — and a chat page variant that uses it. Browser →
gateway direct is also possible (WebSocket has no CORS gate; the token rides
the query string exactly as hermes' own web UI does).
