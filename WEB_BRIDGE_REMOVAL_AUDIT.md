# Web bridge removal audit

2026-09-11. Source audited: `codex/finitechat-wasm-spike` at `79aaaaa8`,
based on `origin/main` at `4c924a95`. This is a source audit plus review of the
completed real-browser test evidence. No production services, data, or code
paths were changed, and no new inference test was run for this audit.

## Finding

The demonstrated text conversation, real Hermes terminal tool execution,
Topic/Chat context, future-message sync across Devices, browser persistence,
Room member list, metadata snapshot, and on-demand per-Chat history do not use
`finitechat-hosted-device` or the dashboard hosted chat APIs. There is no silent
hosted fallback in the browser adapter.

That does **not** mean the whole dashboard/Hermes product can lose the service
today. A Site preview inside the shared chat UI still calls a hosted signer.
The normal send path attempts to supply verified requester context for Sites
tools; the spike omits that path. Production account identity, Project bootstrap, Connections
controls, attachments, and Brain operations still have hosted dependencies or
are explicitly disabled/bypassed in this experiment.

The architectural win is real. The remaining work is replacing the service's
other responsibilities and proving the new owners, not discovering how to get
basic chat through MLS.

## What “web bridge” means here

The deletion target is `finitechat-hosted-device`: a server-side human Device
with an account key, MLS state, transcript, runtime, bindings, and internal
HTTP API. Next authorizes an Account Auth session and calls that service.

The following remain necessary:

- `finitechat-server`: ciphertext delivery/log, KeyPackages, Welcomes and blobs.
- `finitechat hermes serve` and the Hermes FiniteChat plugin: the **agent's**
  Device, durable inbox and transcript, decryption/encryption, and local
  communication with Hermes. Code calls this a resident bridge too; it is not
  the server-side human web Device. Removing it would remove the agent's chat
  transport.
- Shared `finitechat-client`, MLS, protocol, delivery and native core code.
  No other workspace crate depends on the hosted-device crate by Cargo manifest;
  its primary callers are dashboard HTTP clients and deployment tooling.
- Core/Account Auth for login, account identity and Project authorization.
  A control-plane key/grant handoff can remain without relaying chat plaintext.

The current spike runs only relay, native FiniteChat daemon, Hermes gateway and
Next. `scripts/finitechat-wasm-spike.py:95` launches those children;
`child_environment` at line 46 excludes ambient hosted-service configuration.

## Trace of the working path

1. `src/app/api/wasm-spike/bootstrap/route.ts` returns the disposable fixture's
   nsec, public Agent identity, Room and endpoints. It neither opens nor calls
   a Hosted Web Device. This is not implemented Core login/key custody.
2. `src/components/browser-chat-provider.tsx` installs the browser adapter in
   the existing dashboard context. `src/lib/browser-finitechat.ts:110` loads
   WASM, obtains the handoff, and opens the encrypted IndexedDB checkpoint.
3. `finitechat/crates/finitechat-wasm/src/browser.rs:147` publishes the browser
   KeyPackage, signs a narrow pre-join enrollment request to the agent, and
   activates the agent's Welcome. Each browser profile owns an MLS leaf.
4. The native agent's `browser_spike.rs` admission operation runs on the
   existing FiniteChat actor. It authors Add/Welcome for that exact Device and
   KeyPackage. It does not ask another human Device to approve or forward it.
5. `browser.rs:229` encodes the shared `HermesSendRequestV1` payload and encrypts
   it. `browser.rs:397` saves/retries the exact envelope and posts directly to
   the relay. The browser's sync loop applies the actual ordered MLS log.
6. `finitechat-cli/src/hermes.rs:408` opens the agent Device and resident sync.
   Its local inbox stream reaches `integrations/hermes/finitechat/adapter.py`;
   replies use the agent's same local service. Hermes retains the actual
   Topic/Chat session context. No hosted human service participates.
7. Metadata/history use encrypted `finitechat.browser.request.v1` and response
   events. `finitechat-core/src/browser_spike.rs:124` reads the agent's durable
   records; per-Chat history starts at line 214. Copies are agent-attested
   historical records, not original MLS history decryption on the new Device.

Paths beginning `src/` above are under `finitecomputer-v2/apps/dashboard/`.
Names such as `HostedWebChat`, `HostedChatContext`, and `HostedChatState` are
reused UI/types. Their names do not establish a running-service dependency.
The context currently lives alongside the old provider; extract it when
removing the provider rather than rewriting the UI.

## Remaining dependencies and gaps

| Surface | Actual dependency / shortcut | Replacement needed |
| --- | --- | --- |
| Site preview **inside chat** | `hosted-web-chat.tsx:1556` calls `/api/site-previews/.../session` when the user opens a recognized Site URL. `site-preview.ts:122` requires hosted config and calls `hostedDeviceSitesIdentityProvider`. This path has no browser capability gate. | Implement Sites' typed viewer-session proof using the browser key, retaining Sites' own authorization/session exchange. Until then this is a latent broken feature in the spike, not a working bridge-free feature. |
| Hermes creates a Site for the user | `hosted-web-chat.ts:228` attempts to obtain Core-verified email and a Sites-issued requester assertion; when present, hosted dispatch adds it to encrypted message metadata. `adapter.py:329` materializes it for tools; `finite-sites/crates/fsite-cli/src/requester_context.rs` consumes it. Browser sends do not obtain this context. | Retain an authenticated, product-scoped assertion issuer; give the browser the assertion to carry in MLS, or establish equivalent explicit delegation at admission. An nsec proves a Principal, not a verified email address. Never trust a browser-supplied email alone. Ordinary chat already tolerates absence of this assertion. |
| Login and first Agent creation | `agent-creation-requests/route.ts:181` asks the bridge to mint/read the owner's chat identity, passes its public ID to Core, then seals bootstrap authorization in the bridge. Profile-image upload also uses it. Core currently persists the public owner ID into the runtime spec, not this spike's nsec handoff. | Core-owned stable User Key/account mapping and authenticated handoff under the chosen custody assumption; correct public owner identity before runtime launch; relocate profile upload. |
| Canonical Room binding / enrollment | Production binding and exact bootstrap journal live in hosted-device `lib.rs:1125`. Spike uses one fixed Room/Agent/user and feature-gated admission. | Durable product-to-Room discovery and one authorized creation path. Agent authors membership. Core authorizes the actual Project/Principal/Device grant; browser selection never creates or chooses authority. Scope grants to Device/KeyPackage, define retries/revocation and multiple Projects. |
| Agent ownership and Connections controls | `hosted-web-chat.ts:362` sends `agent.owner.claim`; `hosted-agent-controls.ts` sends inference, Telegram, SimpleX and Google commands through `hostedDeviceRuntimeCommand`. Spike sets `ownerClaimed = client !== null`, makes claim/recovery call sync, and shows only the Room device list. The fixture does not run `finite-agentd`. | Browser sends and correlates the existing typed encrypted runtime commands; prove the actual agent-side handler and authorization. Google OAuth callback must get its result to the agent when the browser is absent, through an explicit product capability/delivery flow—not a hidden permanent human Device. |
| Attachments, voice notes and images | Browser capability is disabled; upload/download functions throw, and message projection uses `media: []`. Production hosted service encrypts/uploads and decrypts/downloads files. | Port shared blob encryption/reference verification plus browser upload, download, cache and resource bounds; test actual Hermes media input/output. Moving only upload is insufficient. |
| Brain cards, approvals and identity operations | Explicitly disabled by the browser provider. `brain-hosted-client.ts` uses the hosted typed identity provider for HTTP proofs and approval signatures; the hosted service also wraps/opens resource-bound grants. | Implement the existing typed Brain identity-provider contract in the browser, preserving Brain authorization and grant boundaries. Enabling the current React cards alone still calls hosted APIs. |
| Other chat state | `MarkRoomRead` and `SetTyping` are no-ops; profiles and live activity are empty, unread counts zero, per-message delivery projection absent. Harness disables streaming and approvals. | Port the selected canonical projections/ephemeral behavior. Prove streamed edits/finalization, cancellation and interactive tool flows before claiming parity; this audit does not claim every such feature already works in the old dashboard either. |
| Device management | Room member list works. It is not the account-wide active/revoked catalog; RevokeDevice is unsupported. | Explicit registration/Remove/revocation behavior and credential renewal. With an exported account nsec, logout alone cannot revoke knowledge of the key; enrollment policy must enforce any Core device-grant revocation contract. |

There is a source-level contract mismatch in the existing requester-context
path: `createHostedRequesterContext` calls `hostedDeviceState` and expects
`state.hosted_agent_binding.agent_npub`. The hosted `/v1/app/state` handler at
`finitechat-hosted-device/src/lib.rs:663` returns plain `AppState`, which has no
binding field; the separate binding endpoints wrap that field. As written on
this branch, the helper returns no requester context at that check. Therefore
this audit does not claim automatic Sites ownership currently succeeds through
that path. Preserve and prove the intended verified-identity contract while
replacing it; do not reproduce the mismatch. This was not tested on production
and is not changed by this audit.

These are a mix of **remaining calls**, **authority supplied elsewhere today**,
and **unimplemented behavior**. They are not evidence that text/history secretly
pass through a Hosted Web Device.

## State ownership after removal

| State | Writer and readers today | Proposed ownership |
| --- | --- | --- |
| Human account key | Hosted service `runtime_for` creates/loads `finite-home`; chat runtime and Brain/Sites signers read it. | Core key custody under the accepted nsec assumption; authenticated browsers consume the same identity. Agent receives public identity/grants, never the human nsec. |
| Human MLS leaf, send journal and local projection | Hosted `FiniteChatRuntime`/SQLite writes; hosted HTTP/SSE and dashboard read. | Browser WASM and IndexedDB store are the writer, serialized with Web Locks; tabs and UI read. Already present in the spike. |
| Project-to-Room binding and creation authorization | Hosted sealed records write; dashboard setup, chat and Connections read. | Core product lifecycle record/discovery plus an agent-side durable protocol journal. Core binding is navigation/authorization metadata, not MLS state. Exact placement is a design decision; preserve one authority. |
| MLS membership and encrypted log | Member Devices author signed/encrypted operations; relay stores/delivers; admitted Devices apply. | Same protocol. Agent admits browser Devices. No account key alone recreates old MLS state. |
| Historical transcript and metadata | Hosted Device and agent each retain what they processed; browsers normally read hosted projection. | Agent FiniteChat store serves requested history; browser caches imported copies. Prove completeness/backup of the agent's source rather than assuming its store duplicates every retained hosted record. |
| Hermes model context and inbox | Hermes/plugin and native agent service write/read their local state. | Remain agent-owned; include those stores and inbox recovery in the durability proof. |
| Product assertions and delegated access | Core/WorkOS establish account context; Sites issues assertions, hosted path carries them; agent tools consume them. | Product-owned issuer/delegation with browser-carried encrypted proof or explicit agent authorization. Do not recreate a general hosted human signer. |

The existing accepted Hosted Web Device ADR 0011 and binding ADR 0012 must be
superseded deliberately. Preserve the invariant that navigation is never
membership authority. Update the Recovery Set before retiring its old source;
the nsec plus ciphertext log cannot replace missing MLS/history material.

## Removal work packages and effort

These are planning estimates for one engineer familiar with this repository,
not measured delivery promises. They assume the current exported-nsec custody
choice, reuse of existing crypto and runtime-command formats, fresh accounts,
and no simultaneous native-client rewrite. Existing-user conversion is excluded.

1. **Real identity and launch integration — roughly 3–5 engineer-days.**
   Replace fixture login, key source, fake Project and fixed endpoints with
   authenticated Core identity, real Project creation and a provisioned Agent.
   Make the browser provider the intended route. Carry owner public identity
   and product-scoped grants; retain a single canonical Room discovery record.
2. **Admission, persistence and history reliability — roughly 5–8 days.**
   Close the KeyPackage/Commit journal crash gaps, serialize/retry enrollments,
   renew credentials, bound/persist history responses and metadata, test agent
   restart. Fix the observed Hermes inbox lease/reconnect failure. Add browser
   lifecycle/eviction behavior and agent-history recovery coverage appropriate
   to the promised durability. Full recovery design could extend this range.
3. **Feature coverage — roughly 8–15 days.**
   Blob/media support, ephemeral/projection parity, real agent-control commands,
   typed Brain/Sites proofs, verified requester context, and OAuth completion.
   These are separate bounded slices; tool/approval and cross-browser failures
   could increase the estimate. Browser nsec signing alone is not the whole
   feature implementation.
4. **Packaging, deletion and release gates — roughly 3–5 days.**
   Pinned versioned JS/WASM delivery; secure-context/CORS on product-owned public
   routes; replace loopback-only enrollment; production build and browser CI.
   Then remove the unused hosted service/client/routes and its deployment,
   secret, status and test dependencies. Preserve shared protocol/UI logic.

A useful **fresh-account, text-first milestone is around 2–3 focused weeks**
including a small bridge-absent CI/release slice. **Full service retirement with
the surrounding features preserved is around 4–8 engineer-weeks**. Confidence is
higher in the deletion inventory than the reliability/feature estimates.

The mechanical deletion is relatively small: about 2.9k lines of hosted Rust
service, 2.3k lines of its HTTP tests, three main dashboard provider/client/
orchestration files (~1.9k lines), and seven hosted chat route files. Some
contracts and tests must move, not vanish. This count excludes shared UI and
all feature callers; it is an inventory, not a productivity estimate.

Deletion inventory:

- `finitechat/crates/finitechat-hosted-device`, root Cargo membership/lock
  resolution, Nix package/build references. Preserve `finitechat-core` and
  `finitechat-cli` used by the Agent and native clients.
- Dashboard `hosted-chat-provider.tsx`, hosted HTTP client/orchestration and
  `/api/chat/machines/[machineId]/hosted-device/*` after moving shared context/
  DTOs and all remaining callers. Refactor creation, Connections, Sites and
  Brain callers before removing the client module.
- `infra/nixos/modules/finitechat-hosted-device.nix`, host imports, service
  dependencies, dashboard hosted URL/token environment, secret-bootstrap
  contract, CI closure gates, devfinity fixtures/smokes, monitoring,
  `scripts/finite_status.py` and deployment contract tests.
- Replace the hosted-specific portions of `backups.nix`, restore scripts and
  runbooks with the new Recovery Set. That snapshot also contains other product
  stores: do not delete the whole backup system because its name says hosted.
- Update custody/trust language and obsolete hosted-only tests/docs. Keep
  valuable continuity/recovery scenarios under their new owners.

## Acceptance gates and next slice

The next valuable slice is **real Core sign-in -> real provisioned Agent ->
new browser Device -> existing real dashboard**, with the hosted-device service
not launched and its URL/token configuration absent. Keep text, metadata and
history as the declared scope. This replaces the largest remaining fixture
assumptions without first porting every product feature.

The full deletion gate should run that same absence test and prove:

- First enrollment, returning browser, two independent profiles, simultaneous
  tabs and concurrent enrollment; no duplicate Room/Device from retry.
- Bidirectional chat/tools, Topic/Chat switching, future messages, and paged
  per-Chat history from the agent with no unrelated-chat backfill.
- Browser close/reload, lost delivery receipt, failed durable write, credential
  expiry, gateway/native-agent restart and disconnect-before-ack. A paused
  process is not sufficient proof of process-crash recovery.
- Site creation with correct verified ownership and embedded viewing, Brain
  approvals/grants, media in both directions, actual Connections commands,
  and Google completion through their replacement paths.
- No requests to hosted chat APIs or hosted signer APIs. Other product APIs
  are allowed only when they own an explicit control/authorization operation;
  none may become a plaintext chat relay.
- Same user Principal, bound Room and retained history after the promised
  recovery operation. Existing-account handoff requires its own evidence and
  approved cutover; no blanket claim that old hosted stores can be discarded.

Existing evidence reviewed, not rerun in this audit:
`/tmp/finitechat-final-browser-regressions-2.log`,
`/tmp/finitechat-multibrowser-test-5.log`, `/tmp/finitechat-history-test-2.log`,
and `/tmp/finitechat-connections-browser-check.log`. Those tests block dashboard
APIs other than the fixture bootstrap and verify direct signed encrypted relay
traffic. They establish the exercised chat paths, not Sites/Brain/Connections
feature parity. The production gaps and observed inbox lease problem remain
recorded in `HOW_TO_DO_IT_FOR_REAL_LOG.md`.
