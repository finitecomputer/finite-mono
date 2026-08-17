---
name: website-building-finite
description: Build and ship websites, landing pages, dashboards, documents, and stateful web apps on Finite. Use when a human wants the product implemented, QAed in a real browser, and published through Finite Sites with fsite.
---

# Website Building

Use this skill for the concrete delivery workflow: establish direction, scaffold,
build, reveal early, inspect in-browser, and publish through finite.

Read this skill when:
- the task is to build or ship a website, landing page, dashboard, internal tool UI, or browser game
- the task needs real browser QA with Playwright
- the task will be published through finite

If the main problem is art direction, visual quality, hierarchy, polish, or "make this feel designed," read the sibling `impeccable-finite` skill first, then return here for implementation.

## Finite Overrides

The `references/` tree was adapted from a broader web skill. On Finite, these
rules override anything else:

- publish through Finite Sites with `fsite`, never `deploy_website` or a
  Runtime port-exposure command
- keep Project Outputs private by default; public sharing requires an explicit
  human decision and `--yes-public`
- do not edit proxies, DNS, or host-level networking
- prefer user-space installs (`npm`, `bun`, `uv`, `pipx`) over asking for host binaries
- for interactive or data-heavy sites, use Playwright QA before publishing
- initialize git in new projects and commit after each meaningful milestone

## Fast First Reveal

Optimize for a fast, useful first reveal. Do not spend a long hidden turn polishing
an aesthetic direction the human has not seen or approved.

For a new site or substantial redesign:

1. Infer a credible initial direction from the prompt and existing brand context.
   When using `impeccable-finite`, form its compact direction card and carry that
   premise into the draft. Ask a question first only when the answer is truly blocking.
2. Build the smallest coherent draft that makes the direction tangible. Prefer a
   strong above-the-fold experience plus one representative section or state over
   a complete but generic site.
3. Run reveal QA only: confirm that the page starts, the primary view renders, and
   there are no fatal browser errors. Make at most one automatic correction pass.
4. Reveal the draft before exhaustive Playwright work. Include a screenshot or
   private preview when available, label what is provisional, and name 2-3
   meaningfully different aesthetic options. Recommend one and explain each option
   in concrete terms such as subject cues, signature, type, density, color, imagery,
   and motion—not vague labels alone.
5. Ask the human to choose a direction or react in their own words. Make the cost
   of the next step legible: distinguish a quick visual revision from completing
   the build and running full QA.

If the human already supplied precise art direction, still reveal the first coherent
draft early, but do not manufacture an unnecessary choice. If the task is a small,
well-specified edit, skip this checkpoint.

Do not silently turn the first reveal into a 10-30 minute visual self-critique loop.
The feeling of rapid creation is part of the product. Early human taste is more
valuable than repeated agent guesses, and revision gets more expensive as the site
becomes complete.

Read the sibling `finite-sites-publishing-finite` skill before creating,
updating, previewing, listing, or sharing any Project Output.

## Routing

Choose one route, then load the matching references.

- Informational / marketing / editorial:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/04-layout.md`, `references/shared/05-taste.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, and `references/informational/informational.md`.
- Dashboard / data-heavy app:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/04-layout.md`, `references/shared/05-taste.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, `references/shared/10-charts-and-dataviz.md`, `references/shared/12-playwright-interactive.md`, `references/shared/19-backend.md`, and `references/shared/20-llm-api.md` when applicable.
- Browser game / immersive interactive:
  Read `references/shared/01-design-tokens.md`, `references/shared/02-typography.md`, `references/shared/03-motion.md`, `references/shared/08-standards.md`, `references/shared/09-technical.md`, `references/shared/12-playwright-interactive.md`, plus `references/game/game.md` and companions as needed.

## Core Workflow

1. Identify the site type, product goal, known constraints, and initial art direction.
2. If design quality or direction is central to the request, read
   `impeccable-finite` and choose a coherent first lane without prolonged upfront
   questioning.
3. Research only what is needed to avoid a generic first direction.
4. Scaffold the project in its own folder and run `git init` if needed.
5. Build and reveal the smallest coherent draft using the Fast First Reveal rules.
6. Incorporate the human's aesthetic guidance, then complete the experience with
   real assets and a custom SVG logo when appropriate.
7. Commit after each meaningful milestone.
8. For complex sites, run progressive Playwright QA. Bound automatic correction
   loops and surface decisions or stubborn issues instead of polishing indefinitely.
9. Declare the correct `finite.toml` output kind, validate it with
   `fsite project init --dry-run`, then commit and push the Deploy Branch.
10. Verify the private served preview before sharing. Only switch to public
   after explaining the exposure and receiving explicit human agreement.

## Design Standards

- Avoid interchangeable AI-looking layouts.
- Use expressive typography and intentional spacing.
- Create visual rhythm with real imagery, diagrams, or illustration.
- Make dashboards feel like products, not admin templates.
- Treat screenshots as product review, not just bug checks.
- Use screenshots to align with the human early, not only as private evidence at
  the end of a long agent loop.

## Operational Rules

- Use a static `kind = "site"` output for built browser assets, a
  `kind = "document"` output for Markdown, and `kind = "app"` only when a
  server process or durable mutable state is required.
- If a site needs a backend, prefer one app server that serves both UI and API.
  For `kind = "app"`, listen on `0.0.0.0:$PORT` and write live mutable state
  only under `DATA_DIR`.
- Finite Sites does not run builds. Commit the source and selected deploy
  bytes or intentional app runtime payload before pushing.
- For Playwright QA, start the local preview with a truly detached process
  group, not a plain `nohup ... &` shell one-liner that can leave the terminal
  tool hanging. Prefer `setsid sh -lc 'COMMAND >/tmp/app.log 2>&1 < /dev/null'
  >/dev/null 2>&1 & echo $! >/tmp/app.pid`, then verify it with
  `curl http://127.0.0.1:PORT` before opening the browser.
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
