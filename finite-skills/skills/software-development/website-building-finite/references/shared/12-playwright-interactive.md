# Playwright QA on Finite

Use Playwright for complex sites: dashboards, multi-step flows, data-heavy apps, and anything where the agent needs to see the result and iterate.

## Rules

- Use Playwright for real browser QA, not just DOM inspection.
- Keep one server running and reload between edits instead of restarting constantly.
- Take screenshots at desktop and mobile.
- Check both functionality and visual quality.
- For simple one-page static sites, a careful manual code review may be enough. For anything richer, use Playwright.

## Setup

If Playwright is missing in the project:

```bash
npm install -D playwright
```

On Finite, prefer the system Chromium that ships in the runtime over Playwright's downloaded browser cache. The most reliable pattern is:

```js
import { chromium } from "playwright";

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || "chromium",
  args: ["--no-sandbox"],
});
```

If you need the CLI directly and the project does not already vendor Playwright, install the package first and then use either `npx playwright ...` or the durable global binary at `"$HOME/.npm-global/bin/playwright"`.

If the repo already uses Playwright, reuse its setup rather than inventing a second one.

## Start the App

Run the app locally on a known port with a detached process that survives separate tool calls.
On finite, plain backgrounding is not enough for multi-step QA loops.

Use a pattern like:

```bash
nohup npm run dev -- --host 0.0.0.0 --port 3000 \
  >/tmp/project-qa.log 2>&1 < /dev/null &
echo $! >/tmp/project-qa.pid
curl http://127.0.0.1:3000
```

or for a static project:

```bash
nohup npx serve . -l 3000 --no-clipboard --single \
  >/tmp/project-qa.log 2>&1 < /dev/null &
echo $! >/tmp/project-qa.pid
curl http://127.0.0.1:3000
```

Use `127.0.0.1`, not `localhost`, when opening the app in Playwright.

On Finite:

- Prefer a server bound to `0.0.0.0` for dev QA loops.
- Prefer system Chromium via `executablePath: process.env.CHROMIUM || "chromium"`.
- Pass `--no-sandbox` when launching Chromium inside the runtime container.
- Treat `npx playwright install chromium` as optional, not the primary path.

## QA Loop

1. Write a short QA inventory: key flows, important states, and the claims you expect to make.
2. Open the app in Playwright at desktop width first.
3. Exercise the main flow with real clicks and keyboard input.
4. Inspect visual quality, spacing, contrast, overflow, and hierarchy.
5. Repeat on a mobile viewport.
6. Fix issues and reload instead of restarting the server.
7. Capture the screenshots that support your claims.
8. Stop the detached QA process when you are done if it is no longer needed:

```bash
kill "$(cat /tmp/project-qa.pid)"
```

## Minimum Checks

- Desktop screenshot at 1280px or wider
- Mobile screenshot around 390px wide
- One end-to-end happy path
- One off-happy-path or error-state check
- One visual pass specifically looking for clipping, weak contrast, ugly spacing, and broken hierarchy

## Publishing Gate

Do not publish until:

- the local app loads cleanly
- the main interaction path works
- screenshots support the quality claim you want to make
- the app feels intentional on both desktop and mobile

## After Publish

Open the published hostname and run one more smoke pass. A site is not validated just because localhost worked.
