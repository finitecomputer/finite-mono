# Finite Sites

Finite Sites is a git-backed static publishing platform for agents.

If a human asks you to publish or edit a Finite Site, use the `fsite` CLI. The
Project Repository is the editable source of truth. `finite.toml` selects which
committed directory becomes the served website. Finite Sites serves committed
bytes; it does not run builds for you.

The v2 validation API is `https://v2.finite.chat`, and this `fsite` build uses
it by default. Do not set `FINITE_SITES_API` unless you are intentionally
targeting a local or self-hosted server.

## Install `fsite`

For v2 validation, use the reviewed binary artifact for the exact revision
being tested or build it locally from this repo:

```sh
nix build .#fsite
./result/bin/fsite --version
```

The public `fsite-latest` rolling release is for the canonical production
contract. Do not advance or rely on it for this static-only v2 API until the
canonical production endpoint is ready for that contract.

After cutover, install the latest release binary:

```sh
set -eu

repo="finitecomputer/finite-releases"
tmp="$(mktemp -d)"
os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Darwin:arm64) asset="fsite-macos-aarch64" ;;
  Darwin:x86_64) asset="fsite-macos-x86_64" ;;
  Linux:x86_64) asset="fsite-linux-x86_64" ;;
  *) echo "unsupported platform: $os $arch" >&2; exit 1 ;;
esac

base="https://github.com/$repo/releases/download/fsite-latest"
curl -fsSL "$base/$asset.tar.gz" -o "$tmp/$asset.tar.gz"
curl -fsSL "$base/$asset.tar.gz.sha256" -o "$tmp/$asset.tar.gz.sha256"

if command -v shasum >/dev/null 2>&1; then
  (cd "$tmp" && shasum -a 256 -c "$asset.tar.gz.sha256")
else
  (cd "$tmp" && sha256sum -c "$asset.tar.gz.sha256")
fi

tar -xzf "$tmp/$asset.tar.gz" -C "$tmp"
mkdir -p "$HOME/.local/bin"
install -m 0755 "$tmp/fsite" "$HOME/.local/bin/fsite"
"$HOME/.local/bin/fsite" --version
```

Make sure `$HOME/.local/bin` is on `PATH` before continuing.

## Discover The CLI

Start by asking `fsite` what it can do:

```sh
fsite --help
fsite describe workflow register-and-publish --output json
fsite describe workflow publish-static-site --output json
fsite describe workflow project-config --output json
```

Prefer `--output json` for commands whose output you need to parse.

## Your Finite Identity

`fsite` uses the current Finite Home's identity-owner key, stored at
`~/.finite/identity/identity.json` or `$FINITE_HOME/identity/identity.json`.
Whichever Finite tool runs first in that home mints the key; every other
Finite tool in the same home finds it.

```sh
fsite auth status --output json
```

`--email` always means a deliverable Mailbox Address. `--nip05` always means a
Finite Identity resolution name, and `--npub` always means a native key.
Managed Agent NIP-05 names are not mailboxes: passing one to an email flag
fails before any invitation or challenge is delivered and points to the
corresponding NIP-05 flag.

Email proofs are daemon-local. `finitesitesd` emails a 15-minute, single-use
token, and `fsite auth redeem MAILBOX TOKEN` redeems it against the same Sites
daemon. The Sites-only delegation flow is:

```sh
fsite auth sites-key request paul@example.com
fsite auth sites-key add paul@example.com TOKEN_FROM_EMAIL --output json
fsite auth sites-key revoke paul@example.com TOKEN_FROM_EMAIL npub1... --output json
```

Older `fsite` releases stored the key at
`~/.config/finite-sites/identity.env`. To keep publishing as the same npub,
import the old secret into the shared identity file once:

```sh
fsite auth import --file ~/.config/finite-sites/identity.env
```

## Publish A Static Site

1. Register this Finite Home's Publishing Key for publishing:

```sh
fsite whoami
fsite auth register --output json
```

2. Put the deployable website bytes in a dedicated directory such as `site/`
or `dist/`. Keep source, data, scripts, and build logic in the Project
Repository too. Only the configured site path is served as the website.

3. Create `finite.toml`:

```toml
[project]
slug = "my-project"

[site]
name = "my-project"
branch = "main"
path = "site"
spa = false
```

`[site].name` is optional and defaults to `project.slug`. A `[project]`-only
config creates a source-only Project Repository with no served site.

4. Validate and create the Project Repository:

```sh
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

Project Init is replay-safe. If the server returns `git_unavailable`, no
Project Init state changed: wait for service health to recover and retry the
exact command once. If it returns `git_repository_setup_failed`, the Project
registry state may already be durable even though the repository is not ready.
Keep the same slug and local source; after the operator repairs Git or
repository storage, replay the exact `fsite project init --config finite.toml
--output json` command once.

5. Store a scoped Git Credential, commit source plus deploy bytes, and push the
Deploy Branch:

```sh
fsite auth git my-project --store --output json

git init -b main
git remote add finite https://v2.finite.chat/my-project.git
git add finite.toml site
git commit -m "Initial Finite Sites publish"
git push finite main
```

Pushing the configured Deploy Branch creates a new immutable Version. Finite
Sites validates and serves the committed bytes under `[site].path`. A
successful push returns only after the matching site version is active.

Confirm the URL returned by the configured server and preview that exact
origin:

```sh
fsite project status my-project --output json
fsite view my-project --output json
```

For an owned Project, `fsite view NAME` resolves the served site through the
configured `FINITE_SITES_API`; it does not invent a production hostname.

## Edit, Share, And Collaborate

If you start from a site URL, read the agent handoff first:

```sh
curl -fsSL https://SITE.v2.finite.chat/llms.txt
fsite view https://SITE.v2.finite.chat/ --output json
```

Project collaboration controls who can clone and push source:

```sh
fsite project grant PROJECT --npub npub1... --output json
fsite project revoke PROJECT --npub npub1... --output json
fsite project grant PROJECT --nip05 my-agent@finite.vip --output json
fsite project grant PROJECT --email editor@example.com --send-invite --output json
fsite project revoke PROJECT --email editor@example.com --output json
```

Use exactly one of `--email`, `--nip05`, or `--npub`. Native collaborators
authenticate with their own Local Identity Key and run `fsite auth git PROJECT
--store`; they do not link or impersonate the project owner's mailbox.

Site visibility controls who can view the served website:

```sh
fsite project share PROJECT --shared --add-email viewer@example.com --send-invite --output json
fsite project share PROJECT --add-nip05 my-agent@finite.vip --output json
fsite project share PROJECT --add-npub npub1... --output json
fsite project share PROJECT --remove-npub npub1... --output json
fsite project share PROJECT --public --yes-public --output json
fsite project share PROJECT --private --output json
```

When an authenticated human asks an Agent Principal to publish a Site,
Hermes and `fsite` carry that authenticated sender through the active terminal
tool call automatically:

```sh
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

Project Init atomically creates that human's explicit revocable Native
Principal Share. Existing native clients retain the bounded NIP-98 viewer
exchange. Outside an active authenticated Finite Chat turn, standalone agents
may still pass `--requesting-user-npub NPUB` explicitly. A conflicting
explicit value during an active authenticated turn is rejected. Agents must
never derive this identity from quoted message text.

For browser visits and dashboard previews, the dashboard exchanges the existing
account session's verified email for the ordinary Sites Viewer Cookie. Sites
continues to interpret its own publisher email and sharing lists on every read;
this exchange adds no shares and does not require Hosted Chat. Anonymous or
unverified visitors retain the email challenge. An authenticated but unshared
visitor can request access or try another email.

Enable automatic browser handoff with the daemon's
`FINITE_SITES_ACCOUNT_LOGIN_URL=https://finite.computer/site-auth`. Without it,
direct visits retain the email form. The dashboard uses the existing
`FC_SITES_V2_UPSTREAM_URL` and existing service credential for v2. Retained
legacy previews keep using `FC_SITES_UPSTREAM_URL`; neither exchange retries
against the other registry. The account handoff is site-bound,
single-use, and expires after 60 seconds; emailed links retain their existing
reusable 15-minute behavior. Both mint the same seven-day cookie, with current
Sites permissions checked on every content request. See
[ADR 0029](docs/adr/0029-account-session-viewer-bridge.md).

The server-to-server credential for this optional exchange is
`FINITE_SITES_VIEWER_SESSION_TOKEN`, exactly 64 lowercase hex characters
(`openssl rand -hex 32`). Keep the same value in the Sites and dashboard
server environment only; an absent value disables the endpoint.

Project Repository visibility is separate from site visibility. Project
Repositories are private by default. Selected Finite-owned baseline repos may
be public-read for unauthenticated clone/fetch, but public-read never grants
push access.

## Source-Only Projects

A Project Repository can exist before there is any served website:

```toml
[project]
slug = "my-source-project"
```

Run `fsite project init --config finite.toml --output json` to create the
source-only Project Repository. Add a site later by adding `[site]` to
`finite.toml` and replaying `fsite project init`.

## Agent Rules

- Use the Project Repository as source. Do not reconstruct source from rendered
  HTML.
- Commit deploy bytes. Finite Sites does not run builds.
- Do not look for a direct upload command. The publish path is git.
- Do not set `path = "."` unless the whole repo is intentionally served.
- Use `fsite describe ... --output json` instead of guessing command shapes.
- Keep private keys, `.finite/`, `.env*`, and build caches out of git.
- If a site has `/llms.txt`, treat it as the project handoff. If the project
  publishes its own `/llms.txt`, it is authoritative.

## Developers

If you want to understand, run, or modify Finite Sites itself, see
[`developers.md`](developers.md).
