# Shared Frostr Signer Feature Ledger

## Run

- Run ID: 2026-07-01-shared-frostr-signer
- Loop: plebdev feature-dev local-main variant
- Target repo: `/Users/plebdev/Desktop/Projects/finite/finite-auth`
- Base branch: `main`
- Feature branch: `main`
- Human owner: plebdev
- Started: 2026-07-01
- Current status: planning artifacts complete; implementation slices pending
- Skill setup status: local agent docs exist; GitHub issue tracker and PR
  remote are not configured yet

## Goal

Map the model where a FiniteBrain user and their agents share the same Nostr
keypair by default through a Frostr group key. The server holds one share, the
active user client holds one share, and native secure storage holds one share.
Agent accountability should come from Finite signing-session and audit records,
not separate default Nostr public keys.

## Durable Artifacts

- CONTEXT updates: Shared User-Agent Signer, Agent Signing Session, Native
  Secure Storage Share, Cold Backup Share
- ADRs: `docs/adr/0003-share-user-agent-signer-through-frostr.md`
- Specs: `docs/specs/native-secure-secret-storage.md`
- PRD issue: local PRD file, because the repo has no remote issue tracker
- Slice issues: local issue breakdown file
- Issue sessions: not started in this mapping slice
- Review packets: not started in this mapping slice
- Local CodeRabbit report: not run for docs-only mapping
- PR URL: not applicable; user requested all work on `main`

## Commands

- Source scan: `rg`
- Verification: `git diff --check`
- Typecheck: not run, docs-only mapping
- Test: not run, docs-only mapping
- Build: not run, docs-only mapping
- Visual verification: not applicable

## Slice Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| Local-SFS-1: Hard-cut shared signer domain vocabulary | AFK | complete in this mapping pass | not started | Rust/store migration moved to Local-SFS-2 | docs diff checked |
| Local-SFS-2: Replace agent key binding with signing session model | AFK | pending | not started | none yet | no |
| Local-SFS-3: Model Frostr quorum paths for user and agent signing | AFK | pending | not started | none yet | no |
| Local-SFS-4: Define bifrost and native storage adapter interfaces | AFK | pending | not started | include Rust `keyring` adapter guidance | no |
| Local-SFS-5: Product gate for unattended agent signing | HITL | parked | not started | product/security decision required | no |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| Local-SFS-5: Product gate for unattended agent signing | It decides whether agents can sign without fresh user presence. The cold backup share is not the mechanism. | Background-agent implementation only. | Accept or reject unattended signing with scopes, expiry, revocation, notification, audit requirements, and a non-backup signing mechanism. | Out of scope for this mapping slice. |

## Open Questions

- Should native secure storage ever be allowed to satisfy agent signing instead
  of remaining recovery-only? Recommended answer: no for the first
  implementation.

## Escalations

- The standard feature-dev loop wants a feature branch and non-draft PR into
  `staging`; this run is intentionally kept on `main` per user instruction.
- This mapping slice does not execute the code migration. The existing
  delegated-agent-key scaffold is now known implementation debt tracked by
  Local-SFS-2.
