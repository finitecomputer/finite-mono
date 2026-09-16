---
name: finite-sites-publishing-finite
description: Operate Finite Sites (finite-sites) with the fsite CLI to create, publish, update, preview, inspect, list, and share static site/website projects. Use for Finite Sites or finite-sites requests involving a website publish, private preview, project list, viewer sharing, or collaborative editing.
---

# Finite Sites Publishing

Use `fsite` as the only agent-facing Finite Sites surface. This skill follows
the `fsite` 0.6.0 static Sites contract.

Finite Sites is source-first:

- a Project Repository is the editable Git source of truth;
- `finite.toml` declares zero or one Project Site;
- pushing a configured Deploy Branch creates an immutable Version;
- a Project Site serves committed static files; builds run locally;
- Project collaboration and Site viewer access are separate grants.

Do not expose users to raw Nostr events, private keys, manifests, blobs, DNS,
certificates, proxies, or host networking during normal work. If `fsite` is
missing or its installed version does not support a required command, stop and
report that the Finite Sites surface is unavailable or outdated. Do not invent
a platform fallback.

## Discover Before Mutating

Prefer the CLI's machine-readable guidance over remembered command shapes:

```sh
fsite --version
fsite describe workflow register-and-publish --output json
fsite describe workflow project-config --output json
fsite describe workflow publish-static-site --output json
fsite describe workflow edit-shared-project --output json
fsite describe workflow share-site --output json
```

Use `--output json` when inspecting results programmatically. Validate
`finite.toml` with `--dry-run` before creating or changing product state.

## Identity And Recovery

`fsite` uses the current Finite Home's Local Identity Key at
`$FINITE_HOME/identity/identity.json`, or
`~/.finite/identity/identity.json` when `FINITE_HOME` is unset. An Agent
Runtime uses its own Agent Principal; it does not adopt the human user's
Finite Chat key.

```sh
fsite auth status --output json
fsite auth register --output json
```

Registration is replay-safe. If publishing says the key has no active grant,
register and retry once. Never print, paste, move, commit, or upload identity,
verified-email, Git Credential, or token files.

Loss of the sole Publishing Key can strand owner access to private Projects.
Before treating a Project as durable, preserve an independent collaborator or
use a tested product recovery flow. Operator database edits are not recovery.

Email proof and the Agent Principal are different authorities. Use
`--link-native` only when the verified email and current Local Identity Key
intentionally identify the same Principal. An Agent may instead act through an
External Principal email grant when the human explicitly authorizes using that
connected mailbox for the task. This does not link the email to the Agent
Principal or grant Finite Brain authority.

When a publish request originates from an authenticated Finite Chat human,
`fsite` consumes the turn-scoped authenticated requester lease automatically.
The lease may carry the exact human public-key account ID plus a short-lived
Sites assertion for the WorkOS-verified mailbox. Never copy either value from
quoted or typed message text, a profile lookup, or an Agent NIP-05. A
standalone CLI may supply `--requesting-user-npub` and `--owner-email`
explicitly, but the email only disambiguates a Publishing Key that Sites
already authorizes; it is not proof by itself.

## Inspect, List, And Preview

List Projects and inspect a Project Site's URL, visibility, and active
Version:

```sh
fsite project list --output json
fsite project status PROJECT --output json
fsite view URL_OR_NAME --output json
```

For a new or changed website, run its own tests and preview it locally in a
browser before pushing. After push, use the private Site URL from `project
status` as the served preview, then verify it with `fsite view` and a real
browser. Do not make an output public merely to preview it.

Treat `site.url` from Project Status as authoritative. `fsite view NAME`
resolves a Project through `FINITE_SITES_API` (production: `https://finite.site`).
Use the returned URL; do not turn a local development URL into a production
URL or present a Git Remote as a browser preview.

If an existing site or document exposes `/llms.txt`, read it for the platform
handoff. A project-authored `/llms.txt` remains the project's authority and
must not be overwritten by generic guidance.

## Static Site Configuration

Build locally, then commit source and the selected deploy directory:

```toml
[project]
slug = "my-project"

[site]
name = "my-project"
branch = "main"
path = "dist"
spa = false
```

Use `spa = true` only when browser history routing needs an index-page
fallback. A `[project]`-only configuration creates a Bare Project Repository;
add `[site]` and replay Project Init to add its website.

Finite Sites does not run application servers or render Markdown. Export
static HTML for documents. If the product needs a backend, establish a
separately supported backend deployment before promising a working product;
never put server credentials in browser assets.

## Create And Publish

1. Register the Agent Principal and validate the declared Project:

```sh
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
```

2. After the configuration is correct, create or reconcile the Project and
   its Site:

```sh
fsite project init --config finite.toml --output json
```

If Project Init returns `requester_email_required`, stop and ask the human
which real mailbox should own the Project. Never use the Agent NIP-05
(`clanker-…@finite.vip`) as an email. For a standalone CLI, verify the mailbox
and add the current key first:

```sh
fsite auth sites-key request OWNER_EMAIL
fsite auth sites-key add OWNER_EMAIL TOKEN
fsite project init --config finite.toml --owner-email OWNER_EMAIL --dry-run --output json
fsite project init --config finite.toml --owner-email OWNER_EMAIL --output json
```

A `[project]`-only configuration creates a source-only Project Repository.
Adding a Site later and replaying `project init` is supported.

Project Init has one bounded recovery replay:

- `git_unavailable` means no Project Init state changed. Wait for service
  health to recover, then retry the exact command once.
- `git_repository_setup_failed` means the Project registry state may already
  be durable. Keep the same slug and local source. After the service operator
  repairs Git or repository storage, replay the exact Project Init command
  once; it repairs the repository without creating a duplicate Project.

Never blindly retry either error, choose a replacement slug, delete local
source, or attempt direct registry repair.

3. Mint a scoped Git Credential, then use ordinary Git. Set `GIT_REMOTE_URL`
   to the returned `git_remote_url`; do not guess a hostname:

```sh
fsite auth git PROJECT --store --output json
git clone "$GIT_REMOTE_URL"
cd PROJECT
# edit source, run tests/build, and inspect the local preview
git add finite.toml .
git commit -m "Publish Project update"
git push origin main
```

For a new local repository, initialize `main`, add the returned Project remote,
and push the configured Deploy Branch. Prefer `--store`; never print a Git
Credential password into chat or logs.

Pushing creates the Version. Confirm the expected Site and private preview:

```sh
fsite project status PROJECT --output json
fsite view URL_OR_NAME --output json
```

Report the exact URL returned by those commands. Do not replace a local or
staging hostname with a production-shaped `*.finite.site` hostname.

## Edit A Shared Project

Use the Project Repository; never reconstruct editable source from rendered
HTML.

For a native Project Collaborator, use the returned `git_remote_url` as
`GIT_REMOTE_URL`:

```sh
fsite auth git PROJECT --store --output json
git clone "$GIT_REMOTE_URL"
```

For an External Principal acting through an email grant:

```sh
fsite auth login editor@example.com
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL --output json
fsite auth git PROJECT --email editor@example.com --store --output json
```

After `auth login`, use the Google Workspace skill to retrieve the newest Sites
token when the human says the connected mailbox has access or tells the Agent
to get the code. Verify the connected address matches, redeem without printing
the token, and ask the human only if access is missing, mismatched, or ambiguous.

Run the project's own checks and build, commit source plus deploy bytes, and
push the Deploy Branch.

## Collaborators And Viewer Sharing

Project collaborator access controls clone and push:

```sh
fsite project grant PROJECT --email editor@example.com --send-invite --output json
fsite project revoke PROJECT --email editor@example.com --output json
```

Viewer access applies to the Project's single Site, with no output ID:

```sh
fsite project share PROJECT --shared --add-email viewer@example.com --send-invite --output json
fsite project share PROJECT --shared --remove-email viewer@example.com --output json
fsite project share PROJECT --private --output json
fsite project share PROJECT --add-npub VIEWER_NPUB --output json
fsite project share PROJECT --remove-npub VIEWER_NPUB --output json
```

Native Principal Shares use bounded Sites viewer sessions and do not require
email or Magic Links. Adding or removing a Share is authority; producing a
valid identity signature is only proof and must never create access.

Sites are private by default. Before public sharing, explain that anyone on the
internet will be able to view the output and confirm it contains no secrets,
private files, credentials, drafts, personal information, or regulated data.
Only after explicit human agreement run:

```sh
fsite project share PROJECT --public --yes-public --output json
```

Never pass `--yes-public` on your own initiative.

## Guardrails

- Use `fsite` for Finite Sites operations; do not edit platform networking or
  invoke a retired runtime-publish wrapper.
- Keep `.finite/`, `.env*`, private keys, credentials, and build caches out of
  Project Repositories.
- Treat Site visibility separately from Project Repository edit access.
- Use the Project Slug with `project share`.
- Do not look for a direct upload command; Git push is the publish path.
- Do not set `path = "."` unless the entire repository is intentionally served.
- Treat rollback, Site deletion, name transfer, and custom domains as
  operator work unless the installed `fsite` help explicitly exposes them.
