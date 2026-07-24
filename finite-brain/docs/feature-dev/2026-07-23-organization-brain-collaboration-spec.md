# Complete Organization Brain collaboration

## Problem Statement

An Organization Brain administrator can currently add another Agent as a
member and administrator without giving that Agent the current Folder Key
Grants needed to read the Brain's existing Folders. The role mutation succeeds
and looks complete, but the recipient opens a Brain whose restricted content
is locked. The inviting Agent must discover and manually compose identity
resolution, membership, role, and one grant operation per Folder.

The managed FiniteBrain skill makes this worse by sometimes probing public
NIP-05 endpoints even though `fbrain` already resolves Finite identities
natively. It may stop after the Brain Role succeeds, and neither its report nor
the Product Client clearly separates Brain Role from Folder Access Readiness.
Users therefore experience a successful-looking share followed by diagnosis
and repair work in the recipient Agent.

## Solution

Make Organization Brain collaboration one idempotent desired-state operation.
An administrator supplies the Brain and a canonical email-first target. The
trusted client resolves the target, prepares encrypted grants from every
current Folder Key it can open, and submits one batch command that ensures
membership, the administrator Brain Role, and current Folder Key Grants for
the default scope of all existing Folders.

The result is one stable collaboration receipt. It reports `complete` only
when authoritative Brain state proves the requested Brain Role and Folder
Access Readiness for every Folder in scope. If the acting administrator cannot
open one or more current Folder Keys, the operation preserves useful progress
but reports `partial`, names every affected Folder, and remains safe to retry
from a key holder. A retry converges rather than duplicating relationships or
grants.

The existing low-level permission commands remain role-only primitives for
advanced workflows. The managed skill uses the high-level operation for normal
"share this Org Brain with Agent B" requests. The Product Client presents
Brain Role and Folder Access Readiness separately and offers the same
convergent repair action.

## User Stories

1. As an Organization Brain admin, I want to add another Agent by Managed
   Agent Email, so that I do not handle public keys.
2. As an Organization Brain admin, I want one command to establish the intended
   collaboration, so that I do not compose permission primitives.
3. As an Agent, I want `fbrain` to use native identity resolution, so that it
   does not depend on ad hoc public NIP-05 requests.
4. As an inviting Agent, I want the default operation to include every existing
   Folder, so that "share the Brain" has its natural meaning.
5. As a recipient Agent, I want administrator status and readable current
   Folders, so that I can begin work immediately.
6. As a recipient Agent, I want `admin_only`, `all_members`, and `restricted`
   Folders handled consistently, so that policy entitlement is not mistaken
   for decryptability.
7. As a security-conscious user, I want Folder Keys wrapped only by a trusted
   client that already holds them, so that the server never manufactures
   access.
8. As a security-conscious user, I want Folder Keys and grant plaintext absent
   from responses, logs, and durable CLI state.
9. As an inviter missing one Folder Key, I want the result to say `partial`
   rather than `complete`, so that I cannot accidentally overstate access.
10. As an inviter, I want every incomplete Folder named by a safe path and
    reason, so that I know who can repair it.
11. As an inviter, I want useful membership, role, and available grants
    preserved when one Folder is unavailable, so that successful work is not
    discarded.
12. As an inviter, I want to retry the same command, so that repair requires no
    new operation vocabulary.
13. As an inviter, I want exact retries to be idempotent, so that timeouts do
    not duplicate roles or grants.
14. As an inviter, I want network uncertainty represented explicitly, so that
    a lost response is not described as a clean failure.
15. As an inviter, I want concurrent Folder creation or key rotation detected,
    so that stale grants cannot produce a false complete result.
16. As an inviter, I want a target that is already a member to converge to the
    requested administrator collaboration without error.
17. As an inviter, I want an existing admin with one missing grant repaired by
    the same operation.
18. As an inviter, I want an already complete collaboration to return complete
    without unnecessary mutation.
19. As an advanced operator, I want low-level member, admin, and Folder-grant
    commands to remain available, so that narrow policy operations stay
    possible.
20. As an advanced operator, I want low-level role mutations clearly labeled
    as not guaranteeing Folder Access Readiness.
21. As a Product Client user, I want each collaborator's Brain Role shown
    separately from Folder readiness, so that I can understand real access.
22. As a Product Client admin, I want an incomplete collaborator row to show
    the ready Folder count and a repair action.
23. As a Product Client admin, I want repair to use the same desired-state
    operation, so that CLI and UI semantics cannot drift.
24. As a managed Agent, I want the FiniteBrain skill to invoke one high-level
    command and inspect its typed state.
25. As a managed Agent, I want the skill to report complete and partial results
    accurately without exposing advanced identity details.
26. As a managed Agent, I want a partial result to tell the user which current
    key holder must retry, so that recovery is actionable.
27. As a recipient Agent, I want to open and sync the Brain after collaboration
    completes, so that the feature proves more than metadata.
28. As a recipient Agent, I want to read existing content and add a Page, so
    that bidirectional work is proven.
29. As the original Agent, I want to sync and observe the recipient's Page, so
    that collaboration is proven end to end.
30. As a developer, I want one collaboration receipt shared across CLI,
    Product Client behavior, skill guidance, and tests, so that completion has
    one definition.
31. As a tester, I want two independent identities and Finite Homes in the
    acceptance path, so that shared local signer state cannot hide a bug.
32. As a tester, I want the real signed Brain HTTP boundary in the acceptance
    path, so that in-process command tests cannot hide protocol failures.
33. As an operator, I want failures to identify the broken product boundary and
    retain safe diagnostics, so that smoke-test failures are repairable.
34. As a user, I want a newly created Folder after the initial collaboration to
    appear as drift until normal recipient policy or a retry grants it, so that
    snapshot semantics remain honest.
35. As a user, I want removing access to remain governed by existing key
    rotation rules, so that this additive workflow cannot weaken revocation.

## Implementation Decisions

- **Organization Brain Collaboration** is the desired state. It combines a
  requested Brain Role with Folder Access Readiness across an explicit Folder
  snapshot.
- The common CLI interface is `fbrain collaborators ensure-admin --brain
  <brain-id> --target <email|NIP-05|npub>`. Canonical email is the user-facing
  form; public-key forms remain advanced-compatible.
- `ensure-admin` defaults to every existing Organization Brain Folder.
  Callers do not need an `--include-existing-folders` flag that can be
  forgotten.
- The first version supports the administrator desired state. The interface
  can later add other explicit desired roles without exposing member-before-
  admin sequencing.
- The existing `permissions add-member`, `permissions add-admin`, and
  `permissions grant-folder` operations remain low-level and keep their
  current bounded semantics.
- The collaboration module resolves the target exactly once with Brain's
  native identity resolver and reuses the resolved Member Identity throughout
  the operation.
- The trusted client fetches an authoritative Folder inventory and key-version
  snapshot, opens its available current Folder Key Grants, and creates wrapped
  recipient grants locally.
- One signed batch Brain command accepts the resolved target, desired role,
  exact Folder/key-version snapshot, supplied encrypted grants, and signed
  access-change evidence. The server validates and transactionally applies all
  valid supplied state.
- The server never receives a raw Folder Key, decrypts a grant, derives a key,
  or creates a grant envelope. Missing source keys are declared as incomplete
  Folder outcomes.
- A batch may commit membership, admin role, and available Folder grants while
  reporting missing-key Folders as partial. The batch itself is atomic for the
  state it claims to have applied.
- The command is idempotent and convergent. Existing membership, role, or
  current grants are `alreadyReady`, not conflicts.
- The receipt has stable top-level states `complete`, `partial`, and
  `indeterminate`. `complete` requires authoritative postcondition evidence;
  `partial` names known gaps; `indeterminate` means mutation may have committed
  but the response cannot prove it.
- Each Folder outcome includes Folder identity/path, expected key version,
  readiness outcome, a stable reason code when incomplete, and retryability.
- Safe Folder outcomes include `granted`, `alreadyReady`,
  `missingSourceKey`, `staleVersion`, and `failed`.
- A final authoritative inspection prevents stale inventory or a concurrent
  key rotation from producing `complete`.
- The same desired-state interface supports read-only collaboration inspection
  for UI presentation and diagnostics.
- The Product Client shows Brain Role and Folder readiness independently, for
  example `Brain role: Admin` and `Folder access: 2/3 — needs repair`.
- Product Client repair repeats the same collaboration intent from the current
  holder; it does not construct a separate repair workflow.
- The managed FiniteBrain skill teaches the high-level command for normal
  collaboration requests and moves low-level permission composition into an
  explicitly advanced section.
- Human output uses readable email and Folder paths. JSON may include the
  resolved `npub` for machine diagnostics, but neither form includes secrets or
  grant plaintext.
- This change preserves all existing access-removal and Folder Key rotation
  invariants.

## Testing Decisions

- Good tests assert externally observable membership, Brain Role, current
  Folder Key Grant coverage, readable content, sync convergence, and typed
  results. They do not assert private helper order or database implementation.
- The primary acceptance seam is a built `fbrain` process communicating with
  the real signed Brain server using two independent Finite Homes and Member
  Identities.
- The acceptance scenario creates an Organization Brain and existing
  restricted content as Agent A, runs one email-first `ensure-admin` for Agent
  B, opens the Brain as Agent B, reads the existing Page, writes and syncs a new
  Page, then syncs as Agent A and observes Agent B's Page.
- The same seam covers an existing member, an existing incomplete admin,
  already-complete collaboration, and a retry that repairs a previously
  missing source key.
- A partial-path scenario proves one missing current source key produces a
  partial receipt with the correct Folder and makes no complete claim.
- A concurrency scenario rotates or advances a Folder key version between
  planning and commit and proves stale state cannot return complete.
- A transport retry scenario proves an exact retry converges after an
  indeterminate response.
- Signed server integration tests cover authorization, identity/target
  validation, Organization-Brain-only behavior, idempotency, batch
  transactionality, current-version validation, per-Folder outcomes, and
  secret-free responses.
- CLI interface tests use existing in-process command fakes for fast receipt,
  exit-status, rendering, and no-secret coverage, supported by process
  acceptance through the real executable.
- Product Client tests cover distinct role/readiness presentation, complete and
  partial result rendering, and the repair action through the collaboration
  interface.
- Managed-skill static and scenario tests cover email-first command choice,
  complete versus partial reporting, safe diagnostics, and the warning on
  low-level role-only commands.
- Existing Folder Access, invitation, sharing, key rotation, Product Client,
  CLI, and monorepo smoke suites remain green.
- Final verification includes focused tests, component suites, repository
  quality gates, and a fresh two-Agent local smoke run through the real
  runtime/product surfaces.

## Out of Scope

- Changing Personal Agent Access or Personal Brain collaboration.
- Making the server a Folder Key holder or adding server-side decryption.
- Automatically granting future restricted Folders forever from one snapshot.
- Changing Folder access removal, revocation, or key rotation policy.
- Replacing email invitations for targets whose Member Identity is not yet
  resolvable.
- Removing or silently changing the semantics of low-level permission
  commands.
- General role-policy redesign beyond the administrator collaboration intent.
- Sharing a Folder across Brains, share links, or Personal Brain mounts.
- Production deployment or mutation.

## Further Notes

- This work follows ADR-0034 and the existing decisions that keep Folder Object
  crypto client-owned and raw Folder Keys session-only.
- Brain already has the correct native identity-resolution path. The feature
  removes skill-level misuse rather than introducing a second resolver.
- Existing policy-derived effective recipient lists are insufficient evidence
  of Folder Access Readiness. Completion must prove a current grant for the
  target and current key version.
- The complete Personal and Organization Brain matrix remains tracked
  separately; this spec contributes the post-creation two-Agent collaboration
  scenario to that higher-level acceptance gate.
