# Onboarding flow prototype

This development-only route visualizes a proposed first-agent onboarding flow.
It is intentionally not connected to billing, agent creation, or chat.

## Prototype map

- `page.tsx` keeps `/dev/onboarding` unavailable outside development.
- `onboarding-flow-preview.tsx` owns the four-screen preview harness.
- `finite-loader.tsx` and `finite-loader.module.css` provide the reusable Finite
  mark animation.
- `Button size="xl"` is the shared 48px application-action size.

## Production integration

Use the existing production flow instead of promoting the preview state machine:

- Submit the name and access choice through
  `/dashboard/agent-creation-requests`.
- Preserve the draft across Stripe with the existing signed, HTTP-only
  `finite-agent-draft` cookie in `lib/agent-onboarding.ts`. Do not add
  `localStorage`.
- Reuse the existing Stripe checkout and Launch Code branches in
  `CoreAgentCreationForm`.
- Drive post-payment progress from Core's durable agent-creation request
  statuses (`requested`, `launching`, `running`, `failed`) and the existing
  `PendingRefresh` loop.
- Only show the online screen after Core reports `running` and the project has a
  usable chat destination. The final CTA should link to that destination.

## Product-truth boundary

The five pre-payment checklist items and their timers are presentation-only.
Core currently exposes aggregate creation states, not durable phases for
workspace creation, box startup, skill loading, memory security, or channel
setup. Do not infer those phases from elapsed time or store them client-side.
