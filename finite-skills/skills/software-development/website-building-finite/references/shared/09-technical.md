# Technical Rules & Workflow

This file is authoritative for finitecomputer. It overrides any foreign `deploy_website`, S3, iframe, or opaque-URL assumptions from the source material.

## Project Structure

Create each site in its own project folder:

```text
project-name/
├── index.html
├── base.css
├── style.css
├── app.js
├── assets/
└── package.json        # only when using a toolchain
```

Use a real app folder for React/Vite/Next/etc. Do not scatter files directly into `/home/node/workspace`.

## Platform Rules

- Publish with `finitec publish`, never with custom infra edits.
- Private by default. `self` is the default auth mode.
- Public exposure requires one standalone human message containing exactly `MAKE PUBLIC`.
- Prefer one durable process per published site. Include `--run` and `--cwd` when publishing.
- Use relative paths for assets inside the project.
- External links should still use `target="_blank" rel="noopener noreferrer"` when leaving the app.
- If a project needs secrets or model keys, keep them server-side. Do not hardcode them into frontend JS.
- User-space installs are allowed. If you need a missing npm or Python package, install it in the project or home directory rather than asking for host changes.

## Workflow

1. Pick the site type and art direction.
2. Research real references.
3. Scaffold the project.
4. Run `git init` if needed.
5. Build the experience with real content and a custom SVG logo.
6. For complex sites, run Playwright QA before publishing.
7. Publish privately first.
8. Iterate using the same hostname and same project directory.

## Publishing

Reserve a hostname:

```bash
finitec publish reserve --label launch
```

Publish a static or dev-server-backed site:

```bash
finitec publish expose \
  --hostname HOSTNAME \
  --port 3000 \
  --run "npm run dev -- --host 0.0.0.0 --port 3000" \
  --cwd /home/node/workspace/project-name \
  --mode self
```

Important: if you started a temporary local QA server on that same port, stop it before running `finitec publish expose --run ...`. The published app runner should be the only long-lived process bound to the published port.

For Vite apps, do not stop at `host: '0.0.0.0'`. Published routes often need Vite to allow the external hostname in both the dev server and the preview server. The preferred Finite pattern is:

```js
import { defineConfig } from "vite";

const publishHostname = process.env.FC_PUBLISH_HOSTNAME;
const publishPort = Number(process.env.FC_PUBLISH_PORT || 3000);

export default defineConfig({
  server: {
    host: "0.0.0.0",
    port: publishPort,
    strictPort: true,
    allowedHosts: publishHostname ? [publishHostname] : undefined,
  },
  preview: {
    host: "0.0.0.0",
    port: publishPort,
    strictPort: true,
    allowedHosts: publishHostname ? [publishHostname] : undefined,
  },
});
```

`finitec publish` sets `FC_PUBLISH_HOSTNAME` and `FC_PUBLISH_PORT` for the durable published process. Local preview runs can still work without those variables.
The authoritative platform contract for those injected env vars and bind rules is mirrored at `$FC_PROFILE_ASSETS_ROOT/contracts/publish-runtime-contract.json`.

For pure static HTML, serving through a lightweight process is fine as long as the process is reproducible and long-lived.

Update an existing site by editing the same project and re-running `finitec publish expose` with the same hostname.

## Recommended Local Serving

- Vite / React:
  `setsid sh -lc 'npm run dev -- --host 0.0.0.0 --port 3000 >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`
- Next.js:
  `setsid sh -lc 'npm run dev -- --hostname 0.0.0.0 --port 3000 >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`
- Plain static site:
  `setsid sh -lc 'npx serve . -l 3000 --no-clipboard --single >/tmp/project-qa.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/project-qa.pid`

Avoid `python -m http.server` for anything beyond the simplest quick preview.
Avoid plain `nohup ... &` preview one-liners when using Hermes terminal tools; they can leave the terminal wrapper hanging even though the server started successfully.
For browser QA, verify the server with `curl http://127.0.0.1:3000` before opening Playwright.
After QA, clean up the temporary preview process before the final publish step if the publish command will reuse the same port and command.

## Backend Note

If the site needs backend logic, prefer a single server that can serve both the UI and API on one published port, or publish a dev server / app server that already does that. Read `19-backend.md`.

## Quality Checklist

- Research was done before design.
- Typography, color, and spacing feel intentional.
- There is a custom inline SVG logo.
- Interactive or data-heavy views were checked with Playwright.
- Mobile and desktop both look deliberate.
- The published route was tested after publish, not just locally.
- The site is private unless the human explicitly confirmed `MAKE PUBLIC`.
