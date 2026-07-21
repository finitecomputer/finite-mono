# Email Invite Frontend Smoke Review Packet

## Issue

- Issue: finitecomputer/finite-brain#84 and finitecomputer/finite-brain#85 final frontend/smoke batch
- Slice type: AFK
- Acceptance criteria: Product Client email invite creation/claim UX, client-only Invite Secret handling, authorized Folder unlock, unselected restricted Folder denial, known-identity behavior preservation, and full browser smoke verification
- Baseline: `HEAD` before the final uncommitted batch (`6a54cd44f086d2569f49f7808b5f608853fde144`)
- Current diff: `git diff HEAD -- crates/finite-brain-app/src/main.rs crates/finite-brain-server/src/lib.rs crates/finite-brain-server/src/product-client.html crates/finite-brain-server/src/product-client.js crates/finite-brain-server/src/product-client.test.js crates/finite-brain-server/src/routes/public.rs scripts/verify-obsidian-product-client.mjs docs/feature-dev/2026-07-07-email-invite-frontend-smoke-ledger.md`

## Implementation Summary

The Product Client now generates email invite links to `/client#inviteCode=...&inviteEmail=...&inviteSecret=...`, hydrates invite state from that fragment, routes the recipient directly to the Access invite panel, offers an in-panel signer connection step, preserves the invite-selected Brain after claim, and supports local browser smoke verification with a second smoke signer plus an explicit smoke email-proof allowlist.

## Implementation Evidence

- `implement` session: current feature-dev thread
- `tdd` used: targeted tests were added for smoke email proof allowlisting, clock-skew tolerance, generated email invite client URL, and Product Client shell markers
- Red test, if applicable: browser smoke initially exposed invite landing, signer connection, timestamp, and post-claim active Brain gaps
- Green implementation, if applicable: final browser smoke claimed an email invite and opened authorized content
- Refactor, if applicable: `loadBrainMetadata` gained a `preserveActive` option for post-accept/post-claim flows
- Commands run:
  - `node --check crates/finite-brain-server/src/product-client.js`
  - `node crates/finite-brain-server/src/product-client.test.js`
  - `node scripts/verify-obsidian-product-client.mjs`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo build`
  - `cargo fmt --check`
  - `git diff --check`
  - `coderabbit review --agent --type uncommitted`

## Review Instructions

Review only this final frontend/smoke integration batch unless a severe cross-slice regression appears. Keep standards and spec findings separate.

Check:

- Acceptance criteria from #84 and #85 are met.
- Tests verify behavior through public interfaces.
- Invite Secret remains client-side and is not sent to server-visible request bodies.
- Folder Keys and decrypted bootstrap payloads are not displayed or logged.
- Existing known-identity invite, share, direct member/admin, and grant-folder behavior remains covered.
- Relevant test, typecheck, build, and browser smoke verification commands pass.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No documented-standard violations found. The batch follows AGENTS.md guidance by keeping smoke-only proof bypasses explicit, using typed server validation, preserving crypto-adjacent control flow, and adding executable tests around the new safety invariants.
- Smell baseline review found no actionable smells. The small helper additions (`emailInviteClientUrl`, `populateInviteFromHash`, `with_smoke_email_proofs`) are local to the behavior they support and do not introduce meaningful duplication or speculative abstraction.

SPEC_STATUS: pass
SPEC_FINDINGS:
- No spec gaps found for the final frontend/smoke batch. #84's Product Client create/claim path is covered by generated fragment URLs, fragment hydration, signer connection UX, email proof claim, and browser smoke. #85's integrated path is covered by Rust E2E coverage plus the live Product Client smoke that claims as a recipient and opens authorized content.
- No scope creep found. The smoke email verifier and second smoke signer are explicit local-verification hooks, guarded in the app entrypoint, and do not replace finite-identity production proof behavior.
```

## Notes

The `code-review` skill normally asks for parallel sub-agents. The current sub-agent tool policy requires explicit user permission for delegation, so this review was performed directly in the current thread and paired with a completed local CodeRabbit review.
