## Issue

- Issue: #26 — consolidate Vault navigation and retire legacy controls
- Fixed point before session: `2c6de13`
- Worker session: `/root/ticket_26_vault_legacy_cleanup`
- Status: completed

## Scope

- Keep the footer compact Vault switcher and the detailed Manage Vaults modal.
- Remove the redundant Vault picker embedded in Settings → Access.
- Give Settings → Vault a Manage Vaults entry point and correct the Command
  Palette wording/routing for Vault access.
- Retire hidden raw Folder Key/manual OKF/old Vault-control plumbing while
  retaining client-owned pure OKF planning and normal Session Folder Key flows.

## Constraints

- Do not touch `finite-brain/docs/research/`.
- Do not persist raw Folder Keys, decrypted content, Invite Secrets, or other
  session plaintext in browser storage.
- Preserve the existing Settings and Access keyboard behavior; nested dialog
  focus return must remain reliable.

## Test seams

- Product Client static shell: the rendered HTML exposes one Settings Access
  surface, one dedicated Manage Vaults dialog, and no hidden legacy key/import
  controls.
- Product Client command contract: `commandPaletteCommands()` describes Vault
  access as a Settings route rather than a Sidebar destination.

## Implementation

- Consolidated Vault navigation into the footer switcher and Manage Vaults
  dialog. Settings → Vault now supplies an explicit **Manage Vaults** action.
- Routed the ribbon and Command Palette Vault access actions to Settings →
  Access; removed the redundant Access-side Vault picker and its bridge state.
- Removed hidden raw Folder Key, manual OKF-import, legacy selector, and
  hidden Encrypt Draft controls while preserving the pure client-side OKF
  helpers and ordinary Folder Key grant flows.
- Closing Manage Vaults opened from Settings restores Settings → Vault and
  focuses the initiating Manage Vaults action. Settings is closed while the
  nested dialog is active, preventing a visible double-dialog state.

## Verification

- Passed: `scripts/with-dev-env node finite-brain/crates/finite-brain-server/src/product-client.test.js`
- Passed: `scripts/with-dev-env cargo test -p finite-brain-server product_client_serves_spine_assets_and_config --locked -- --nocapture`
- Passed: `scripts/with-dev-env node --check finite-brain/crates/finite-brain-server/src/product-client.js`
- Passed: `scripts/with-dev-env node --check finite-brain/scripts/verify-obsidian-product-client.mjs`
- Passed: `scripts/with-dev-env cargo fmt --all --check`
- Passed: `git diff --check`
- Browser proof on a freshly built disposable Rust server: Access opened
  Settings → Access without legacy picker/raw-key controls; nested Manage
  Vaults exposed one dialog, hid Settings, and returned focus to the Settings
  Manage Vaults action on close. No page or console errors were observed.
- The full smoke verifier could not run because this workspace lacks the
  documented prebootstrap `/tmp/finite-brain-smoke-vault-keys.json` manifest;
  the verifier and its seeding script both require that local-only fixture.

## Follow-up review

- Standards review found no documented-standard breaches. Its sole
  judgement-level P3 was stale hidden-panel terminology in an internal render
  helper; the follow-up commit renames it to `renderAccessShareControls` to
  match the visible controls it initializes.
