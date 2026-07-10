# Managed skills are bundled and explicitly synced

Status: accepted.

`finite-skills/` in the monorepo is the only editable source for Finite-managed
skills. Every new Agent Runtime ships with a tested baseline, while existing
agents update at their own pace through an explicit `finite skills sync`
operation that does not replace the Runtime or force a reboot. Core, RMP, and
Runner do not own desired skills state, transport bundles, poll for updates, or
push rollouts. This deliberately trades automatic fleet convergence for a thin
runtime boundary and faster product iteration; a failed sync must leave the
previous baseline usable, and user-owned skills remain untouched.
