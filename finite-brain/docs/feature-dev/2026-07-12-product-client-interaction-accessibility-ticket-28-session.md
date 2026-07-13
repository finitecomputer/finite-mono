## Issue

- Issue: #28 — complete Product Client keyboard navigation
- Fixed point before session: `44c669e`
- Status: accepted

## Scope

- Make the Quick Switcher, context menu, Folder selector, relevant share and
  invitation forms, and Vault switcher predictable with keyboard input.
- Preserve the existing Settings and Access tab behavior while making focus
  movement, activation, Escape, and Tab semantics match the advertised UI
  roles.

## Acceptance contract

- Quick Switcher supports Cmd/Ctrl+P, ArrowUp/ArrowDown/Home/End selection,
  and Enter activation of the selected visible row.
- Context menus use keyboard-operable menu semantics, skip unavailable items,
  focus an enabled item on opening, and restore the invoking control on Escape.
- Folder selection behaves as a listbox: predictable focus, navigation,
  Enter/Space selection, and Escape close/restore.
- Enter invokes only non-destructive primary share or invite actions;
  invitation Accept and Revoke remain explicit.
- Vault-switcher Tab and Shift+Tab leave in the natural direction, and Escape
  restores focus to its trigger.

## Constraints

- Keep all focus/action changes scoped to the Product Client; do not alter
  server authorization or invitation lifecycle semantics.
- Do not touch `finite-brain/docs/research/`.
- Never log, persist, or document copied values, Invite Secrets, raw Folder
  Keys, or decrypted content.

## Agreed test seams

The issue acceptance criteria pre-agree the following deterministic Product
Client seams for this slice:

- Keyboard-list index movement for the Quick Switcher, context menu, Folder
  selector, and Vault switcher.
- Declarative Enter-to-primary-action routing for the supported share and
  invitation inputs, including composition and disabled-control guards.
- The served HTML/JS accessibility contract: combobox/listbox/menu roles,
  roving focus or active-descendant state, focus restoration, and explicit
  non-destructive action routing.

These seams exercise user-observable browser behavior without invoking server
authorization or invitation lifecycle operations.

## Implementation

- Quick Switcher now keeps one selected-result index, exposes the filtered
  rows as an input-owned combobox/listbox, normalizes selection on filtering,
  and uses Arrow keys, Home/End, and Enter without moving focus out of the
  input.
- Context menus now open from pointer or keyboard invocation, expose enabled
  menuitems and separators, focus the first usable action, skip unavailable
  actions during navigation, and restore the invoking control on Escape.
- The Folder selector uses roving option focus. Its direct trigger/list
  handlers stop the Settings modal's document handler from swallowing Arrow,
  Home/End, Enter/Space, or Escape. Selecting a Folder closes the list and
  returns focus to the selector trigger.
- Supported share and invitation fields route Enter only to their explicit,
  non-destructive primary action. IME composition and disabled controls do
  nothing; Accept and Revoke keep their explicit buttons.
- Vault switcher Escape still restores its trigger; Tab and Shift+Tab now
  close the popup and move in the corresponding surrounding document order.

## Verification

- Passed: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Passed: `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
  - Includes deterministic context-menu focus/disabled-item/Escape restoration
    coverage, list movement, and Enter-routing contracts.
- Passed: `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config --locked -- --nocapture`
- Passed: `scripts/with-dev-env cargo build -p finite-brain-app --locked`
- Passed: `scripts/with-dev-env cargo fmt --all --check`
- Passed: `git diff --check`

An isolated server was built and started on `127.0.0.1:8788`, then stopped.
The preferred `agent-browser` CLI is not installed in this environment, and
the available Chromium wrapper points to a missing application bundle, so an
interactive browser smoke could not run here. The final integrated local
browser smoke remains required once the parent task restarts the shared server.

## Focused review

- Standards review: clean. The Product Client-only scope, explicit keyboard
  control flow, accessibility roles, and deterministic tests meet the local
  guides; no secret persistence or logging was introduced.
- Spec review: clean against #28. Each Quick Switcher, context menu, Folder
  selector, Enter-routing, Vault switcher, and existing Settings-nav criterion
  has a direct implementation and regression contract. The keyboard context
  invocation is necessary to make the advertised menu semantics reachable,
  not unrelated scope expansion.
