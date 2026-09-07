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

repo="finitecomputer/finite-releases"
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

`finitechat` uses the current Finite Home's identity-owner key, stored
at `~/.finite/identity/identity.json` (or
`$FINITE_HOME/identity/identity.json` when `FINITE_HOME` is set, e.g. in
hosted runtimes). Whichever Finite tool runs first in that home mints the key;
every other Finite tool in the same home finds it. Human and Agent Runtime
homes remain separate. The on-disk format and concurrency rules are the
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
that agent home's Local Identity Key, registers the agent's device state, and publishes
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

## Start A Chat With An Agent

The agent publishes its Agent Principal `npub`; it does not create a room or
an invite session at gateway startup. A user Device scans/selects that profile,
publishes its KeyPackage, and starts the room through the normal MLS
Add/Welcome flow. Hosted Web clients are independent Devices using that same contract.

For the full agent integration surface (message polling, sending, the
supervised `hermes serve` bridge, smoke tests, and hardening evidence), see
[`integrations/hermes/README.md`](integrations/hermes/README.md).

## Development

Everything below is for understanding, running, or modifying Finite Chat
itself. Rust owns protocol state, persistence, networking, and product
policy.

The v1 product shape is a chat product for people and agents:

- Nostr keys provide portable account identity and profile discovery.
- OpenMLS protects room contents and membership truth.
- The HTTP server orders opaque encrypted payloads, persists delivery state, and
  never reads message contents.
- Offline text sends are durable, explicit retry is required after failure, and
  attachment upload stays online-only.
- Hermes integration uses the same chat surface as human conversations.

### Repository Map

- `crates/finitechat-core` - Rust app/runtime facade used by the CLI and device binaries.
- `crates/finitechat-client` - device state machine and encrypted local store.
- `crates/finitechat-server` - Axum HTTP delivery server with SQLite durability.
- `crates/finitechat-proto` / `finitechat-http` - wire DTOs and route contracts.
- `crates/finitechat-mls` - OpenMLS helpers and finite device credentials.
- `crates/finitechat-cli` - the `finitechat` binary: auth, Hermes bridge, and
  server calls.
- `integrations/hermes/finitechat` - Hermes platform plugin adapter.
- `docs/adr` and `docs/protocol-v1.md` - current product/protocol decisions.

### Local Loop

The production/default app server is `https://chat.finite.computer`. Local
server URLs are explicit development and test overrides only.

For server iteration or local automated testing, start a local delivery server:

```sh
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
```

When the server is behind a proxy or receives uploads from another local
service, set its externally reachable origin explicitly so encrypted attachment
references work from every device:

```sh
FINITECHAT_PUBLIC_URL=https://chat.example.com \
  finitechat-server serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
```

`--public-url URL` is the equivalent command-line option. The value must be a
bare `http` or `https` origin, without a path, query, credentials, or fragment.

For low-level server or build troubleshooting, the primitives can still run
independently:

```sh
cargo run -p finitechat-server -- serve 127.0.0.1:8787 --sqlite .state/finitechat.sqlite3
```

To isolate the real Hermes gateway without the rest of the product loop, use
the lower-level runner:

```sh
scripts/hermes-real-gateway-demo.sh
```

The Hermes runner launches the same flake-pinned Nix Hermes runtime used by CI
and the runtime image; it does not discover ambient sibling checkouts. It also
needs the model provider key used by the Hermes profile.
The runner loads `.env` when present, or set
`FINITECHAT_HERMES_ENV_FILE=/path/to/provider.env`.

The product-shaped Hermes runtime ladder for local Apple Container, Kata, and
Phala belongs to
`../finitecomputer-v2/docs/hermes-runtime-test-matrix.md`. Finite Chat's local
loop proves the app/protocol/plugin contract; v2 proves the hosted-agent
runtime image and provider deploy shapes.

For team testing, run the real local SaaS from the monorepo root:

```sh
container system start
export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY=<operator-held-key>
just dev saas-smoke
just dev up
```

On a fresh checkout, the smoke command obtains local Launch Code admission,
builds the flake-pinned Nix Hermes runtime into the canonical Runtime image,
provisions a real Apple VM, opens the Hosted Web Device, and proves independent
restart healing. It
preserves the agent for the following interactive `just dev up`; skip the smoke
on later runs with that persisted agent. Physical-phone and remote-Docker
scripts remain historical/manual experiments until rewritten against the same
Agent Principal + Welcome-first contract; they are not promotion gates.

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

Hermes/Python checks:

```sh
nix develop ..#hermes-bridge-ci --command ruff format --check .
nix develop ..#hermes-bridge-ci --command ruff check .
nix develop ..#hermes-bridge-ci --command basedpyright --pythonpath "$HERMES_AGENT_RUNTIME_PYTHON"
nix develop ..#hermes-bridge-ci --command bash -lc \
  'exec "$HERMES_AGENT_RUNTIME_PYTHON" -m unittest discover -s tests -p "*test*.py"'
```

### Releases

Pushing a `finitechat/vX.Y.Z` tag runs
`.depot/workflows/release-finitechat.yml`, which builds the
`finitechat` binary for linux-x86_64, macos-aarch64, and macos-x86_64 and
attaches `finitechat-<platform>.tar.gz` plus `.sha256` checksums to the
public `finitecomputer/finite-releases` release. The install block at the top
of this README consumes the refreshed `finitechat-latest` alias from the
release repository.

### Publish Safety

The repo is intended to publish as `finitecomputer/finitechat`.

Tracked source excludes local and generated state:

- `.env`, key files, SQLite stores, and `.state/` are ignored.
- `target/` is ignored.

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
Do not cut a release that depends on server behavior until the deployed chat
server has been verified against the finite-chat commit being shipped.

The production health endpoint must identify the deployed server build:

```sh
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
```

Expected production output includes `status: "ok"`, `server_version`,
`server_contract_version`, `source_fingerprint`, and `source_dirty: false`.
The fingerprint is derived automatically from the Nix package's scoped Chat
inputs; the NixOS rollout record separately identifies the monorepo commit. If
`server_contract_version` or `source_fingerprint` is missing, the production
server is an old build and the app release is blocked until the hosted stack
deploys a compatible finite-chat package. See
`docs/server-deployment-gate.md` for the required handoff and verification
steps.

Production serving readiness is separate from that liveness/version contract:

```sh
curl --fail --silent https://chat.finite.computer/readyz | jq .
```

`/readyz` acquires the ordering-authoritative delivery lock and commits then
reads back a singleton service-owned row through the production SQLite store.
It returns 503 when either shared seam is unavailable or the one-second server
budget is exceeded. The probe never creates a Room, Device, Message, or other
user-visible history, and results are cached for thirty seconds so a
public caller cannot amplify the probe's durable write rate.
