# Stateful App Project Outputs

Stateful apps are an additive Project Output kind in `finite.toml`:

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

This keeps the Project Repository as the source of truth. Agents commit the
whole source tree collaborators need, including app source, migrations, seed
data, and any explicit runtime payload. Finite Sites does not run builds or
infer an output path. Pushing the configured Deploy Branch versions the
configured app directory as one app bundle.

## Decision

- `kind = "site"` and `kind = "document"` keep their existing behavior.
- `kind = "app"` is opt-in and requires `site_name`, `branch`, `path`, and
  `start`.
- App Outputs use the Site Base Domain and share the Site Name namespace with
  static site outputs. This avoids two different things answering at the same
  `{site_name}.finite.chat` URL.
- App `path` must be a directory. Single-file app deploys are rejected because
  the runtime contract is a directory-shaped bundle.
- `start` is one printable ASCII command line beginning with `node`, `bun`, or
  `uv`.
- At runtime Finite Sites sets `PORT` and `DATA_DIR`. The app must listen on
  `0.0.0.0:$PORT`; live mutable state must be written only under `DATA_DIR`.
- `DATA_DIR` survives deploys, restarts, and wake/sleep. Deploys must not
  overwrite it.

## Why

The product goal is agent collaboration over projects, not a second Vercel-like
build surface. Agents are already good at git and local build/test loops. The
platform should provide the repository, auth, versioning, sharing gate, and
runtime state boundary, while keeping the deploy input explicit.

## Non-Goals

- No direct app upload command.
- No automatic `npm install`, `bun install`, `cargo build`, or framework
  detection on the server.
- No hidden conversion from static site outputs to app outputs.
- No change to Document Output rendering or static site deploy semantics.

