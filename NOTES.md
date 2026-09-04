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

## Real finitechat for parity testing (working, 2026-09-04)

The devfinity inference-key gate only consumes the key on the runtime
profile; services-only writes nothing. A dummy override boots the REAL
chat stack (postgres, core, finitechat server, hosted-web-device,
identity, brain, sites — no mocks) without any secret:

    DEVFINITY_PORT_OFFSET=1000 \
    FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE=dev-parity-dummy-no-inference \
      just dev up --headless --services-only

Ports land at +1000: core 15200, finitechat 19787, hosted-web-device
39918, dashboard 14002. `DEVFINITY_PORT_OFFSET` exists for exactly this.
Caveat: agent REPLIES still need the runtime + a real key
(`just dev inference-key`); chat mechanics (topics, chats, history,
MLS) are fully real. Check with `just dev status` under the same env.

The spike worktree dashboard serves BOTH real halves side by side:

    source .local-state/realstack-env.sh   # generated from the run dir:
    #   process-compose.yaml dashboard env + secrets/dashboard-auth.sh
    #   (NEXT_TSCONFIG_PATH dropped — it names a main-checkout file) +
    #   NEXT_PUBLIC_HERMES_GATEWAY_* + isolated NEXT_DIST_DIR
    cd finitecomputer-v2/apps/dashboard
    ../../scripts/with-dev-env pnpm run dev --hostname 127.0.0.1 --port 13003

Currently running: fixture+hermes at 13002 (finitechat half is MOCK),
real stack's own dashboard at 14002 (main-checkout code, no hermes
section), worktree dashboard at 13003 (real finitechat half + live
hermes section).

## Same components, hermes backend (2026-09-04, third pass)

Decision: NO new chat UI, ever. The existing chat components (AgentSidebar,
HostedWebChat, the whole tree) render UNCHANGED over the hermes gateway:

- `hermes-chat-provider.tsx` implements the SAME context the finitechat
  provider does (HostedChatContextValue — the context is now exported from
  hosted-chat-provider.tsx for exactly this swap). It maps room → the
  gateway, topics → projects (+ Recents), chats → sessions (+ local
  unprompted drafts), messages → session.resume transcript + streamed
  deltas as one "running" assistant message, SendMessage → prompt.submit,
  OpenChat → session.resume, CreateTopic → projects.create,
  StartTopicChatIntent → session.create with cwd = the project's path,
  RenameChat → session.title. MarkRoomRead/SetTyping/archive are quiet
  no-ops (gaps tracked below).
- `dashboard-shell.tsx` swaps the provider by route:
  `/dashboard/machines/[machineId]/gateway-chat` (page mirrors ../chat)
  gets HermesChatProvider; `/chat` is untouched. The route matcher
  (dashboard-chat-route.ts) treats gateway-chat as a chat surface.
- The invented portal chat pane and custom sidebar section from the
  previous pass are DELETED — that was new UI and the point of this pass
  is that none is needed.
- Verified: typecheck, SSR shell over the swapped provider, and the ws
  flow itself (earlier passes). Interactive loop in the real components:
  open http://127.0.0.1:13002/dashboard/machines/runtime_web_design/gateway-chat.

## Truth-sourcing rules for the provider (2026-09-04, fourth pass)

If hermes states it, the provider sources it; the provider invents nothing:

- Topics are EXACTLY projects.tree projects. The invented "Home" topic is
  gone — Recents (hermes' own scoped_session_ids bucket, what its desktop
  renders) is always present so New chat has a target; drafts pin to the
  top of it. canonicalNewChatTopic now falls back to the first topic, so
  transports without a Home topic still get a working New-chat FAB.
- Draft lifecycle mirrors hermes: session.create reports stored_session_id
  up front; the provider keeps drafts keyed by it and, the moment the
  stored id materializes in session.list (after the first prompt), swaps
  the draft row for hermes' row (auto-title, project placement) while
  carrying selection, handle, transcript, and stream state across.
  session.reclaimed (ws_orphan_reap) deletes a draft's row.
- Sidebar rows dedupe by chat id: local entries (live handles) win,
  previewSessions fill the rest.
- Known hot-reload artifact: heavy provider edits under Next fast refresh
  keep stale ref state (duplicate rows); a page reload resets it.

Thinking traces (fixed): reasoning deltas ride a kind:"tool" message, so
the shared transcript groups them into the existing collapsed ToolRollup
("Working · N steps" auto-open while running, "Worked through N steps"
collapsed after, expandable <pre> body). The answer stream is the only
prose bubble; the two can never bleed. Stable per-turn message ids
(chat:think:turnKey / chat:reply:turnKey) let React reconcile in place.
Correction: hermes history (session.resume) DOES return each assistant
row's reasoning, plus role:"tool" rows (name + context, no text). The
history mapping sends both to kind:"tool" rollup rows — tool steps are
labeled name:context, reasoning becomes a think step — and drops rows
with nothing to show. (The first mapping turned empty tool rows into
blank timestamp-only bubbles and dropped the reasoning entirely.)

First-message disappearance (fixed): the real transcript filters messages
by conversation_id === selectedTopic.topic_id. The first message was sent
while the chat was a draft in Recents, so it carried conversation_id
"recents"; when hermes filed the chat under its project the filter dropped
it. Two rules now hold: (1) conversation_id is a RENDER concern — publish()
stamps every message with the chat's CURRENT topic, so a topic move can
never orphan a message; (2) the moment a draft materializes, the local
transcript is REPLACED by hermes' authoritative history (session.resume
returns it inline; an in-flight stream keeps its tail after the fetched
rows). The local transcript is a display buffer, never truth.

## Remote gateways + gated mode (VERIFIED end to end 2026-09-04)

Pointing the dashboard at a REMOTE hermes (e.g. a bot on the tailnet) is an
env flip once the host is up:

    . .local-state/remote-gateway-env.sh   # gitignored; secrets live only there
    # then relaunch the fixture with those vars exported

Three facts drive the shape (read from hermes source + probed):

- Gated gateways auth by POST /auth/password-login {provider, username,
  password} (JSON; sets session cookies), then mint SINGLE-USE 30s ws
  tickets via POST /api/auth/ws-ticket → ws ?ticket=. The provider now
  supports this alongside the static ?token= path (username/password env
  selects gated mode; the password provider id is discovered from
  GET /api/auth/providers).
- Hermes' CORS allowlist is localhost-origins WITHOUT allow_credentials,
  so cookie login cannot happen cross-origin from the dashboard. The dev
  answer is an opt-in same-origin rewrite (next.config.ts,
  HERMES_GATEWAY_PROXY_TARGET): /hermes-gateway/* → the remote, verbatim.
  On our own prod domain (agents.finite.computer) dashboard and gateway
  are same-site and cookies flow natively — no proxy needed there.
- Zero-code alternative: restart the remote gateway with
  HERMES_DASHBOARD_SESSION_TOKEN=<secret> and use the static ?token= path
  (full-trust; fine for sandboxes).

Verified against the live sandbox: login → ws-ticket → ws (through the
same-origin rewrite) → 21 real sessions rendered under the bot's own
"Home" project (plus a live-created "workspace" project), and the
253-message session opened with its real history: prose bubbles plus
collapsed tool rollups ("Worked through N steps") exactly as stored.
Fixture gotcha: the web-design fixture allowlists env into the next dev
child — the gated-mode vars and the proxy target had to be added there.

Recents note: on any gateway, sessions are claimed by the project matching
their cwd, so Recents stays empty unless a session's cwd matches no known
project — that is hermes' model, not a bug. Expect the remote's real home
to render MANY projects; previews are capped (preview_limit 25) and
zero-message drafts are invisible by protocol.

## Local gateway hygiene (learned the hard way, 2026-09-04)

The spike's first gateway borrowed the desktop-owned ~/.hermes home and
hermes' default port. A hermes desktop UPDATE then killed our process and
took port 9119 for its own backend (desktop runs `hermes serve` too) —
"gateway unreachable". Fixes now in place:

- Isolated home: HERMES_HOME=.local-state/hermes-home (config.yaml +
  auth.json copied once for provider credentials; sessions/state stay
  separate from desktop forever). Fresh home ⇒ empty sidebar is correct.
- Dedicated port: `nix run .#hermes-agent -- serve --port 9120` with
  HERMES_DASHBOARD_SESSION_TOKEN from .local-state/hermes-session-token.
- Relaunch command + NEXT_PUBLIC_HERMES_GATEWAY_WS_URL=ws://127.0.0.1:9120/api/ws
  for the fixture. Eventual home for this hermes: inside the Apple
  Container runtime (needs `just dev inference-key`), not on the host.

Auth decisions captured: per-agent static token is the credential for
now; if hermes gated-mode password is added for DESKTOP clients, the web
dashboard never prompts for it — web rides brokered single-use tickets
(see defense-in-depth note).

## Public agent gateways: agents.finite.computer (captured 2026-09-04, design only)

Strategic frame: if every Finite agent's hermes gateway is reachable at a
stable public URL speaking VERBATIM tui_gateway JSON-RPC, then hermes
desktop (and TUI, and hermes' own web UI) can operate a Finite bot
directly — a whole client surface for free, inherited by pinning. Our web
chat becomes just one client. Hard constraint this buys: we never invent
protocol; anything our UI needs must be something a hermes-native client
could also do.

Domain + routing:

- `agents.finite.computer` at the edge. Recommended shape: per-agent
  wildcard subdomains, `wss://<agent-id>.agents.finite.computer/api/ws` —
  verbatim proxying (no path surgery), normal Host semantics for hermes'
  auth/Origin checks, wildcard TLS via the edge. Cheaper start (fine for
  a first cut): path-based `agents.finite.computer/<agent-id>/…` with
  prefix-strip at the edge.
- Doctrine fit: the edge proxies, never filters — Caddy forwards the
  agent listener verbatim; auth is enforced by the gateway itself
  (ticket/token, see the defense-in-depth note) and by gated issuance in
  the dashboard.
- Connections screen: add a "Hermes connection" card (the dashboard
  already has /dashboard/machines/[id]/connections) showing the
  per-agent ws URL + credential so users can paste it into hermes
  desktop.

THE open infra question — edge → gateway reachability: agent gateways
live inside Kata containers on runner hosts (lat2/3/4). The public edge
needs a path to the right container: per-agent host-port allocation,
a runner-host tunnel/relay, or routing through Core. This is the next
design decision; everything above depends on it.

Desktop credentialing without upstream hermes work (options, all
config-level):

1. Per-agent static session token (env-injected at spawn, displayed in
   the connections card, rotatable on redeploy) — simplest, full-trust,
   no revocation granularity.
2. Hermes gated mode with a per-agent password (auth providers are
   existing config) — per-agent scoped, browser-grade login flow for
   desktop.
3. Hermes OAuth provider pointed at WorkOS-as-OIDC, IF WorkOS can serve
   that role — one identity everywhere, but still cannot express
   account-owns-agent; only layer-1 hygiene, keep the broker anyway.

Compat cautions: verify hermes' Origin/host guards accept browser
cross-origin WS from our dashboard origin and desktop's no-Origin
connects; verify ws ticket flow works through the edge (upgrade
pass-through). Bumping the pin inherits new hermes client features —
and pins us to their protocol choices, which is the point.

## Defense in depth: gating the ws connection (captured 2026-09-04, design only)

Goal: reaching an agent's gateway requires BOTH (1) a real connection
credential AND (2) being signed in as the WorkOS account that owns that
agent. Two independent gates; neither alone suffices.

What hermes already ships (verified in pinned v2026.8.3 source):

- `hermes_cli/dashboard_auth/ws_tickets.py` — gated mode mints
  **single-use, 30s-TTL WS tickets** via authenticated REST
  (`POST /api/auth/ws-ticket`, passed as `?ticket=` on upgrade) because
  browsers can't set Authorization headers on WS. Plus a
  process-lifetime `internal_ws_credential` (env-only, never in HTML)
  for server-spawned clients. A leaked ticket is uninteresting.
- `dashboard_auth/` stack: middleware, token_auth, OAuth providers,
  login page, cookies, audit logging.

Leading design — the dashboard as ticket broker (small lift, no protocol
changes):

    browser ──WorkOS session──▶ dashboard route /api/chat/[machine]/gateway/ticket
                                  │ checks: WorkOS session + account-owns-agent
                                  │         (same Core machine-access check the
                                  │          hosted-device routes already do)
                                  ▼
                              agent gateway POST /api/auth/ws-ticket
                                  (dashboard authenticates with the agent's
                                   per-agent supervisor token, env-injected at
                                   runtime spawn — HERMES_DASHBOARD_SESSION_TOKEN
                                   already works this way)
                                  ▼
    browser ◀── single-use 30s ticket ── then ws://gateway/api/ws?ticket=…

Layers: (1) WorkOS auth + ownership is the only way to obtain a ticket;
(2) the ticket itself is single-use/TTL; (3) in the prod topology the
gateway binds loopback inside the agent's runtime, so the broker is the
only reachable path. Per-agent tokens arrive naturally because each
agent runtime spawns its own hermes.

Open questions:
- Does the static env token authenticate REST (ticket minting), or only
  gated-mode cookie/OAuth sessions? (token_auth.py exists — verify.)
- If hermes' OAuth provider can point at WorkOS as an OIDC IdP, hermes
  itself could validate viewer identity — but it cannot express
  ACCOUNT-OWNS-THIS-AGENT, so the broker still has to do ownership.
  Treat direct-WorkOS-to-hermes as a possible future simplification of
  layer 1, never a replacement for the ownership check.
- Fallback with zero hermes changes: a WorkOS+ownership-gated WS
  passthrough proxy (tunnel only — no protocol translation, no state).
  It reintroduces one server hop; only take it if ticket brokering
  needs upstream hermes work.
- Upstream ask if needed: externally-mintable tickets (an RPC for a
  supervisor to mint tickets) — matches the existing internal
  credential's philosophy.

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
  queries; the turn streams as events. `sessions.changed` (session_id
  "") IS the list-moved broadcast — refetch lists on it and the sidebar
  live-updates when any client creates/completes/reaps a session.
- A turn, captured live: `session.info` (model/provider/effort/tools) →
  `message.start` → `thinking.delta` (deliberating indicator) →
  `reasoning.delta` (reasoning stream) → `message.delta` (answer
  stream) → `reasoning.available` → `message.complete`
  {text, usage, status, reasoning}. `prompt.submit` acks with
  `{"status":"streaming"}` immediately.
- Opening a stored session: `session.resume {session_id}` returns a
  FRESH short gateway handle plus the transcript inline. Cold
  `session.history` without resume is "session not found".
- Auto-titling: hermes titles the session from its first turn
  ("Identifying the chat model" for a model question) — no client-side
  naming needed for parity with auto-titled chats.
- Titles: `session.list` can show an empty title briefly after
  materialization; `session.title` sets explicitly. UI falls back
  title → preview.

## Parity probe: side-by-side sidebar + basic chat (landed 2026-09-04)

Everything below landed in `components/hermes-gateway-chat.tsx` (the
server-side client and /api route from the first pass are DELETED):

- Sidebar section: projects (topics) + flat Recents, from
  `projects.tree`/`session.list`, refreshed live on `sessions.changed`.
- Session rows open a portal chat pane (no existing chat component is
  touched): transcript hydrated via `session.resume`, composer sends
  `prompt.submit`, reasoning + answer stream live, `message.complete`
  finalizes.
- "+" button: new blank chat (`session.create` draft); the pane says the
  draft is invisible until its first message; after the first turn the
  sidebar live-updates with hermes' auto-title, filed under the project
  matching the session's cwd.
- Verified end-to-end in the browser: "Hello from the direct-ws web
  chat! Which model are you?" → streamed "I'm Hermes Agent, running on
  the GLM model trained by Z.ai." → sidebar gained "Identifying the
  chat model" under finite-mono.

### Feature-by-feature backlog (existing chat behavior → gateway equivalent)

| Existing (finitechat) | Gateway protocol mapping | Status |
|---|---|---|
| Chat list | `session.list` + `projects.tree` | ✅ read-only, grouped by project |
| Open chat → transcript | `session.history` (rows by `row_id`) | next |
| Send message | `prompt.submit` | ✅ streams live |
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
