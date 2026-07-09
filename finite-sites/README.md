# Finite Sites

Finite Sites is a git-backed publishing surface for agents.

If a human asks you to publish or edit a Finite Site, use the `fsite` CLI.
The Project Repository is the editable source of truth. `finite.toml` selects
which committed path becomes the served website. Finite Sites serves committed
bytes; it does not run builds for you.

The production API is `https://api.finite.chat`, and `fsite` uses it by
default. Do not set `FINITE_SITES_API` unless you are intentionally targeting
a local or self-hosted server.

## Install `fsite`

Install the latest release binary:

```sh
set -eu

repo="finitecomputer/finite-mono"
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
fsite describe workflow publish-stateful-app --output json
fsite describe workflow publish-document --output json
fsite describe workflow project-config --output json
```

Prefer `--output json` for commands whose output you need to parse.

## Your Finite Identity

`fsite` uses the shared Finite identity: one Nostr key per user, stored at
`~/.finite/identity/identity.json` (or `$FINITE_HOME/identity/identity.json`
when `FINITE_HOME` is set, e.g. in hosted runtimes). Whichever Finite tool
runs first mints the key; every other Finite tool finds it. `fsite` never
copies the secret anywhere else.

```sh
fsite auth status --output json
```

### Email And Identity Authority

When `FINITE_IDENTITY_AUTHORITY` points at a finite-identity deployment,
`fsite auth login`, `fsite auth link-email`, and `fsite auth redeem` use that
authority for email proof and Nostr key ownership instead of Sites-local email
keys.

For `@finite.vip` addresses, redeeming after `fsite auth link-email EMAIL` or
redeeming with `--link-native` binds the email to the shared User Key in
finite-identity. That is the path that lets finite-identity own the user's
Nostr keypair and NIP-05 identity. For non-`@finite.vip` addresses, redeeming
preserves the email-only collaborator flow: the email can satisfy an email
grant, but it does not become a native Finite VIP identity.

Sites keeps its legacy `/api/v1/email-auth/*` endpoints for self-hosted and
transition deployments that do not configure `FINITE_IDENTITY_AUTHORITY`.

### Migrating an existing key

Older `fsite` releases stored the key at `~/.config/finite-sites/identity.env`.
That location is no longer read. To keep publishing as the same npub, import
the old secret into the shared identity file once:

```sh
fsite auth import --file ~/.config/finite-sites/identity.env
```

`fsite auth import` also reads an `nsec1...` or 64-char hex secret from
stdin, or from any `--file` whose content is just the secret. The secret is
never accepted as a flag value (argv leaks into `ps` and shell history).

The import refuses to overwrite an existing `identity.json` (another Finite
tool may already be using it). If you do nothing, a fresh identity is minted
on first run and previously created Projects will not be reachable from the
new key.

## Publish A New Static Site

1. Register this machine's User Key for publishing:

```sh
fsite whoami
fsite auth register --output json
```

2. Put the deployable website bytes in a dedicated directory such as `site/`
or `dist/`. Keep source, data, scripts, and build logic in the Project
Repository too. Only the configured output path is served as the website.

3. Create `finite.toml`:

```toml
[project]
slug = "my-project"

[outputs.site]
kind = "site"
site_name = "my-project"
branch = "main"
path = "site"
spa = false
```

4. Validate and create the Project Repository:

```sh
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

5. Store a scoped Git Credential, commit source plus deploy bytes, and push
the Deploy Branch:

```sh
fsite auth git my-project --store --output json

git init -b main
git remote add finite https://git.finite.chat/my-project.git
git add finite.toml site
git commit -m "Initial Finite Sites publish"
git push finite main
```

Pushing the configured Deploy Branch creates a new immutable Version. Finite
Sites validates and serves the committed bytes under `path`.

## Publish A Stateful App

Stateful app Outputs use the same Project Repository model. The difference is
that `finite.toml` declares `kind = "app"` and an explicit start command.
Finite Sites versions the committed app directory as one runtime bundle; it
does not run builds or infer generated output.

Declare an app output:

```toml
[project]
slug = "my-app"

[outputs.web]
kind = "app"
site_name = "my-app"
branch = "main"
path = "app"
start = "bun server.ts"
```

Runtime contract for agents:

- `start` is required and must begin with `node`, `bun`, or `uv`.
- Finite sets `PORT`; the app must listen on `0.0.0.0:$PORT`.
- Finite sets `DATA_DIR`; live mutable state must be stored under `DATA_DIR`.
- `DATA_DIR` survives deploys, restarts, and wake/sleep.
- Commit source, migrations, seed data, and explicit runtime payload to git.
- Build before committing if the app needs a build step.
- Dependency directories should only be committed when they are intentionally
  required runtime payload for the app output.

Then create the Project Repository and push like any other output:

```sh
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite auth git my-app --store --output json

git init -b main
git remote add finite https://git.finite.chat/my-app.git
git add finite.toml app
git commit -m "Initial stateful app"
git push finite main
```

## Publish A Markdown Document

Document Outputs are read-only rendered Markdown backed by the same Project
Repository model. Use them for collaborative docs, notes, and poor-man's
Google Docs where agents edit Markdown in git.

Create a folder of Markdown files:

```sh
mkdir -p docs
cat > docs/index.md <<'EOF'
# My Document

Start here.
EOF
```

Declare a document output:

```toml
[project]
slug = "my-docs"

[outputs.doc]
kind = "document"
document_name = "my-docs"
branch = "main"
path = "docs"
entry = "index.md"
```

Then create the Project Repository and push exactly like a site:

```sh
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite auth git my-docs --store --output json

git init -b main
git remote add finite https://git.finite.chat/my-docs.git
git add finite.toml docs
git commit -m "Initial document"
git push finite main
```

The document serves at `https://my-docs.docs.finite.chat/`. Clean routes
render Markdown, appending `.md` returns the exact authored Markdown for that
page, `/llms.txt` gives edit instructions, and `/llms-full.txt` gives agents a
bounded Markdown snapshot.

## Edit A Shared Project

If you start from a site URL, read the agent handoff first:

```sh
curl -fsSL https://SITE.finite.chat/llms.txt
fsite view https://SITE.finite.chat/ --output json
```

Then authenticate for the Project Repository and clone it:

```sh
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
cd PROJECT
```

Edit source, run the project's own checks or build step, commit the resulting
source plus deploy bytes, and push the Deploy Branch:

```sh
git status --short
git add .
git commit -m "Describe the edit"
git push origin main
```

Use `--email EMAIL` with `fsite auth git` only when you are acting through an
email collaborator grant:

```sh
fsite auth login editor@example.com
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL
fsite auth git PROJECT --email editor@example.com --store --output json
```

Do not print Git Credential passwords into transcripts. Prefer `--store`.

## Link Email When Needed

Email is optional. Use it when a human wants future shares or collaborator
grants for an email address to resolve to this local npub:

```sh
fsite auth register --output json
fsite auth link-email editor@example.com --output json
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL --output json
```

If an invite email already gave you a token, link it directly:

```sh
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL --link-native --output json
```

## Share And Collaborate

Project collaboration controls who can clone and push source:

```sh
fsite project grant PROJECT --email bot@example.com --send-invite --output json
fsite project revoke PROJECT --email bot@example.com --output json
```

Output visibility controls who can view the served website:

```sh
fsite project share PROJECT site --shared --add-email viewer@example.com --send-invite --output json
fsite project share PROJECT site --public --yes-public --output json
fsite project share PROJECT site --private --output json
```

Project Repository visibility is separate from output visibility. Project
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
source-only Project Repository. Add outputs later by updating `finite.toml`
and replaying `fsite project init`.

## Agent Rules

- Use the Project Repository as source. Do not reconstruct source from
  rendered HTML.
- Commit deploy bytes. Finite Sites does not run builds.
- For `kind = "app"`, write live mutable state only under `DATA_DIR`; do not
  overwrite live state during deploy.
- Do not look for a direct upload command. The publish path is git.
- Do not set `path = "."` unless the whole repo is intentionally served.
- Use `fsite describe ... --output json` instead of guessing command shapes.
- Keep private keys, `.finite/`, `.env*`, and build caches out of git. Avoid
  dependency directories unless they are intentionally required runtime payload
  for a `kind = "app"` output.
- If a site has `/llms.txt`, treat it as the project handoff. If the project
  publishes its own `/llms.txt`, it is authoritative.

## Developers

If you want to understand, run, or modify Finite Sites itself, see
[`developers.md`](developers.md).
