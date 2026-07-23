# Local Integration Harness

`devfinity` is the Finite monorepo local integration harness. The default
profile is the real local SaaS: it builds the canonical Agent Runtime image,
runs the generic Runner against Apple Container, and connects that runtime to
the local WorkOS fixture, Postgres, Core, Finite Chat, Hosted Web Device, Sites,
Brain, and dashboard used by automated integration tests. This is the complete
browser-product spine, not every unrelated service in the monorepo; Search, for
example, is not part of this profile.

Core owns provider placement in this path just as it does in production. The
generated devfinity Core configuration supplies an Apple Container placement
override; the browser does not choose or submit a Runner class. Production
leaves that override unset, so Standard hosting continues to resolve to Kata.

`process-compose` remains the process supervisor and log/TUI surface.
Devfinity owns topology, generated state, prerequisite checks, and explicit
profile selection. The Runtime receives Brain's host-reachable Apple bridge URL
as its transport plus the dashboard's browser-visible Brain origin as the one
canonical signed authorization origin. It also receives an exact development
HTTP-host allowlist; it never substitutes production when that configured local
Brain endpoint is unavailable.

## Real local SaaS

On an Apple silicon Mac running macOS 26 or newer:

```sh
container system start
just dev inference-key
just dev saas-smoke
just dev up
```

On later runs with a persisted agent, skip `just dev saas-smoke`. On a fresh
checkout, the smoke command is the one-time acceptance bootstrap: it obtains
and redeems a single-use Launch Code through the local operator fixture,
launches the agent, proves real chat and restart healing, and preserves the
agent for the following interactive `just dev up`. The smoke command owns the
default stack while it runs, so do not run it alongside `just dev up` or the
web-design fixture.

Then open <http://127.0.0.1:13002/dashboard>. The local account bypasses WorkOS
and Stripe only at their external boundaries. The bootstrap still uses the real
Core entitlement, Project, creation request, promoted Runtime artifact, Runner
lease, Apple container, Agent Principal, and Hosted Web Device chat.

### Optional WorkOS staging authentication

To exercise real WorkOS staging authentication locally, copy the repository-root
`.env.example` to the repository-root `.env`, ask Paul for the staging API key and client ID, and set
`WORKOS_STAGING_API_KEY` and `WORKOS_STAGING_CLIENT_ID`. The template supplies
the non-secret `WORKOS_STAGING_OPERATOR_ORG_ID` value. Configure the staging
application with these local URLs:

```sh
cp .env.example .env
chmod 600 .env
# Fill in the two blank values, then:
just dev up --workos-staging
```

- Redirect URI: <http://127.0.0.1:13002/callback>
- Sign-in endpoint: <http://127.0.0.1:13002/login>
- Logout URI: <http://127.0.0.1:13002/>

`.env` is ignored and must never be committed.
These staging values are used only by local Devfinity; packaged Electron builds
load the production dashboard and authenticate with production WorkOS.

`just dev inference-key` prompts without echoing and stores an existing
`fpk_live_...` key in the ignored local state directory with mode `0600`.
Devfinity reuses it on later runs and runs the in-tree chained Finite Private
limiter: local Core provisions a separate runtime key, local admission and
metering happen normally, and only the limiter owns the upstream key.

`FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY` remains an explicit one-run override. For
an explicit direct-key fallback:

```sh
export FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE=<finite-private-key>
```

Devfinity fails before starting the stack when neither credential is present.
It will not present a chat UI whose model calls are known to fail.

For Brain integration work that has no operator inference credential, an
explicit one-run exception can exercise Hosted Device identity, owner pairing,
the Agent Working Tree, and Apple Container restart persistence without making
an LLM request:

```sh
DEVFINITY_BRAIN_ONLY_SMOKE=1 \
  FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE=devfinity-brain-smoke-unused \
  just dev saas-smoke
```

This does not pass or replace the canonical real-chat acceptance. It only
skips the three inference reply assertions; the Hosted Web Device must still
connect, and all Brain/Apple assertions still run.

The complete Greenfield Brain setup/deletion matrix is a separate disposable
gate:

```sh
just brain-product-matrix
```

It launches the canonical image through the local Docker Runner against a fresh
temporary devfinity state root and a deterministic OpenAI-compatible model
stub. The model still drives real Hermes terminal tool calls, reads the
installed managed FiniteBrain skill, invokes the built `fbrain`, signs real
Brain HTTP, and crosses Hosted Device and Product Client boundaries. The gate
proves negative, unclear, and affirmative Personal setup, agent-first and
user-first Organization cases, durable content, duplicate protection, and
restart behavior. It is required in pull-request CI on the isolated Docker
runner. The ordinary persisted Apple SaaS smoke never runs this destructive
scenario reset.

Rerun the real launch/chat/restart acceptance test at any time with:

```sh
just dev saas-smoke
```

That check creates or reuses the local Project, waits for the Apple runtime,
opens the real Hosted Web Device chat, requires a real Hermes response, has the
owner establish the runtime Agent Principal as the one Personal Agent,
proves the agent can discover and write the Personal Brain through a Working
Tree under `/data/workspace`, reads the note back as the owner, restarts
the chat services and Agent Runtime, and repeats the identity, access, Working
Tree, and readback checks before requiring the final chat response.

`just dev up` does not silently grant an empty account agent admission; use the
fresh-checkout sequence above instead.

## Portable services-only profile

Linux CI and focused host-service work use an explicit profile that makes no
claim to provision an Agent Runtime:

```sh
just dev smoke
cargo run -p devfinity -- up --services-only
```

`just dev smoke` uses the isolated state root
`.local-state/dfs` (kept short for the macOS Unix-socket path
limit), requests `--fresh` explicitly, verifies
the service spine, and tears it down. `--fresh` is rejected for the default SaaS
profile so a routine start can never erase agent data.

## Apple Container host networking

Devfinity first checks for Apple's official `host.container.internal` bridge.
Apple documents creating it as:

```sh
sudo container system dns create host.container.internal --localhost 203.0.113.113
```

Devfinity never runs that privileged command itself. Apple notes that enabling
the localhost domain disables Private Relay and that its packet-filter rule is
lost on restart. When the domain is not configured, Devfinity reads the current
default vmnet gateway from `container network inspect default`, binds only the
container-facing Chat, Brain, and limiter sockets to that address (not every LAN interface),
and runs a disposable canonical-image probe before registering the artifact or
starting Runner. If neither route works, startup fails before an agent is
leased.

The runtime's published HTTP port remains bound to host loopback.

## Commands

```sh
just dev up
just dev up --headless
just dev saas-smoke
just dev smoke
just brain-product-matrix
just dev rust-smoke
just dev status
just dev cleanup
```

Passing a command after `--` starts the selected stack headlessly, waits for its
services, runs the command with non-secret generated environment variables, and
tears down the supervised host processes afterward.

`cleanup` is best-effort recovery for process-compose and its host process
trees. Detached Apple Agent Runtimes intentionally remain alive so local Chat
can exercise real service interruption and stream healing. Use the dashboard's
generic Stop operation when the runtime itself should stop.

## Common recovery

- If port 13002 is busy, stop the web-design loop or other devfinity instance.
  The design fixture can instead use
  `FC_WEB_DESIGN_PORT=13003 just dev web-design`.
- A preserved Apple runtime from another worktree can keep the default
  `finite-devfinity` container slot and port 18080 occupied after its host stack
  stops. Keep that runtime intact and give the isolated smoke its own identity:

  ```sh
  DEVFINITY_STATE_DIR="$HOME/.finite-devfinity-overnight" \
  DEVFINITY_APPLE_CONTAINER_NAME_PREFIX=finite-overnight \
  DEVFINITY_RUNTIME_AGENT_PORT=18081 \
  just dev saas-smoke
  ```

  The alternate state root must also keep the generated process-compose Unix
  socket below macOS's path-length limit.
- If a previous full stack exited badly, run `just dev status`, inspect
  `.local-state/devfinity/runs/default/logs/`, then run `just dev cleanup` and
  start it again. Cleanup preserves databases, chat state, and Runtime data.
- If Apple Container is unavailable, verify `container --version`, then run
  `container system start`. Host-network failures include the exact bridge
  remediation described below.
- If startup reports a missing inference credential, obtain the approved local
  development value from a Finite operator over the team's secret-sharing
  channel and export one of the two documented variable names. Never write it
  into this repository or a committed `.env` file.
- If a long-running local sign-in expires, restart the supervised stack to
  remint the local WorkOS fixture token; persisted product state remains.

## State and key handling

Default state lives under:

```text
.local-state/devfinity/runs/default/
```

Important paths include:

- `postgres/data/`: durable Core records and Project/Runtime links.
- `finitechat/server.sqlite3`: durable local delivery-server state.
- `hosted-web-device/`: per-user Hosted Web Device identity and MLS/chat state.
- `runner/apple-container/`: durable Agent Runtime bind mounts.
- `runtime-image/context/`: cached canonical-image build context.
- `runtime-image/build-report.json`: non-secret local image provenance.
- `logs/`: process logs.
- `env` and `urls.txt`: non-secret local client configuration.

Ordinary `up`, shutdown, and `cleanup` preserve every directory above. Runtime
container deletion must never imply state deletion.

Inference credentials are not written to process-compose YAML, `env`,
`urls.txt`, build reports, or logs. Devfinity copies only the selected credential
into a mode-0600 file under the gitignored run directory, removes the credential
from process-compose's inherited environment, and sources that file only in the
limiter or Runner process that owns it. The file is removed after supervised
shutdown; a crash may leave the protected copy for recovery/inspection, and the
next run replaces it atomically.

## Prerequisites

The default SaaS profile requires:

- Apple silicon and macOS 26 or newer.
- Apple Container CLI 1.1 or newer with `container system start` available.
- Enough space and memory for the canonical image build. Devfinity starts an
  absent builder with 8 CPUs and 8 GiB; it does not reconfigure an existing
  builder.
- One of the inference credentials described above.
- The normal Nix development shell (`direnv allow`), which supplies Rust,
  Postgres, Node, and process-compose.

The services-only profile is portable and does not require Apple Container or
an inference credential.
