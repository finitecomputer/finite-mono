## Post-review interaction follow-up

- Parent issues: #26, #27, #28
- Fixed point before session: `341df32`
- Worker session: `/root/post_review_interaction_fixes`
- Status: completed

## Scope

- Preserve the non-secret Settings → Vault return token when a nested Manage
  Vaults flow resets its Vault session, without retaining it through a real
  Session Lock or page lifecycle lock.
- Make clipboard feedback kind-specific, short-lived, and race-safe without
  displaying copied IDs, invite fragments, or clipboard error details.
- Repair the Quick Switcher modal trap and focus collection semantics.
- Restore the invoking control when a context-menu Access action opens
  Settings.

## Implementation

- A reset initiated while the nested Manage Vaults dialog still owns its
  Settings return token restores that token after clearing session plaintext.
  Explicit Session Locks, pagehide, authorization-loss locks, and signer
  identity security locks opt out and discard it.
- Clipboard feedback is labeled by safe kind: Page ID, Folder ID, or
  client-only invite link. One generation/timer pair supersedes older feedback
  and expires the current label after a short interval. Clipboard completions
  publish only when their session epoch, feedback generation, and unlocked
  status are still current.
- Quick Switcher now traps Tab only at its first/last sequential control,
  restores the ribbon trigger on Escape, and suppresses the global Save and
  Quick Switcher shortcuts while it is modal. All modal/document focus
  collectors filter `tabindex=-1` roving options.
- Access routes restore context-menu trigger focus before Settings captures its
  return target, rather than storing a removed menuitem.

## Verification

- Passed: `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Passed: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Passed: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Passed: `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config --locked -- --nocapture`
- Passed: `scripts/with-dev-env cargo fmt --all --check`
- Passed: `scripts/with-dev-env cargo build -p finite-brain-app --locked`
- Passed: `git diff --check`

## Test seams

- A nested Settings → Vault → Manage Vaults flow changes Vaults, resets, then
  returns to Settings → Vault with focus on Manage Vaults; an explicit Session
  Lock clears that return token.
- Deterministic clipboard seams cover kind labels, expiry with a stale timer,
  deferred-copy completion after a lock, and deferred-copy completion after a
  newer copy or client action.
- Deterministic modal seams cover roving focus exclusion, native interior Tab,
  both wrap directions, Escape return focus, global shortcut suppression, and
  context-menu Access return focus.

## Independent final-review follow-up

- A final independent review found that an old generic error could otherwise
  reappear after a newer successful copy notice expired. A successful client
  action now supersedes that stale generic error, and the deterministic expiry
  seam proves the feedback hides rather than resurrecting it.

## Limits

- Browser and full-stack verification are left to the coordinating session,
  which owns the current local server rebuild/restart. No user-owned research
  material was changed.
