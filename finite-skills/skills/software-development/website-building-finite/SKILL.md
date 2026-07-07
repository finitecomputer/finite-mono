---
name: website-building-finite
description: Build and ship websites, landing pages, dashboards, and other web products on finitecomputer. Use this when a human wants the actual app built, QAed with Playwright, and published through finite.
---

# Website Building

Use this skill for the concrete delivery workflow: scaffold, build, run locally, inspect in-browser, and publish through finite.

Read this skill when:
- the task is to build or ship a website, landing page, dashboard, internal tool UI, or browser game
- the task needs real browser QA with Playwright
- the task will be published through finite

If the main problem is art direction, visual quality, hierarchy, polish, or "make this feel designed," read the sibling `impeccable-finite` skill first, then return here for implementation.

## Finite Overrides

The `references/` tree was adapted from a broader web skill. On finite, these rules override anything else:

- publish with `finitec publish`, never `deploy_website`
- private by default; public exposure requires one standalone human message containing exactly `MAKE PUBLIC`
- do not edit Traefik or invent host-level networking
- prefer user-space installs (`npm`, `bun`, `uv`, `pipx`) over asking for host binaries
- for interactive or data-heavy sites, use Playwright QA before publishing
- initialize git in new projects and commit after each meaningful milestone

Read the sibling `publish-web-apps-finite` skill before exposing a site.
Read `/platform/FINITE.md` before publishing.

## Routing

Choose one route, then load the matching references.

- Informational / marketing / editorial:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/04-layout.md`, `references/shared/05-taste.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, and `references/informational/informational.md`.
- Dashboard / data-heavy app:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/04-layout.md`, `references/shared/05-taste.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, `references/shared/10-charts-and-dataviz.md`, `references/shared/12-playwright-interactive.md`, `references/shared/19-backend.md`, and `references/shared/20-llm-api.md` when applicable.
- Browser game / immersive interactive:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/03-motion.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, `references/shared/12-playwright-interactive.md`, plus `references/game/game.md` and companions as needed.

## Core Workflow

1. Clarify the site type, product goal, and art direction.
2. If the design direction is unclear or weak, read `impeccable-finite` before implementing.
3. Research references first. Collect examples before designing.
4. Scaffold the project in its own folder.
5. Run `git init` if needed and commit after each major milestone.
6. Build the experience with real assets and a custom SVG logo.
7. For complex sites, run Playwright QA and iterate until the site actually looks good.
8. Publish privately first with `finitec publish expose --run ... --cwd ...`.
9. Only switch to `public` after an explicit `MAKE PUBLIC` confirmation.

## Design Standards

- Avoid interchangeable AI-looking layouts.
- Use expressive typography and intentional spacing.
- Create visual rhythm with real imagery, diagrams, or illustration.
- Make dashboards feel like products, not admin templates.
- Treat screenshots as product review, not just bug checks.

## Operational Rules

- Prefer one long-lived app process per published site.
- If the site needs a backend, run a real server and have it serve both the app and API when practical.
- When publishing, include `--run` and `--cwd` so the app survives runtime restarts.
- For Playwright QA, start the app with a truly detached process group, not a plain `nohup ... &` shell one-liner that can leave the terminal tool hanging. Prefer `setsid sh -lc 'COMMAND >/tmp/app.log 2>&1 < /dev/null' >/dev/null 2>&1 & echo $! >/tmp/app.pid`, then verify it with `curl http://127.0.0.1:PORT` before opening the browser.
- If the app uses Vite, remember that `vite preview` often needs `preview.allowedHosts` for the published hostname, not just `server.allowedHosts`.
- If Playwright is missing, install it in user space inside the project; do not ask for host packages unless user-space install genuinely fails.

## Reference Map

- `references/shared/01-design-tokens.md`: base tokens and CSS system
- `references/shared/02-typography.md`: font pairing and type rules
- `references/shared/03-motion.md`: animation and motion systems
- `references/shared/04-layout.md`: responsive structure and composition
- `references/shared/05-taste.md`: polish, empty states, and finishing passes
- `references/shared/06-css-and-tailwind.md`: Tailwind / CSS implementation patterns
- `references/shared/07-toolkit.md`: libraries and supporting tools
- `references/shared/08-standards.md`: accessibility, performance, anti-patterns
- `references/shared/09-technical.md`: finite-specific build, publish, and project rules
- `references/shared/10-charts-and-dataviz.md`: charts and dashboard patterns
- `references/shared/11-web-technologies.md`: compatibility notes
- `references/shared/12-playwright-interactive.md`: finite-specific Playwright workflow
- `references/shared/19-backend.md`: backend patterns for published apps
- `references/shared/20-llm-api.md`: using shared API keys safely
- `references/informational/informational.md`: informational / marketing site guidance
- `references/game/game.md`: browser game guidance
- `references/game/2d-canvas.md`: 2D canvas specifics
- `references/game/game-testing.md`: game QA
