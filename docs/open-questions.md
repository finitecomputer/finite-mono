# Open questions

Status: PROPOSED

Open questions are context, not queue. Agents may append dated evidence, but do not propose solutions or convert one into work without a separate design conversation and a blessed run.

## Hosted Web Device continuity (opened 2026-07-10)

Wrong: A Hosted Web Device depends on a Finite-operated identity and MLS store. Ordinary restart preserves that state, but loss or corruption of the store has no proven path back to a usable Device plus retained history or a usable export. The current web flow starts the runtime when the dashboard opens; autonomous continuity without an active browser is not an established product contract.

Rejected so far: Silently minting an unrelated chat account, or treating server-held ciphertext as recovered user data, is unacceptable. Restoring an nsec and linking a replacement Device restores account authority, not prior MLS history.

A real solution must: preserve the Device's revocable trusted-server posture; give the user retained data or an explicit usable export after the declared failures; not make Electron a launch dependency; and not turn the chat server into a plaintext-history authority.

Notes:

- 2026-07-10: `finitechat-hosted-device` tests ordinary restart, transcript, and attachment survival, but not lost/corrupt-store or replacement-Device recovery; [the architecture document](../finitechat/docs/architecture.md) says those recovery paths are not built.
- 2026-07-10: [the recovery plan](../finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md) records no proven path for the relevant store/history/key-loss cases.

## Operator-blindness without user lockout (opened 2026-07-10)

Wrong: Finite cannot honestly promise both that user data will never become inaccessible and that operators are cryptographically unable to recover it. Kata remains host-operator-trusted, and the repository has no proven User Recovery Key, Finite-Assisted Recovery Authority implementation, complete Recovery Snapshot, or empty-target restore.

Rejected so far: A TEE or provider durable volume is not a backup or proof of operator-blindness. An encrypted backup without recoverable keys is not a successful recovery path. Removing practical recovery authority before an equivalent user-controlled path is designed and exercised is rejected.

A real solution must: preserve ordinary restart and upgrade on mounted state; state the actual privacy level honestly; keep recovery material out of ordinary Core state; require the declared consent, audit, and key-release properties; and prove recovery of the same Recovery Set onto an empty target before stronger privacy claims.

Notes:

- 2026-07-10: [ADR 0001](adr/0001-recoverability-precedes-operator-blindness.md) explicitly leaves Recovery Snapshot, key backup, Recovery Authorities, export, retention, and empty-target restore open while allowing an honestly disclosed trusted-cohort first slice.
- 2026-07-10: the [runtime recovery plan](../finitecomputer-v2/docs/runtime-recovery-and-observability-plan.md) records unproven Device/history/key-loss paths and missing off-host runtime recovery.
- 2026-07-10: Paul scoped the current run as an internal production canary only. Customer admission is a separate next run, and the paid-cohort backup/restore gate remains intact for that run.

## Agent-operation bootstrap before untrusted admission (opened 2026-07-10)

Wrong: Until the authorization ledger has a Principal, the current `agent.owner.claim` path makes the first already-admitted Finite Chat Principal the durable mutation authority. That is a trusted-user first-slice expedient, not a cryptographic binding between Account Auth, the Project, and authority to mutate an Agent Runtime.

Rejected so far: Account Auth identity, bearer tokens, and Runner capabilities are not agent-operation authorization. The existing first-admitted-Principal claim is explicitly not sufficient before untrusted room admission.

A real solution must: establish a durable authorized Principal before arbitrary untrusted admission; fail closed for every other Principal; preserve the typed Finite Chat command boundary and replay-safe ledger; and avoid dashboard, Runner, or provider shell/filesystem authority.

Notes:

- 2026-07-10: `finite-agentd/src/daemon.rs` authorizes the sender of `agent.owner.claim` when the authorized-principal count is zero.
- 2026-07-10: [ADR 0003](adr/0003-agentd-is-the-agent-owned-platform-boundary.md) requires a Core-bound one-time claim or equivalent attested enrollment before untrusted room admission, without changing command schemas.
- 2026-07-10: For the trusted internal canary only, Hosted Web Chat will require the existing first-admitted Principal claim before becoming usable and will hide the external Agent-address affordance until then. This narrows the trusted-canary window but does not satisfy the customer pre-admission requirement.

## Honest cancellation of an agent turn (opened 2026-07-10)

Wrong: Finite Chat defines a `runtime.command.cancel` data-model primitive, but a human chat turn is not currently that runtime command and `finite-agentd` does not execute cancellation payloads. “Stop Hermes” therefore has no proven stable turn identity or durable terminal behavior in the SaaS path.

Rejected so far: Restarting compute, killing Hermes without a turn contract, hiding an unfinished turn, or presenting command-ledger cancellation as though it already cancels a model/tool turn are rejected.

A real solution must: define whether queued and active turns are separately cancellable; name the stable turn/request identity; make cancel/result races deterministic; survive process restart; leave the next turn usable; and show an honest terminal state without corrupting chat history or workspace work.

Notes:

- 2026-07-10: Paul moved turn cancellation out of the internal production canary and into the later customer-facing run.
- 2026-07-10: `finite-agentd/src/daemon.rs` acknowledges and discards inbound runtime payloads that are not requests; current chat turns are ordinary chat events rather than implemented cancellable runtime commands.
- 2026-07-10: Paul consciously kept Stop Hermes out of this run, including its training-oriented Launch Code path. The product response is to make talking to and steering the agent simpler and better, not to normalize fear of an opaque process or mislabel process termination as turn cancellation.

## Dashboard Finite Sites list/share authority (opened 2026-07-10)

Wrong: “Sites list” can mean Projects or Outputs published by the Agent Principal, Projects owned by the human's Native Principal, or Outputs already shared to the Account Auth email. Those are different authorization sets, and the current dashboard email-session exchange can view an already shared Output but cannot create a Share.

Rejected so far: Signing a sharing mutation as the Hosted Web Device without a grant, treating verified email as the Agent Principal, reviving legacy host/app lifecycle controls, or giving Dashboard/Runner filesystem authority are rejected.

A real solution must: name whether the surface lists Project Repositories, Project Outputs, or both; identify the owning and acting Principal; use Finite Sites' existing Project List, Visibility, Share, and delegation contracts; preserve revocation; and keep publishing plus sharing independent from Runtime lifecycle.

Notes:

- 2026-07-10: Paul moved dashboard Sites list/share out of the internal production canary and into the later customer-facing run.
- 2026-07-10: Finite Sites already exposes NIP-98-authenticated Project List and Output sharing routes, while the dashboard's verified-email viewer exchange explicitly does not create a Share.

## Stuck launch cancellation and provider cleanup (opened 2026-07-10)

Wrong: A creation request can remain `requested` or `launching` without a user escape. Expired Runner leases are re-leasable and Core has a service-only cancellation mutation, but cancelling while a provider create is in flight can race compute registration and cleanup, leaving orphaned compute or incorrect entitlement state.

Rejected so far: Exposing the current service cancellation directly for `launching`, clearing Core rows before provider cleanup is proven, releasing quota twice, or adding an unbounded general reconciliation control plane are rejected.

A real solution must: define when the user sees keep-waiting, retry, and cancel choices; represent cancellation/cleanup durably; make Runner and Core retries idempotent across crashes and lease expiry; prove provider compute and credentials are cleaned or adopted; and release the entitlement exactly once.

Notes:

- 2026-07-10: Paul moved proactive stuck-launch recovery out of the internal production canary and into the customer-facing run. If the canary actually sticks, that observed failure blocks acceptance and is fixed before retrying.
- 2026-07-10: the Dashboard currently shows an unbounded waiting state for `requested`/`launching` and exposes reset only after `failed`; Core's cancellation route is service-authenticated rather than owner-scoped.

## Brain access when a new Device meets existing encrypted Vaults (opened 2026-07-10)

Wrong: Dashboard account access and encrypted Brain access are different boundaries. A new Hosted Web Device may open the Brain surface while existing Vault and Folder grants remain tied to a different Principal, leaving a person unable to demonstrate usable access to their existing encrypted knowledge.

Rejected so far: Treating account email as a replacement for a cryptographic Folder Key grant, silently broadening a grant, or letting the dashboard proxy become a Folder Key authority would weaken the documented boundary.

A real solution must: preserve existing readable data and explicit, product-scoped, revocable grants; keep Folder Keys out of ordinary dashboard and Core state; make any migration or recovery path understandable; and avoid locking a person out of their existing knowledge.

Notes:

- 2026-07-10: [the identity boundary](../finitecomputer-v2/docs/identity-boundary-v1.md) separates WorkOS/dashboard access from Nostr Principal authorization and treats encrypted Folder Key grants as a separate Brain concern.
- 2026-07-10: this is recorded for Austin's review; it is not work in the [internal production canary](runs/production-canary.md).
- 2026-07-10: The dashboard iframe/proxy implementation still exists. Its embedded Product Client requires `window.nostr`; WorkOS does not supply that signer, so the iframe is transport/UI composition rather than an identity solution.
- 2026-07-10: Paul decided to hide Brain from both admin and main-product navigation for now without deleting the iframe work. Electron has a local Finite Chat identity and is the likely good client shape, but no Electron-to-Brain signer or Folder Key bridge is currently wired.

## One chat product across Hosted and Electron Devices (opened 2026-07-10)

Wrong: Dashboard chat is implemented in `hosted-web-chat.tsx` while Electron has a separate Vite renderer in `finitechat/apps/electron-chat/src/App.tsx`. They share Finite Chat concepts but can drift in interaction, copy, features, and bug fixes, contradicting the promise that Electron is another Device rather than another chat product.

Rejected so far: Making Electron depend on Hosted Web Device uptime, treating its renderer as an unrelated legacy UI, or indefinitely duplicating product behavior between two implementations.

A real solution must: preserve Electron's local Device key and durable store; keep dashboard chat usable without Electron; share one canonical product interaction and `AppState`/`AppAction` behavior; and define which UI components, presentation model, and platform adapters are actually shared without moving local secrets into the browser.

Notes:

- 2026-07-10: The internal production canary remains dashboard-only. This question repairs the run's previous dangling reference to Electron/device-unification architecture; it is not added to the active queue.
- 2026-07-10: The current hosted UI lives in `finitecomputer-v2/apps/dashboard/src/components/hosted-web-chat.tsx`; Electron's renderer lives in `finitechat/apps/electron-chat/src/App.tsx`.
