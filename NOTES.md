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

## The experiment path

    browser chat UI
      → ws://127.0.0.1:9119/api/ws   (hermes web server, tui_gateway JSON-RPC)

Run the agent locally (flake packages hermes for aarch64-darwin):

    nix run .#hermes-agent -- web            # http://127.0.0.1:9119, /api/ws
    nix run .#hermes-agent -- web --port 9119

`hermes web` (hermes_cli/web_server.py) mounts `tui_gateway.ws.handle_ws` at
`/api/ws` — the same `server.dispatch` the Ink TUI uses over stdio. Wire
protocol: newline-delimited JSON-RPC both ways; server emits `gateway.ready`
immediately on connect. `/api/ws` takes `?token=<session token>` (printed at
startup) when auth is enabled.

Model provider: hermes resolves providers from its own config (`~/.hermes`
config.yaml / env / auth.json). A Finite Private key for local inference is
cached by devfinity at
`.local-state/devfinity/runs/default/finite-private-upstream.key`
(or `just dev inference-key` to set it).

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

## Dashboard with hot reload

devfinity's own dashboard process IS `pnpm run dev` (hot reload) on
127.0.0.1:13002 — but it watches the main checkout, which is shared. Iterate
HERE instead:

    cd /Users/futurepaul/dev/finite/finite-mono-worktrees/hermes-gateway-spike/finitecomputer-v2/apps/dashboard
    pnpm install --frozen-lockfile
    source ../../.local-state/spike-env.sh    # extracted from the running stack
    pnpm run dev --hostname 127.0.0.1 --port 13003

spike-env.sh is generated from the stack's process-compose.yaml dashboard
process env (FC_CORE_BASE_URL, FC_HOSTED_WEB_DEVICE_URL, FC_BRAIN_*,
FINITECHAT_HOSTED_API_TOKEN, FC_CORE_API_TOKEN, WorkOS fixture vars, plus
NEXT_DIST_DIR pointed at this worktree so manifests never mix with the main
checkout's — see the warning in devfinity/src/lib.rs ~2192).

## Integration point for the swap

`src/lib/hosted-web-device.ts` is the seam: everything above it
(hosted-web-chat.ts orchestration + API routes) is finitechat-specific
bookkeeping (agent bindings, recovery); everything the renderer needs is the
presentation model. For the spike, add a parallel minimal path — e.g. a
`/api/chat/gateway/*` route pair (state + updates) backed by a server-side
ws client to `ws://127.0.0.1:9119/api/ws` that folds gateway events into
chat-ui model snapshots — and a chat page variant that uses it.
