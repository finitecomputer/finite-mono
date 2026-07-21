# Issue Session

## Issue

- Issue: #13 — Access & sharing in Settings
- Fixed point before session: `93ddfcb`
- Worker session: `/root/ticket_13_access_settings`
- Commit: `18d0b10` plus `93ddfcb` refresh follow-up
- Status: complete for the Access relocation slice

## Inputs

- Spec issue: #10
- Ticket: #13
- Relevant glossary terms: Brain, Member Identity, Session Lock, Session Folder Key, Ephemeral Client Plaintext
- Relevant ADRs: 0010, 0013, 0014, 0016

## Implementation

- Public interface used: real Rust-served Product Client `/client`; existing access/share/member/invitation request functions
- Behaviors covered: Settings → Access & sharing tab, Access ribbon/context deep links, Folder access inspector, Brain member administration, share-link actions, shared-Folder state, busy/result status, and existing disabled/error semantics
- The existing Access panel is reparented into the Settings mount before binding, preserving its IDs, event handlers, and crypto/request lifecycle seams without duplicating controls
- Commands run during implementation: JS syntax check, deterministic Product Client test, `cargo test -p finite-brain-server`, `cargo fmt --check`, `git diff --check`
- Full suite command: pending final integration ticket

## Review

- Review fixed point: `93ddfcb`
- Standards findings: no hard repository, glossary, ADR, security, or secret-handling findings
- Spec findings: acceptance behavior is satisfied in the browser DOM; the source markup remains at its historical location and is moved before bind to avoid duplicate IDs
- Worthy fixes applied: refreshed access-management list loading to follow the Settings section rather than the removed sidebar mode; raised the CSS fixture read limit to accommodate the current stylesheet

## Risks

- The source HTML still contains the historical sidebar location as a relocation anchor. Final integration should verify the rendered DOM has no Access panel under the file sidebar and should hide stale top-level Brain controls.
