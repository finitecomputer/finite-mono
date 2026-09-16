# Technical Rules And Workflow

This file is authoritative for Finite. It overrides foreign
`deploy_website`, S3, iframe, opaque-URL, proxy, or Runtime port-exposure
assumptions from the source material.

## Project Structure

Create each site in its own project folder:

```text
project-name/
├── finite.toml
├── src/
├── dist/               # static output when applicable
├── assets/
└── package.json        # only when using a toolchain
```

Use a real app folder for React, Vite, Next, or another framework. The Project
Repository contains source, data, build logic, and the deploy bytes or runtime
payload collaborators need. `finite.toml` selects what Finite Sites serves.

## Platform Rules

- Publish with `fsite` and ordinary Git through a Finite Sites Project
  Repository.
- Keep Project Outputs private by default. Public sharing requires an explicit
  human decision and `--yes-public`.
- Do not edit proxies, DNS, host networking, or platform configuration.
- Use `kind = "site"` for static bytes, `kind = "document"` for Markdown, and
  `kind = "app"` for a server process or durable mutable state.
- Finite Sites does not run builds. Build and test before committing.
- Keep secrets server-side. Never commit `.env*`, `.finite/`, private keys,
  credentials, or build caches.
- Use relative asset paths inside static projects.
- External links should use `target="_blank" rel="noopener noreferrer"` when
  leaving the app.
- Install missing development dependencies in the project or user home rather
  than asking for host changes.

Read the sibling `finite-sites-publishing-finite` skill for the complete
publishing, identity, sharing, and recovery contract.

## Workflow

1. Pick the output type and art direction.
2. Research real references.
3. Scaffold the Project Repository and `finite.toml`.
4. Build the experience with real content and intentional assets.
5. Run unit, integration, accessibility, and browser checks appropriate to the
   project.
6. Preview locally at desktop and mobile sizes.
7. Validate the Project without mutation:

   ```sh
   fsite project init --config finite.toml --dry-run --output json
   ```

8. Create or reconcile the Project, mint a scoped Git Credential, then commit
   and push the configured Deploy Branch:

   ```sh
   fsite auth register --output json
   fsite project init --config finite.toml --output json
   fsite auth git PROJECT --store --output json
   git add finite.toml .
   git commit -m "Publish website update"
   git push origin main
   ```

9. Inspect and test the private served preview:

   ```sh
   fsite project status PROJECT --output json
   fsite view URL_OR_NAME --output json
   ```

10. Change viewer sharing only when the human asks. Never make an output
    public merely to preview it.

## Static Site Configuration

Use a dedicated output directory for built static assets:

```toml
[project]
slug = "project-name"

[outputs.site]
kind = "site"
site_name = "project-name"
branch = "main"
path = "dist"
spa = false
```

Set `spa = true` only when history-API routes need an index fallback. The
served site is the committed `dist/` snapshot, not a development server.

## Stateful App Configuration

Use an app output only when static bytes cannot provide the product:

```toml
[project]
slug = "project-name"

[outputs.web]
kind = "app"
site_name = "project-name"
branch = "main"
path = "app"
start = "bun server.ts"
```

The start command must use a supported runtime. Listen on `0.0.0.0:$PORT` and
write live mutable state only under `DATA_DIR`. Finite Sites preserves
`DATA_DIR` across deploys, restarts, and wake/sleep. Do not store live state in
the committed app directory.

## Recommended Local Preview

- Vite / React:
  `setsid sh -lc 'npm run dev -- --host 0.0.0.0 --port 3000 >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`
- Next.js:
  `setsid sh -lc 'npm run dev -- --hostname 0.0.0.0 --port 3000 >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`
- Plain static site:
  `setsid sh -lc 'npx serve . -l 3000 --no-clipboard --single >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`

Avoid `python -m http.server` beyond a minimal static check. Avoid plain
`nohup ... &` preview one-liners in Hermes terminal tools because they can
leave the terminal wrapper hanging. Verify the local server with
`curl http://127.0.0.1:PORT` before opening the browser, and stop it after QA.

## Backend Note

If the site needs backend logic, read `19-backend.md` and publish a
`kind = "app"` Project Output.

## Quality Checklist

- Research informed the design.
- Typography, color, spacing, and assets feel intentional.
- Interactive or data-heavy views were checked in a real browser.
- Mobile and desktop both look deliberate.
- The project's own tests and build passed before commit.
- The private served Version was tested after push, not only locally.
- Output visibility matches the human's explicit request.
