---
name: impeccable-finite
description: Raise the design quality of websites, dashboards, and product UI on finitecomputer. Use this when a human wants stronger visual taste, clearer art direction, a redesign, or a polish pass before or during implementation.
---

# Impeccable Design

This is the finite adaptation of the `impeccable.style` design vocabulary. Use it as the design-intelligence layer: clarify the visual direction, critique weak UI, choose a lane, and run deliberate polish passes.

Read this skill when:
- the human says the UI feels generic, boring, flat, noisy, under-designed, or "AI"
- the work needs art direction before building
- the task is a redesign, polish pass, critique, or visual-system pass on an existing product
- the task is a marketing site, product page, dashboard, or onboarding flow that needs stronger taste

Use `website-building-finite` for the actual implementation, Playwright QA, and publish flow. This skill should shape the design decisions that the build skill then executes.

For a deeper redesign or a more detailed pass checklist, read `references/pass-playbook.md`.

## Finite-Specific Rules

- judge real UI from screenshots or a live browser, not just JSX or CSS diffs
- use this skill before big implementation passes when the design direction is unclear
- do not treat "more effects" as better design
- avoid defaulting to purple gradients, glass, neon-on-dark, or generic SaaS card grids
- when the design is already loud, the right move may be restraint, not escalation

## Context Gathering Protocol

Before changing the design, gather these inputs:

1. Product type: marketing, editorial, product UI, dashboard, onboarding, internal tool.
2. Audience: consumer, executive, analyst, creative, operator, student, donor.
3. Brand personality: restrained, warm, technical, editorial, playful, premium, institutional.
4. Risk budget: conservative refresh, noticeable redesign, or high-conviction new direction.
5. Constraints: accessibility, existing brand colors, performance, device mix, dense data, approval process.
6. Current weakness: bland, cluttered, timid, inconsistent, visually loud, poor hierarchy, weak empty states.

Infer these from the prompt and codebase when possible. If uncertainty is aesthetic
rather than blocking, do not hold the first draft hostage to a questionnaire. Pick a
coherent recommended lane, make it tangible quickly, and offer 2-3 meaningfully
different directions when revealing it. Ask a focused question before building only
when the answer changes the product contract or makes a first draft unsafe or wasteful.

## Create A Direction Card

Before implementing a new visual direction, write a compact direction card:

1. Subject, audience, and the page's single job.
2. Visual source: the real materials, artifacts, language, environment, or
   conventions that belong to this subject.
3. Type and palette character, described by intent rather than a large token dump.
4. Layout principle: how the information should move, group, or unfold.
5. Signature: one memorable element rooted in the brief and one aesthetic risk
   worth taking.

Keep the card short enough to form in one quick pass. Use it to sharpen the first
draft, not to delay implementation with a long design document, exhaustive option
matrix, or internal critique loop. Treat it as a proposal until the human reacts to
the first reveal.

## Make The Design Specific

- Derive visual language from the subject's own world instead of choosing a style
  category in the abstract. Use real content early; generic copy produces generic
  composition.
- Make the hero express the page's central claim or experience. Do not automatically
  reach for a large headline, supporting metrics, feature cards, and an accent effect.
- Let structure communicate real relationships. Numbering implies sequence; grouping
  implies similarity; dividers imply boundaries. Do not use these devices as empty
  decoration.
- Spend boldness on the signature element and keep the supporting system disciplined.
  Match execution complexity to the direction: maximal work needs depth, while
  minimal work needs unusually precise type, spacing, and detail.
- Treat interface copy as visual and interaction design. Use the user's vocabulary,
  concrete nouns, active controls, and consistent action names. Make errors explain
  recovery and empty states point toward a next action.
- Tie major visual choices to concrete facts from the brief. Be able to explain why
  the hero, typography, layout, imagery, and signature fit this product and audience.
  If a choice has no reason beyond looking polished, simplify it or choose again.

## Audit Before You Change Anything

Check:

- hierarchy: what is the focal point, and is it strong enough?
- typography: are size, weight, and spacing doing real work?
- composition: is the page rhythm intentional or just stacked blocks?
- color: is there a clear palette with contrast and discipline?
- interaction: does motion support the task or distract from it?
- state design: are empty, loading, success, and error states designed or ignored?

Do not jump straight into implementation tweaks without first naming what is wrong.

## Pick A Lane

Pick one coherent direction instead of mixing five:

- bolder: more contrast, stronger focal point, clearer personality
- quieter: less noise, more restraint, fewer competing accents
- layout: restructure hierarchy, density, spacing, and grouping
- polish: alignment, spacing, state quality, copy quality, visual consistency
- delight: subtle moments of personality after the baseline is already solid
- onboard: first-run and empty-state clarity that gets users to value faster

Do one or two passes well. If you need more, sequence them. Example: bold first, then polish. Do not stack delight on top of an unstable layout.

For a new build, treat the first lane as a proposal, not a silent commitment. Reveal
it after a bounded smoke check and let the human redirect type, density, palette,
imagery, and motion before the full implementation and QA pass make revision costly.

## Execution Heuristics

- Make one element the hero and ensure it carries the page's thesis. Everything else
  should support it.
- Increase contrast in scale and weight before adding more decoration.
- Use fewer visual ideas with more conviction.
- Prefer purposeful typography over generic font stacks.
- Make dashboards feel like products, not generic admin templates.
- Design first-run, loading, success, and empty states on purpose.
- Add delight only when it does not block the user’s job.
- Match the emotional tone to the product. Banks, donor tools, and games should not all feel the same.

## Anti-Slop Checks

Before revealing or polishing a design, check:

- Make every container earn its boundary. Flatten nested cards and avoid using a
  card as the default unit for every piece of content. Prefer spacing, typography,
  alignment, and dividers when they communicate the hierarchy clearly.
- Use decorative structure only when it carries meaning. Eyebrows, pills, numbered
  labels, accent borders, badges, and editorial rules must communicate category,
  status, sequence, or another real relationship. Remove them when they merely make
  the page look designed.
- Build typographic hierarchy intentionally. Create meaningful contrast between
  display, body, label, and data text through family, scale, weight, width, or
  spacing. Do not choose a typeface merely because it is currently fashionable, and
  do not force additional font families when one family can provide sufficient
  character and hierarchy.
- Say each idea once. Remove labels, descriptions, helper text, and hints that repeat
  the same message. Replace generic claims, marketing buzzwords, and manufactured
  aphorisms with concrete verbs and nouns from the product's domain.
- Use credible imagery or leave the space clean. Do not substitute generic geometric
  collages, improvised SVG scenes, or hand-coded mascots for real art direction. Use
  intentional photography, illustration, generated artwork, diagrams, or product
  imagery. If the available asset weakens the design, omit it.
- Keep the visual system coherent. Reuse the chosen type scale, palette roles,
  spacing rhythm, corner-radius hierarchy, and surface treatment. Introduce an
  exception only when it strengthens the signature element or communicates a real
  product distinction.

## Anti-Patterns

- generic glassmorphism or neon gradients used as a substitute for taste
- gradient text on critical metrics
- everything medium-sized, medium-weight, and equally emphasized
- decorative motion that slows the user down
- default chart styling with no hierarchy work
- empty states with no next action
- trying to fix weak layout with shadows and effects

## Deliverables

Depending on the task, produce one or more of:

- a concise visual diagnosis of what is wrong
- a named art direction with 3-5 concrete design rules
- a prioritized redesign plan
- a polish checklist for the implementation pass
- screenshot-based feedback after a first build

When you move from diagnosis to implementation, hand off to `website-building-finite` or explicitly state that you are now doing the build pass.
