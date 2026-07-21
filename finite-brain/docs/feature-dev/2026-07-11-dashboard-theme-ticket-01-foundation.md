## Parent

#4

## What to build

Establish the Dashboard-Aligned Product Theme foundation so the real
Rust-served Product Client has the Finite dashboard's local typography,
system-driven light and dark appearances, and coherent shell styling while all
existing Product Client workflows and security state remain unchanged.

## Acceptance criteria

- [ ] Funnel Sans, Funnel Display, and JetBrains Mono are served locally from the Product Client origin with no third-party requests.
- [ ] Explicit public-asset tests verify each font's response bytes, content type, and cache policy.
- [ ] A shared presentation token layer defines both light and dark dashboard-aligned colors, text roles, borders, radii, shadows, control dimensions, and semantic statuses.
- [ ] The base shell, ribbon, Brain controls, Session Lock, toolbars, buttons, fields, selects, checkboxes, focus states, disabled states, and locked workspace use the new visual language.
- [ ] The existing workspace geometry, responsive structure, DOM identifiers, event bindings, Session Lock behavior, and Product Client functionality remain unchanged.
- [ ] The locked Product Client is visually verified at representative desktop and mobile widths in both light and dark modes.
- [ ] Targeted Product Client and Rust public-route checks pass.

## Blocked by

None — can start immediately.
