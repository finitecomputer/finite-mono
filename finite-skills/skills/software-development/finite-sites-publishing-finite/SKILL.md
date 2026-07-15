---
name: finite-sites-publishing-finite
description: Operate Finite Sites (finite-sites) with the fsite CLI to create, publish, update, preview, inspect, list, and share site/website outputs, Markdown document outputs, and stateful app outputs. Use for Finite Sites or finite-sites requests involving a site or website publish, private preview, project/output list, viewer sharing, collaborative editing, documents, or stateful apps.
---

# Finite Sites Publishing

Use `fsite` as the only agent-facing Finite Sites surface. This skill follows
the `fsite` 0.4.0 Project Repository contract.

Finite Sites is source-first:

- a Project Repository is the editable Git source of truth;
- `finite.toml` declares zero or more Project Outputs;
- pushing a configured Deploy Branch creates an immutable Version;
- a Project Output can be a static site, rendered Markdown document, or
  stateful app;
- Project collaboration and output viewer access are separate grants.

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
fsite describe workflow publish-stateful-app --output json
fsite describe workflow publish-document --output json
fsite describe workflow edit-shared-project --output json
fsite describe workflow share-output --output json
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
their Native Principal identifier is the exact public-key account ID in
authenticated `event.source.user_id`. Pass that value unchanged as
`--requesting-user-npub` on both the dry-run and applied Project Init; `fsite`
accepts the 64-character account ID and normalizes it to an npub. This makes
Sites atomically create the human's explicit revocable viewer Share. Never
take an identity from quoted or typed message text, a profile lookup, an email
address, or the Agent Principal. If authenticated sender metadata is
unavailable, omit the flag; do not guess.

## Inspect, List, And Preview

List Projects and inspect one Project's outputs, URLs, visibility, and active
Version:

```sh
fsite project list --output json
fsite project status PROJECT --output json
fsite view URL_OR_NAME --output json
```

For a new or changed website, run its own tests and preview it locally in a
browser before pushing. After push, use the private output URL from `project
status` as the served preview, then verify it with `fsite view` and a real
browser. Do not make an output public merely to preview it.

Treat the server-returned `output_url` as authoritative. `fsite view NAME`
resolves an owned Project through the configured `FINITE_SITES_API`, so it may
return a local `*.sites.localhost` URL instead of `*.finite.chat`. Never
synthesize a production URL from a slug, and never present a Project Git
remote as a site preview. If the Project has multiple outputs, pass the exact
`output_url` to `fsite view`.

If an existing site or document exposes `/llms.txt`, read it for the platform
handoff. A project-authored `/llms.txt` remains the project's authority and
must not be overwritten by generic guidance.

## Choose The Output Kind

### Static Site Or Website

Use `kind = "site"` for committed static bytes. Finite Sites does not run the
build. Build and test locally, then commit the selected output directory.

```toml
[project]
slug = "my-project"

[outputs.site]
kind = "site"
site_name = "my-project"
branch = "main"
path = "dist"
spa = false
```

Set `spa = true` only for history-API client routing that needs unknown paths
to serve the app shell. Plain multipage sites and hash routing do not need it.

### Markdown Document

Use `kind = "document"` for one Markdown file or a Markdown tree rendered as a
read-only document. The authored Markdown remains the durable source.

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

Document URLs use the configured document domain, such as
`https://my-docs.docs.finite.chat/`. Clean routes render Markdown; `.md` routes
return authored Markdown. Do not commit generated HTML as the Document source.

### Stateful App

Use `kind = "app"` for a server process with live mutable state. The committed
app directory is an immutable runtime bundle; live state is separate.

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

Runtime contract:

- `start` is required and begins with a supported `node`, `bun`, or `uv`
  command;
- Finite sets `PORT`; listen on `0.0.0.0:$PORT`;
- Finite sets `DATA_DIR`; write all live mutable state under `DATA_DIR`;
- `DATA_DIR` survives deploys, restarts, and wake/sleep;
- build before commit and include the source, migrations, seed data, and any
  intentional runtime payload the start command needs;
- never let a deploy overwrite existing `DATA_DIR` content.

Do not assume Finite Sites runs dependency installation or a build step.
Commit dependency directories only when they are intentionally required
runtime payload, never as an accidental build cache.

## Create And Publish

1. Register the Agent Principal and validate the declared Project:

```sh
fsite auth register --output json
fsite project init --config finite.toml --requesting-user-npub AUTHENTICATED_SENDER_ID --dry-run --output json
```

2. After the configuration is correct, create or reconcile the Project and
   its outputs:

```sh
fsite project init --config finite.toml --requesting-user-npub AUTHENTICATED_SENDER_ID --output json
```

A `[project]`-only configuration creates a source-only Project Repository.
Adding outputs later and replaying `project init` is supported.

Project Init has one bounded recovery replay:

- `git_unavailable` means no Project Init state changed. Wait for service
  health to recover, then retry the exact command once.
- `git_repository_setup_failed` means the Project registry state may already
  be durable. Keep the same slug and local source. After the service operator
  repairs Git or repository storage, replay the exact Project Init command
  once; it repairs the repository without creating a duplicate Project.

Never blindly retry either error, choose a replacement slug, delete local
source, or attempt direct registry repair.

3. Mint a scoped Git Credential, then use ordinary Git:

```sh
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
cd PROJECT
# edit source, run tests/build, and inspect the local preview
git add finite.toml .
git commit -m "Publish Project update"
git push origin main
```

For a new local repository, initialize `main`, add the returned Project remote,
and push the configured Deploy Branch. Prefer `--store`; never print a Git
Credential password into chat or logs.

Pushing creates the Version. Confirm the expected output and private preview:

```sh
fsite project status PROJECT --output json
fsite view URL_OR_NAME --output json
```

Report the exact URL returned by those commands. Do not replace a local or
staging hostname with a production-shaped `*.finite.chat` hostname.

## Edit A Shared Project

Use the Project Repository; never reconstruct editable source from rendered
HTML.

For a native Project Collaborator:

```sh
fsite auth git PROJECT --store --output json
git clone https://git.finite.chat/PROJECT.git
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

Viewer access applies to one output ID from `finite.toml` or `project status`:

```sh
fsite project share PROJECT OUTPUT --shared --add-email viewer@example.com --send-invite --output json
fsite project share PROJECT OUTPUT --shared --remove-email viewer@example.com --output json
fsite project share PROJECT OUTPUT --private --output json
fsite project share PROJECT OUTPUT --add-npub VIEWER_NPUB --output json
fsite project share PROJECT OUTPUT --remove-npub VIEWER_NPUB --output json
```

Native Principal Shares use bounded Sites viewer sessions and do not require
email or Magic Links. Adding or removing a Share is authority; producing a
valid identity signature is only proof and must never create access.

Sites are private by default. Before public sharing, explain that anyone on the
internet will be able to view the output and confirm it contains no secrets,
private files, credentials, drafts, personal information, or regulated data.
Only after explicit human agreement run:

```sh
fsite project share PROJECT OUTPUT --public --yes-public --output json
```

Never pass `--yes-public` on your own initiative.

## Guardrails

- Use `fsite` for Finite Sites operations; do not edit platform networking or
  invoke a retired runtime-publish wrapper.
- Keep `.finite/`, `.env*`, private keys, credentials, and build caches out of
  Project Repositories.
- Treat output visibility separately from Project Repository edit access.
- Use the output ID, not a DNS name, with `project share`.
- Do not look for a direct upload command; Git push is the publish path.
- Do not set `path = "."` unless the entire repository is intentionally served.
- Do not claim state durability outside `DATA_DIR` for stateful apps.
- Treat rollback, output deletion, name transfer, and custom domains as
  operator work unless the installed `fsite` help explicitly exposes them.
