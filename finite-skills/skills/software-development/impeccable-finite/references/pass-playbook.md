# Impeccable Pass Playbook

Load this file when the redesign is substantial, the UI quality bar is high, or you need a deeper pass-specific checklist than the top-level skill body.

## Creation vs Refinement

Use `impeccable-finite` in two broad modes:

- creation: the interface direction is still unsettled and you need a strong lane before implementation
- refinement: the product exists, but it needs a sharper pass such as layout, distillation, polish, delight, or onboarding

Do not mix creation and final polish in the same pass. Decide what stage the work is in.

## Pass Selection

### Distill

Use this when the interface is cluttered, noisy, or trying to do too much.

Focus:
- remove competing actions and redundant information
- reduce visual variation
- keep the primary job obvious

Do not use distill as feature deletion. Quiet important capabilities rather than removing them blindly.

### Layout

Use this when the page technically works but feels spatially wrong.

Focus:
- spacing scale consistency
- tight spacing inside groups, generous spacing between groups
- hierarchy that works under the squint test
- fewer monotonous equal-card grids
- density matched to the content type

Layout problems often cause "this feels off" even when type and color are acceptable.

### Bolder

Use this when the design is too safe, timid, or forgettable.

Focus:
- one hero moment with stronger contrast
- clearer personality
- more decisive scale and hierarchy

Do not confuse bold with loud. Avoid generic "AI boldness" moves like neon gradients, glass everywhere, or dark-mode slop.

### Quieter

Use this when the UI is already over-signaling.

Focus:
- fewer accents
- more whitespace
- more disciplined color use
- calmer typography and interaction styling

If the interface is stressful, quieter is often the right move before any polish pass.

### Polish

Use this at the end, not the beginning.

Checklist:
- alignment is visually correct, not just mathematically centered
- spacing follows the system
- all states exist: hover, focus, active, disabled, loading, empty, error, success
- copy voice is consistent
- motion is smooth and reduced-motion aware
- there are no placeholder strings, rough edges, or stray TODO energy

Polish is for final refinement, not redesign.

### Delight

Use this only after the baseline is stable.

Focus:
- quick, non-blocking micro-interactions
- celebratory or empathetic moments in the right places
- small discoveries rather than constant theatrics

Delight should amplify usability, not compete with it.

### Onboard

Use this for first-run, empty-state, or activation work.

Focus:
- define the aha moment
- reduce time to first value
- replace dead empty states with orientation and next action
- teach at point of use instead of through ceremony

Most products need a better first screen, not a mandatory tour.

### Extract

Use this when the product has enough repeated UI to justify real design-system cleanup.

Focus:
- repeated literal values become tokens
- repeated component patterns become reusable primitives
- repeated layout patterns become composition rules

Only extract patterns that are actually stable and repeated.

## Recommended Sequencing

Common good sequences:

- distill -> layout -> polish
- bolder -> polish
- quieter -> polish
- onboard -> polish
- layout -> delight

Common bad sequences:

- delight before hierarchy is fixed
- polish before the feature is functionally complete
- extract before the patterns are stable
- bolder plus quieter in one pass

## Review Standards

When reviewing a design pass, ask:

- Is there a clear primary action or focal point?
- Does the page have rhythm, not just spacing?
- Is the interface emotionally appropriate to the audience?
- Are empty and loading states designed intentionally?
- Does the result feel specific to this product, or interchangeable with any AI-generated SaaS template?

If the result still feels generic, the pass was not strong enough.
