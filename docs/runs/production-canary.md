# Internal production canary

Status: ACTIVE
Owner: Paul
Opened: 2026-07-10
Acceptance: I, Paul, use a fresh non-admin account in a normal browser to sign up → redeem a Launch Code → launch → complete a two-way agent turn → restart the Agent Runtime from the dashboard → complete another two-way turn in the same visible conversation.
Expires: 2026-07-24

Outcome: prove the production golden path internally. Passing this run authorizes preparation of the separate paid-customer-cohort run; it does not authorize admitting customers.

This is the active work queue. Paul blessed its scope and the parity calls below through the 2026-07-10 grilling session. It operates under [the monorepo doctrine](../monorepo-doctrine.md), [ADR 0001](../adr/0001-recoverability-precedes-operator-blindness.md), and [ADR 0003](../adr/0003-agentd-is-the-agent-owned-platform-boundary.md). It must defend the thin-coupling wall: no feature-specific Runner, Runtime Management, or remote `.env` control path. If it is incomplete at expiry, work stops for explicit extension or replacement; expiry is not acceptance.

## No canary fork

The canary uses the intended customer architecture and normal product path. A canary-only concession must be explicit, narrowly bounded to the internal cohort, fail closed outside that boundary, and leave a named removal or proof obligation for the paid-customer run. Do not add an identity bypass, operator-only happy path, parallel data model, provider-specific product behavior, or temporary control-plane coupling that the customer path would inherit.

## Resolved run boundary

ADR 0001 governs this internal canary: provider-durable state and honestly limited recovery claims are sufficient for this run. The paid-cohort backup and empty-target restore gate in `infra/README.md` remains intact for the next run. Passing this canary neither weakens that gate nor chooses final Recovery Snapshot or key-custody architecture.

This canary exercises the Core-owned Launch Code path used for white-glove training and other approved sponsored access. A Launch Code is not a Stripe promotion code and does not prove billing. The next paid-customer run must separately exercise Stripe Checkout, webhook-to-Core entitlement, and the resulting launch path.

Runner placement is internal product policy. The canary uses the standard Kata class on `finite-lat-1`; onboarding does not expose Kata, Phala, or provider handles. Phala remains a later conformance target behind the same Runner contract.

## Active queue

Every item retained below is required for closure. Priority expresses work order and risk, not optionality. If an item stops being required for this canary, Paul moves it to the parking lot or a later proposed run before work continues.

### P0 — make Core identity trustworthy at the service boundary

Core still accepts caller-supplied `x-finite-workos-*` identity headers behind one shared `FC_CORE_API_TOKEN`; the dashboard sends those headers and the postmortem records that a token holder can impersonate another user. Use the idiomatic WorkOS AuthKit boundary: the Dashboard forwards the standard access-token JWT, and Core validates its WorkOS JWKS signature, issuer, client id, expiry, and subject on every user-scoped request. Finite operators belong to one configured internal WorkOS operator organization, separate from Core Customer Organizations. For this one-operator canary, every admin route uses one operator predicate derived from the validated `org_id`; do not build a permission taxonomy yet. Give Runner and other services separate route-scoped credentials that cannot call user/admin routes or assert a WorkOS subject. Do not mint a parallel Finite session token. Launch Code Batch issuance must not ship until Core independently enforces this boundary.

Done means tests reject expired, wrong-issuer, wrong-client, and invalidly signed tokens; reject spoofed or mismatched identity headers; reject Runner credentials on user and admin routes; reject every admin route when the configured operator `org_id` is absent or wrong; accept the intended user and operator requests; keep WorkOS operator and Core customer organization ids distinct; and preserve normal Runner lease operations through its own credential.

### P0 — close the trusted-canary owner-claim gap

The existing `agent.owner.claim` command durably authorizes the first already-admitted Finite Chat Principal, but normal Chat bootstrap does not send it; Connections sends it only when that surface is opened. Before returning a usable Chat or Connections surface, make Hosted Web Device bootstrap require a successful claim for Paul's Principal. Hide the copyable Agent address/cross-device connection affordance for this entire run; raw npubs, multi-device enrollment, and arbitrary external room admission are outside it.

This is the explicitly trusted internal-canary use of the existing first-admitted claim, not the customer authorization design. It must not add Dashboard, Core, Runner, or provider-shell mutation authority. Core-bound or equivalently attested enrollment before untrusted admission remains a customer-admission requirement in `docs/open-questions.md`.

### P0 — replace the public shared Launch Code with single-use batches

Core currently accepts the repository-visible `off2026` value for any new organization. Replace it with a dashboard backed by Core that issues named Launch Code Batches containing an explicit number of individually single-use codes. Issuance and revocation require a validated session in Finite's configured internal WorkOS operator organization. Return plaintext codes once for copy/download; later views expose only expiry, revocation, and redemption metadata. Batches default to seven days, have an explicit maximum of 30 days, and cannot be indefinite. Redemption must be atomic and idempotent for the same account/request, and code values must not appear in source, logs, URLs, argv, later reads, or ordinary audit output. Generate a 24-hour one-code batch from Paul's operator session, then redeem it and run the product walkthrough from a distinct fresh non-admin account. The same path must generate a batch of 12 for a 12-person training without changing the Agent Runtime contract. An optional CLI may call the same Core API; database edits and provider shell access are not issuance paths.

Keep it small: one minimal Core-owned batch/code model, issuance/list/revoke operations, one admin screen with one-time copy/download, and the existing redemption/creation path. The organizer distributes codes. Do not build campaign management, a participant roster, invitation delivery, scheduling, analytics, or a separate entitlement service.

### P0 — make the normal path discoverable

`marketing-home.tsx` currently offers only an external Google Form, while `/signup` and `/login` already exist. For the canary posture, expose **Sign in** and **I have a Launch Code** through the existing WorkOS account flow, and retain the request-access form for everyone else. Do not advertise open paid/self-serve launch until the customer-facing run, create a second auth system, or turn marketing into a control plane.

### P0 — preserve the existing Finite Private runaway guard

Finite Private limits exist to interrupt a runaway agent loop, not to meter customer dollars. Do not tune, backfill, or redesign the deployed policy for this canary. Read-only verification must confirm that the active profile has an effective finite guard and that an over-limit reservation is denied before upstream inference. If the live policy is unexpectedly unbounded or enforcement is inactive, stop and show Paul; do not silently mutate production or build pricing, alerting, or cost dashboards.

The currently deployed legacy-built limiter already generates a fresh accounting request id for every upstream attempt. The mono-built replacement regressed by trusting caller-supplied `x-request-id`, which permits reservation reuse. Do not deploy the recorded mono limiter digest until the existing live behavior and its duplicate-client-id regression test are ported: generate the accounting id inside the limiter for every accepted attempt and use it only for that attempt's reserve/settle pair. No Core fingerprinting system or broader metering redesign is required for this fix.

### P0 — verify the production Kata Runner path

The product contract and dashboard policy select Kata for production, and the Nix module defines an enabled Kata Runner timer, while `infra/README.md` still calls the Runner dormant because of an older Phala/Enclavia path. Before creating the canary request, verify the live Kata timer, route-scoped Core credential, capacity, promoted Runtime artifact, durable-volume binding, and readiness path. Reconcile the operational docs from that evidence. Do not add provider selection UI or use the canary to finish Phala.

**Status: BLOCKED — do not create the canary request.** Read-only production
preflight at 2026-07-10T19:03Z found `finite-saas-runner.timer` enabled and
active on `finite-lat-1`, with its last service exit `0`. Its configured Kata
lane selects `finite-agent-runtime-2026-07-10.5`, capacity is 12 total / 2
active at 4 CPU and 8G per Runtime, and the observed launch shape has two
read-write host binds at `/data`. This is configuration evidence, not product
readiness evidence: artifact `.5` returned `503` from both `/healthz` and
`/contact`; `finite-agentd` reported `bridge status stream_error` because the
Finite Chat inbound stream ended. The older `.2` artifact returned `200` for
those endpoints. The live runner environment also has only the shared
`FC_CORE_API_TOKEN`; the required route-scoped `FC_CORE_RUNNER_API_TOKEN` is
not deployed. No production state was changed. The Kata launch, contact,
two-way-turn, and restart-preservation checks remain unverified until a
route-scoped Runner credential is deployed and the selected artifact reaches
readiness.

### P1 — keep dashboard chat state coherent under real latency

A recent CI product-flow failure showed that an older Hosted Web Device HTTP snapshot can arrive after a newer SSE snapshot and remove the chat-derived Finite Sites preview. The latest rerun passed, so this is a latency-sensitive regression rather than a currently reproducible permanent outage. The Dashboard receives a `rev`, but currently replaces state unconditionally; the Hosted Web Device initializes that revision at zero whenever a per-user runtime is reopened, so a process-lifetime maximum is not restart-safe.

Keep the fix in the Dashboard: treat each SSE connection as a new local source generation, let its first full state establish that generation's baseline, and then apply only snapshots with a newer `rev`. Associate HTTP state/action responses with the source generation in which they began and discard them if the connection generation changed before they completed. Do not add a persisted global revision or a new state service. Regression coverage must prove that a slow older HTTP response cannot overwrite a newer SSE state, and that the first full state after reconnect is accepted even when its `rev` is lower than the prior connection's.

### P1 — hide the Skills dead end

The chat menu links every person to `/dashboard/skills`, while that page redirects non-admins when Core is configured. Hide the Skills navigation item from non-admin users for this canary and keep the existing admin surface. Do not add installed-state controls, automatic sync, or a second skills source. The later customer-facing run may add a read-only catalog plus honest explicit `finite skills sync` guidance.

### P1 — hide the unfinished Brain entry points

The dashboard and chat navigation currently expose Brain, and the dashboard Brain page still embeds the existing Product Client iframe through the Brain proxy. That work is not lost, but the iframe requires a NIP-07 signer and Folder Key access that WorkOS does not provide. Hide Brain from both SaaS and admin navigation for this run without deleting the iframe/proxy implementation. Brain remains reachable only as deliberate development work and is not part of canary acceptance. Re-enabling it requires the Principal, signer, and existing-Vault decisions recorded in `docs/open-questions.md`; Electron is the likely local-signer client but is not wired yet.

### P1 — make normal product copy speak like the product

Run a focused copy pass over the non-admin landing, signup, onboarding, Agent, Connections, and Chat paths before using Launch Codes for training. Current leaks include “Reconnecting to your Hosted Web Device…”, “Opening your Hosted Web Device…”, “Hermes can suggest a title…”, relay/heartbeat status, raw npubs and Agent addresses, and “WorkOS” in user-reachable errors. Replace infrastructure and protocol language with plain descriptions of what the person can do or what the product is doing. Keep precise terms in logs, admin/development surfaces, and technical docs.

This is a copy-only product pass, not a design-system, localization, or error-framework project. Done means the known strings are gone from the normal path, nearby user-reachable errors receive the same treatment, and targeted UI/route tests pin the important replacements.

## Legacy parity checklist — resolved dispositions

The goal is to carry forward the experience, not legacy control-plane coupling.

| Legacy capability | Current SaaS evidence | Disposition |
| --- | --- | --- |
| Conversation sidebar, home-first topics, chats beneath topics, per-topic and global New chat | Present in `hosted-web-chat.tsx` | **KEEP — shipped** |
| Provisional chat title and user rename | Rename is shipped; Hermes-generated naming is not proven | **KEEP** rename/fallback; **LATER** verify agent naming if wanted |
| Typing, thinking, working, sidebar activity, and rolled-up tool work | Present over the event stream | **KEEP — shipped** |
| Reconnect/recovery UI | Present without polling | **KEEP** the UX; **CUT** legacy polling fallback |
| Load-earlier history and return-to-latest behavior | Present | **KEEP — shipped** |
| Text composer, Enter/Shift+Enter, drag/drop/paste/file picker, previews, limits | Present | **KEEP — shipped** |
| Image/file render, download, and share | Present for uploaded media | **KEEP** media; **LATER** remote-Markdown image-card polish |
| Slash-command picker | Not present | **LATER**, only behind a stable product/device action contract |
| Stop an in-flight Hermes turn | Present in legacy, absent in SaaS | **LATER** — consciously excluded from this run; improve the simplicity and quality of talking to the agent rather than ship a fake infrastructure-level cancel, then define honest end-to-end turn cancellation separately |
| Split chat/browser preview, mobile preview, select/copy/open/reload | Present with authenticated session exchange, sandbox, and no-referrer iframe | **KEEP — shipped** |
| Published-app discovery/list and sharing editor | Legacy provides it; SaaS only discovers URLs for preview | **LATER** independent Finite Sites list/share after authority is resolved; **CUT** legacy app lifecycle controls |
| Account footer and product navigation | SaaS has Agent, Connections, Brain, Skills, sign out; chat hides the shell header | **KEEP** Agent, Connections, Chat, and sign out; **HIDE** Brain everywhere and Skills for non-admins until their real contracts are usable; **CUT** legacy OpenCode link |
| Agent status and restart/stop | Present through Core's narrow lifecycle contract | **KEEP — shipped** |
| Inference choices | SaaS intentionally exposes Finite Private and OpenRouter | **KEEP** those two; **CUT** Codex/custom endpoint and remote config surgery |
| Telegram pairing and home chat | Present | **KEEP — shipped** |
| Google Workspace connection | Present | **KEEP — shipped** |
| Matrix connection | Outside requested SaaS connection scope | **CUT for this run** |
| Skills visibility and `finite skills sync` guidance | Normal users hit the redirect described above | **HIDE for canary**; **LATER** read-only catalog and explicit-sync guidance |
| Legacy fleet, manual provisioning, invite/claim, Gitea/repo, host inspection | Explicitly excluded from SaaS dashboard scope | **CUT** |

## Out of scope for this run

- Customer admission and the paid-cohort backup/restore gate; these belong to the next proposed run.
- Full Recovery Snapshot, final key custody, and Electron/device-unification architecture; see [open questions](../open-questions.md).
- Brain Principal/Vault migration and the Electron signer bridge; see [open questions](../open-questions.md).
- Turn cancellation and dashboard Finite Sites list/share authority; see [open questions](../open-questions.md). The absence of a Stop Hermes button in this run is deliberate, not an overlooked parity item.
- Customer-facing stuck-launch timeout, cancellation, and provider cleanup; see [open questions](../open-questions.md).
- A normal-user Skills catalog, explicit-sync guidance, and any agent-installed-state surface; see [parking lot](parking-lot.md).
- Runtime-upgrade rollout completion, Kata CI expansion, legacy control-plane cleanup, and the skills-catalog source migration; see [parking lot](parking-lot.md).
- New provider integrations, Runner-specific feature commands, polling, remote configuration/file editing, or a second chat product.

## Final acceptance — run the production browser canary

Local Devfinity smoke and the dashboard browser test use local/dev or fake services. The PRD still has no checked full-stack normal-browser proof, and the prior pairing postmortem identifies first-message MLS delivery as the likeliest hard failure. This item is the run acceptance, not a code-only substitute: Paul uses a fresh non-admin account and the normal production product to complete one real two-way agent turn, restart that Agent Runtime through the dashboard, watch it return to ready, and complete a second two-way turn in the same visible conversation. Shared Finite Chat server and Hosted Web Device process restarts remain automated integration coverage rather than manual canary ceremony. Acceptance requires no worksheet, evidence form, debug surface, shell, database inspection, or operator reconstruction. If ordinary use does not make success clear, the canary fails. An agent may append a concise note from facts already available during the run, but no evidence generator, report schema, new instrumentation, or machine-readable artifact is required for closure.
