# GPU serving context patch packet — 2026-08-07

## Patch frame

- Target: `finitecomputer/finite-mono`
- Concern: durable campaign backup and unambiguous DeepSeek scheduler handoff
- Patch type: documentation-only
- Branch: `context/gpu-lab-20260807-handoff`
- Evidence: pinned model reports and branches, exact current/candidate config
  diff and hashes, satellite release metadata, read-only Tinfoil state
- Grilling needed: no; this records observed state and measured decisions

## Findings

| Finding | Evidence | Routed artifact | Action |
| --- | --- | --- | --- |
| No single campaign index | Three separate pushed branches | Research handoff | Add compact durable map and outcome summary |
| Tonight is a scheduler update, not initial DeepSeek cutover | Current production is already retry-2-3 | New runbook | Fix scope, identity, gates, and rollback |
| Retry-2 status is stale | Live Tinfoil state | Existing retry-2 runbook | Add current-state supersession notice |
| Runbook index lacks scheduler promotion | New bounded procedure | Runbook index | Add one entry |

## Files changed

- `docs/research/2026-08-07-eight-h200-serving-campaign.md`: owns the campaign
  artifact map and cross-model disposition.
- `infra/runbooks/finite-private-deepseek-v4-flash-0731-scheduler-promotion.md`:
  owns tonight's bounded operational sequence and rollback boundary.
- `infra/runbooks/finite-private-deepseek-v4-flash-0731-retry-2.md`: retains
  historical preparation evidence while pointing current operations away from
  stale GLM-era status.
- `infra/runbooks/README.md`: owns runbook discovery.
- `infra/tinfoil/README.md`: owns the current satellite/enclave map.
- This packet and its ledger: own the context-loop evidence and parked work.

## Guardrails

- No runtime config, code, release, secret, host, or production state changes.
- Existing model reports remain measurement authority; summaries do not
  silently upgrade candidate evidence to production proof.
- The exact current production tag is immediate rollback authority.
- Any new target tag requires measurement and explicit rollout approval.

## Drift check

- Links: passed; every relative target exists.
- Paths: passed; referenced reports, candidate, helper, and status command exist.
- Commands: passed; helper syntax and prep/release-ready contracts passed.
- Contradictions: passed; historical cutover context and the Tinfoil index are
  separated from current scheduler-promotion state.
- Documentation-only scope: passed; only Markdown files changed.

## Parked work

- Production/release: publish and inspect a measured satellite release, then
  request explicit approval for the exact tag and maintenance window.
- Future context: update the handoff after the rollout with actual tag, hashes,
  gates, decision, and rollback result.
