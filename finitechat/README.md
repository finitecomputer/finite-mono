# Finite Chat

Finite Chat is end-to-end-encrypted chat for people and agents. If you are an
agent (Hermes or otherwise) asked to get yourself or a human onto Finite Chat,
use the `finitechat` CLI: one binary owns identity, MLS encryption, invites,
and durable message state.

- Nostr keys provide portable account identity and profile discovery.
- OpenMLS protects room contents and membership truth.
- The server only orders opaque encrypted payloads; it never reads contents.

The production server is `https://chat.finite.computer`. Use it unless you are
intentionally targeting a local development server.

## Install `finitechat`

Install the latest release binary:

```sh
set -eu

repo="finitecomputer/finite-mono"
tmp="$(mktemp -d)"
os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Darwin:arm64) asset="finitechat-macos-aarch64" ;;
  Darwin:x86_64) asset="finitechat-macos-x86_64" ;;
  Linux:x86_64) asset="finitechat-linux-x86_64" ;;
  *) echo "unsupported platform: $os $arch" >&2; exit 1 ;;
esac

base="https://github.com/$repo/releases/download/finitechat-latest"
curl -fsSL "$base/$asset.tar.gz" -o "$tmp/$asset.tar.gz"
curl -fsSL "$base/$asset.tar.gz.sha256" -o "$tmp/$asset.tar.gz.sha256"

if command -v shasum >/dev/null 2>&1; then
  (cd "$tmp" && shasum -a 256 -c "$asset.tar.gz.sha256")
else
  (cd "$tmp" && sha256sum -c "$asset.tar.gz.sha256")
fi

tar -xzf "$tmp/$asset.tar.gz" -C "$tmp"
mkdir -p "$HOME/.local/bin"
install -m 0755 "$tmp/finitechat" "$HOME/.local/bin/finitechat"
"$HOME/.local/bin/finitechat" --version
```

Make sure `$HOME/.local/bin` is on `PATH` before continuing. To build from
source instead, see [Development](#development).

## Discover The CLI

Start by asking `finitechat` what it can do:

```sh
finitechat --help
```

The `auth` family manages the shared identity, the `hermes` family owns agent
onboarding and the message bridge, and the `http` family exposes raw server
routes for debugging.

## Your Finite Identity

`finitechat` uses the shared Finite identity: one Nostr key per user, stored
at `~/.finite/identity/identity.json` (or
`$FINITE_HOME/identity/identity.json` when `FINITE_HOME` is set, e.g. in
hosted runtimes). Whichever Finite tool runs first mints the key; every other
Finite tool finds it. The on-disk format and concurrency rules are the
[Finite Identity Contract](https://github.com/finitecomputer/finite-identity),
shared with `fsite` and the rest of the Finite tools. `finitechat` never
copies the secret into its own stores.

```sh
finitechat auth status
```

If no identity exists yet, `finitechat hermes init` mints one. To keep an
existing npub, import its secret first:

```sh
finitechat auth import --file /path/to/secret
# or pipe it: printf '%s' "$SECRET" | finitechat auth import
```

`auth import` reads an `nsec1...` or 64-char hex secret from stdin or from a
`--file` whose content is just the secret. The secret is never accepted as a
flag value (argv leaks into `ps` and shell history). Import refuses to
overwrite an existing `identity.json` (another Finite tool may already be
using it).

## Onboard A Hermes Agent

Initialize the agent home against the production server. This mints or reuses
the shared Finite identity, registers the agent's device state, and publishes
an agent profile (skippable with `--skip-agent-profile`):

```sh
finitechat hermes init --server https://chat.finite.computer \
  --agent-name "My Agent"
```

The agent home defaults to `~/.finite/agent`; override it with
`--agent-home DIR` (or `FINITE_AGENT_HOME`). For local development, run a
local delivery server and point init at it instead:

```sh
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
finitechat hermes --agent-home .state/agent init --server http://127.0.0.1:8787
```

Then install the Finite Chat plugin into the Hermes agent (Hermes >= 0.16
plugin layout):

```sh
finitechat hermes install
```

`install` writes the embedded `finitechat` plugin into
`$HERMES_PLUGINS_DIR/finitechat`, `$HERMES_HOME/plugins/finitechat`, or
`~/.hermes/plugins/finitechat`, plus a local `finitechat.env` recording the agent
home and binary path (defaults only; explicit Hermes config and process
environment win). Flags: `--plugins-dir DIR` or `--plugin-dir DIR` to place
it elsewhere, `--plugin-name NAME`, `--finitechat-bin PATH`,
`--service-url URL` to point the plugin at a supervisor-managed
`finitechat hermes serve` process, `--force` to overwrite, `--json` for
parseable output.

Enable the plugin in `~/.hermes/config.yaml`:

```yaml
plugins:
  enabled:
    - finitechat

gateway:
  platforms:
    finitechat:
      enabled: true
```

Then `hermes gateway start` brings the agent onto Finite Chat.

## Invite A Human (Or Join A Room)

When the Hermes gateway starts with the Finite plugin, it prints a QR code, a
`finite://join?...` URL, and a rotating 6-digit PIN. A human scans or pastes
that into the Finite Chat app and types the PIN; the agent verifies the PIN
proof before admitting them to the encrypted room. You can also drive invites
directly:

```sh
finitechat hermes invite --room-name "Ops" --json   # create an invite (URL + PIN)
finitechat hermes invite-status --url INVITE_URL --json
finitechat hermes join --url INVITE_URL             # join someone else's invite
```

For the full agent integration surface (message polling, sending, the
supervised `hermes serve` bridge, smoke tests, and hardening evidence), see
[`integrations/hermes/README.md`](integrations/hermes/README.md).

## Development

Everything below is for understanding, running, or modifying Finite Chat
itself. Rust owns protocol state, persistence, networking, and product
policy; SwiftUI renders the Rust-owned app state and dispatches typed
actions.

The v1 product shape is a phone chat app for people and agents:

- Nostr keys provide portable account identity and profile discovery.
- OpenMLS protects room contents and membership truth.
- The HTTP server orders opaque encrypted payloads, persists delivery state, and
  never reads message contents.
- Offline text sends are durable, explicit retry is required after failure, and
  attachment upload stays online-only.
- Hermes integration uses the same chat surface as human conversations.

### Repository Map

- `crates/finitechat-core` - Rust app/runtime facade used by CLI and iOS.
- `crates/finitechat-client` - device state machine and encrypted local store.
- `crates/finitechat-server` - Axum HTTP delivery server with SQLite durability.
- `crates/finitechat-proto` / `finitechat-http` - wire DTOs and route contracts.
- `crates/finitechat-mls` - OpenMLS helpers and finite device credentials.
- `crates/finitechat-cli` - the `finitechat` binary: auth, Hermes bridge, and
  server calls.
- `crates/finitechat-rmp` - UniFFI, XCFramework, Xcode, and simulator helper.
- `ios` - SwiftUI app shell for `computer.finite.finitechat`.
- `integrations/hermes/finitechat` - Hermes platform plugin adapter.
- `docs/adr` and `docs/protocol-v1.md` - current product/protocol decisions.

### Local Loop

The production/default app server is `https://chat.finite.computer`. Local
server URLs are explicit development and test overrides only.

For a friend self-building the native app on their own Mac and phone, start
with `docs/friends-alpha-self-build.md`. That runbook covers branch checkout,
generated iOS bindings/project files, Apple signing, clean physical-device
install, and confirming the app is using the deployed server instead of a local
development override.

For server iteration or local automated testing, start a local delivery server:

```sh
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
```

Run the iOS simulator app against that server with an explicit override:

```sh
FINITECHAT_SERVER_URL=http://127.0.0.1:8787 cargo run -p finitechat-rmp -- run ios
```

To test the iOS app surface with a real local Hermes gateway, use the bundled
runner instead of the plain server command. This is a low-level local runner,
not the physical-phone canary gate:

```sh
scripts/hermes-real-gateway-demo.sh
```

In another terminal, point the simulator app at the runner's local server:

```sh
FINITECHAT_SERVER_URL=http://127.0.0.1:18788 cargo run -p finitechat-rmp -- run ios
```

The Hermes runner needs a prepared Hermes checkout with a `.venv`; set
`FINITECHAT_HERMES_REPO=/path/to/hermes-agent` if it is not in a sibling
checkout. It also needs the model provider key used by the Hermes profile. The
runner loads `.env` when present, or set
`FINITECHAT_HERMES_ENV_FILE=/path/to/provider.env`.

For the hardened "fresh Hermes instance to Paul's phone" quality loop, use
`docs/hermes-phone-canary-loop.md`. That runbook defines Finite Chat's local
phone and remote Docker gates and the evidence required before a human invite
is handed out.

The product-shaped hosted Hermes runtime ladder for local Docker, remote
Docker, and Phala belongs to
`../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`. Finite Chat's local
loop proves the app/protocol/plugin contract; v2 proves the hosted-agent
runtime image and provider deploy shapes.

For team testing, the normal Hermes phone canary is:

```sh
cp .env.example .env
# Fill in one model provider key in .env, usually OPENROUTER_API_KEY.
xcrun devicectl list devices
scripts/hermes-phone-canary.py \
  --install-phone-app \
  --ios-device <device identifier or hardware UDID> \
  --ios-development-team <Apple team id> \
  --keep-running
```

The script uses `https://chat.finite.computer`, builds the current
`finitechat` binary, installs the current iOS app on the paired phone, starts
real Hermes 0.17 with the `finitechat` plugin, proves invite admission
with a throwaway client, requires a real model reply, then prints the human
invite URL, report path, and `stop.sh`. Do not hand an invite to a
human from lower-level scripts that have not produced a passed report.

Remote Docker is the next promotion layer for teammates with access to the
builder host:

```sh
scripts/hermes-remote-docker-canary.py --keep-running
```

That wrapper requires a passed local phone report by default, builds the real
runtime image on `ssh://finite-lat-2`, proves real Hermes chat before and after
entrypoint backup/restore, and only then prints the invite URL for the restored
container.

The normal app flow is:

1. Sign in with an `nsec` or create a local Nostr identity.
2. Use **People** to open an existing profile or **Scan** to scan/paste an
   invite URL or `npub`.
3. Chat from the room surface. Rust owns send state, retry state, delivery
   projection, and attachment download decisions.

### Checks

Fast Rust/server checks:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo test -p finitechat-server --test http_routes
cargo test -p finitechat-server --test http_persistence
cargo test -p finitechat-server --test http_conformance
```

iOS checks:

```sh
cargo run -p finitechat-rmp -- doctor
cargo run -p finitechat-rmp -- bindings swift
cargo run -p finitechat-rmp -- test ios-simulator
```

`finitechat-rmp test ios-simulator` owns the simulator test lifecycle: it
creates or reuses a dedicated RMP simulator, shuts it down, erases it, runs the
full `FiniteChat` test scheme with isolated derived data and `.xcresult` output
under `.state`, then terminates and shuts the simulator down. Use `--json` when
automation needs the resolved UDID and result bundle path.

Hermes/Python checks:

```sh
uvx --no-config ruff format --check .
uvx --no-config ruff check .
uvx --no-config --with hermes-agent basedpyright
python3 -m unittest discover -s tests -p '*test*.py'
```

### Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds the
`finitechat` binary for linux-x86_64, macos-aarch64, and macos-x86_64 and
attaches `finitechat-<platform>.tar.gz` plus `.sha256` checksums to the
GitHub release. The install block at the top of this README consumes those
assets.

### Publish Safety

The repo is intended to publish as `finitecomputer/finitechat`.

Tracked source excludes local and generated state:

- `.env`, key files, SQLite stores, and `.state/` are ignored.
- `target/`, generated Xcode projects, Swift bindings, and XCFrameworks are
  ignored.
- iOS signing uses `ios/project.yml`; the generated `.xcodeproj` is local.

Before pushing, verify the GitHub target is the new repo. If
`finitecomputer/finitechat` resolves to `finitecomputer/finitechat-old`, do not
push or force-push; create or restore the new `finitecomputer/finitechat` repo
first.

### Deployment

This repo owns the Finite Chat server source, HTTP contract, and release gate
for `https://chat.finite.computer`. Hosted Finite Computer SaaS rollout
mechanics belong in `../finitecomputer-v2`, which owns the current chat-server
deploy lane, stack deploy coordination, and hosted runtime matrix. The legacy
`../finitecomputer` repo remains for box1/TRF users while they are unmigrated.
Do not ship a native app/TestFlight build that depends on server behavior until
the deployed chat server has been verified against the finite-chat commit being
shipped.

The production health endpoint must identify the deployed server build:

```sh
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
```

Expected production output includes `status: "ok"`, `server_version`,
`server_contract_version`, `source_commit`, and `source_dirty: false`. If
`server_contract_version` or `source_commit` is missing, the production server
is an old build and the app release is blocked until `../finitecomputer-v2`
deploys a compatible finite-chat commit. See
`docs/server-deployment-gate.md` for the required handoff and verification
steps.

For iOS beta distribution, see `docs/testflight-runbook.md`. Finite Chat uses
bundle ID `computer.finite.finitechat`.
