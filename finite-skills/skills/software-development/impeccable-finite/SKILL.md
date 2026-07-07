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

If these are not obvious from the prompt or codebase, ask focused questions before redesigning the interface.

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

## Execution Heuristics

- Make one element the hero. Everything else should support it.
- Increase contrast in scale and weight before adding more decoration.
- Use fewer visual ideas with more conviction.
- Prefer purposeful typography over generic font stacks.
- Make dashboards feel like products, not generic admin templates.
- Design first-run, loading, success, and empty states on purpose.
- Add delight only when it does not block the user’s job.
- Match the emotional tone to the product. Banks, donor tools, and games should not all feel the same.

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
