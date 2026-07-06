---
name: finite-sites-publishing
description: Publish, update, and share websites through Finite Sites using `fsite`, without exposing users to nostr, npubs, keys, manifests, blobs, DNS, or proxies.
---

# Finite Sites Publishing

Use this skill when a human asks you to publish, deploy, update, or share a
website. Finite Sites hosts static sites at:

```text
https://NAME.finite.chat/
```

Sites are **private by default**. Sharing works like a Google Doc: private
to the owner, shared with specific email addresses, or public.

Do not explain or expose nostr, npubs, secrets, signing, manifests, blobs,
DNS, certificates, Caddy, or Traefik unless you are debugging a platform
issue. Normal publishing happens only through `fsite`.

## Prerequisites

- `fsite` is available in the runtime. It defaults to the hosted Finite Sites
  API; set `FINITE_SITES_API` only for a local or self-hosted server.
- Run `fsite auth register --output json` before creating new Projects. It is
  safe to replay; `registered=false` means this User Key is already a
  Publishing Principal.
- The site can be represented as committed files selected by `finite.toml`.
  That path is often a final build output directory such as `dist/`, but a
  deploy-only project may intentionally use `path = "."`.
- The requested name is a lowercase DNS label of 3–63 characters, such as
  `demo`, `pauls-blog`, or `launch-2026`.

If `fsite` is missing or a command is unsupported, stop and say the Finite
Sites command surface is not available in this runtime. Do not fall back to
raw nostr tooling, DNS, or proxy configuration.

If a create/publish command says the pubkey has no active publish grant, run
`fsite auth register --output json` and retry the original command. Do not ask
for an operator allowlist unless registration itself fails.

## Key Hygiene

`fsite` creates key files automatically:

- identity (User Key): `~/.finite/identity/identity.json`, or
  `$FINITE_HOME/identity/identity.json` when `FINITE_HOME` is set. This is
  the shared Finite identity used by every Finite tool; whichever tool runs
  first mints it.
- verified email identities: `~/.config/finite-sites/emails/*.env`
- pending email-link markers: `~/.config/finite-sites/email-links/*.env`

Never print, paste, move, commit, or deploy these files.

The legacy `~/.config/finite-sites/identity.env` location is no longer read.
If a human wants to keep an old key, run
`fsite auth import --file ~/.config/finite-sites/identity.env`
once; otherwise a fresh identity is minted on first run. `fsite auth status`
shows the identity in use. `fsite auth import` reads the secret from stdin
or `--file`, never from a flag value.

## Project Shape

Treat a Finite project as source first and output second:

- durable data is the foundation;
- add logic around that data only when the project needs computation;
- build a website, PDF, or other user-facing output only when there is
  something useful to present.

Keep those layers in the Project Repository. The deployed site is a Deploy
Output: committed bytes selected by `finite.toml` and served as a Version.
Finite Sites validates and serves the bytes; the agent owns any build step
that produces them.

## Project Repository Workflow

Prefer this flow for collaborative sites:

1. Learn the schema and workflows from the CLI:

```bash
fsite describe workflow register-and-publish --output json
fsite describe workflow project-config --output json
fsite describe workflow publish-static-site --output json
fsite describe workflow edit-shared-project --output json
fsite describe workflow share-output --output json
```

2. Create or update `finite.toml`. Validate before mutating:

```bash
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

Project Init creates the Project Repository and declared Project Outputs from
`finite.toml`. A `[project]`-only config creates a source-only Project
Repository with no served output. Init is replay-safe when existing outputs
match, and it may add missing outputs to the same Project.

3. Grant editors, mint scoped Git Credentials, and use standard git:

```bash
fsite project grant PROJECT --email editor@example.com --send-invite --output json
fsite auth register --output json
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL --link-native --output json
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
cd PROJECT
# edit source/data/logic, run tests/builds, commit deploy bytes
git push origin main
```

If you need to link an email but do not already have a token, run
`fsite auth link-email EMAIL --output json`, then redeem the new token with
`--link-native`.

For an External Principal that should remain email-only, use
`fsite auth login EMAIL`, `fsite auth redeem EMAIL TOKEN`, and
`fsite auth git PROJECT --email EMAIL --store --output json`.

Pushing the configured Deploy Branch creates a Version from committed bytes.
Finite Sites does not run builds.

If the local User Key or Agent Key is already a native Project Collaborator,
omit `--email`:

```bash
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
```

To remove a Project Collaborator, use the Project owner identity and revoke:

```bash
fsite project revoke PROJECT --email editor@example.com --output json
```

Check `removed` and `revoked_git_credentials` in the JSON response. The
command is replay-safe. If that email should also lose viewer access to a
Project Output, remove that share separately:

```bash
fsite project share PROJECT OUTPUT --shared --remove-email editor@example.com --output json
```

## Initial Project Publish Workflow

Use this flow for new sites and edits.

1. Identify the Project Slug, Output ID, and Site Name. Check what already
   exists when useful:

```bash
fsite project list --output json
fsite project status PROJECT --output json
```

2. Create or update `finite.toml`. Learn the schema from the CLI instead of
   guessing:

```bash
fsite auth register --output json
fsite describe workflow project-config --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
```

3. Clone the Project Repository, commit the source and deploy bytes selected
   by `finite.toml`, and push the Deploy Branch:

```bash
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
cd PROJECT
# edit source/data/logic, run tests/builds, commit deploy bytes
git push origin main
```

If the site is a single-page app with client-side routing (React Router,
Vue Router, etc. using history-API URLs like `/settings`), set `spa = true`
on that Project Output in `finite.toml` so unknown paths serve the app shell
instead of 404.

Plain multi-page sites and hash-routed apps do not need `--spa`.

Pushing the Deploy Branch creates a new Version. Tell the human the URL when
the deploy succeeds, and that the site is currently private.

4. Share it the way the human asked:

```bash
fsite project share PROJECT OUTPUT --shared --add-email friend@example.com --send-invite --output json
fsite project share PROJECT OUTPUT --shared --remove-email friend@example.com --output json
fsite project share PROJECT OUTPUT --private --output json
fsite project share PROJECT OUTPUT --public --yes-public --output json
```

People shared by email sign in with a magic link sent to that address —
no account or password.

## Collaborative Editing

Grant edit access with `fsite project grant`. Editors verify email when they
are using an External Principal, mint a Git Credential, clone, commit, and
push. Native Project Collaborators can mint a Git Credential without an email
round trip. Do not scrape rendered HTML as source and do not look for a source
archive.

If `https://NAME.finite.chat/llms.txt` exists and is platform-generated, use
it as the handoff guide. If the project contains its own `llms.txt`, preserve
it and follow it as project-specific guidance.

Project-backed generated `/llms.txt` uses explicit auth before clone/push
rather than hidden credential-helper behavior.

## Agent-Safe CLI Direction

Prefer machine-readable `fsite` surfaces when they exist. The CLI should
document every capability through inspectable commands, not rely on hidden
external docs. For new project commands, prefer JSON input/output and dry-run
validation before mutation.

## Server Apps (tier 2)

Server apps are not part of the current agent-facing publish surface. If the
site needs a database, API routes, or server rendering, explain that Finite
Sites currently accepts committed static Deploy Outputs and that app outputs
need a future Project Output type.

## Public Warning

Before making any site public, warn clearly and get agreement:

```text
This will make https://NAME.finite.chat/ public. Anyone on the internet
can view it. Do not include secrets, private files, personal information,
credentials, drafts, or anything you would not want public.
```

Only after the human agrees, run the command with `--yes-public`. Never
pass `--yes-public` on your own initiative. For updates to an
already-public site, warn again only when the new content appears
personal, confidential, regulated, or otherwise sensitive.

## Out Of Scope

Rollback, deleting a site, releasing or transferring a name, and custom
domains are operator actions for now. If asked, say so and offer to note
the request for a Finite operator.
