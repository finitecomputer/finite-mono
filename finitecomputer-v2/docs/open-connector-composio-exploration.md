# Connector platform exploration: OpenConnector, Composio, and native integrations

- Status: exploratory, not an architecture decision
- Date: 2026-07-13
- Repository baseline: a8eac53
- OpenConnector snapshot: v1.1.0 /
  d0c9b9d9ae63496d8fcf3b17506fb55ebc657205

## Executive recommendation

Do not adopt OpenConnector as Finite's production connection plane yet. It is
promising as a reference implementation and test corpus, but its open-source
runtime is extremely young, its authorization model is not sufficient for a
shared multi-tenant Finite service, and its Slack integration is a collection
of Web API actions rather than a Hermes messaging channel.

Use a hybrid, product-owned approach:

1. Treat conversation channels and tool/data connectors as different product
   capabilities even if they share card styling on the Connections page.
2. Pursue Slack as a native Hermes channel. Hermes 0.18.2 already contains the
   adapter we need. First fix the gateway authorization default, bake the Slack
   dependencies into the runtime image, and choose a safe multi-workspace
   ingress design.
3. Keep Google native and make the official Google Workspace CLI, gws, the
   canonical execution surface if a spike proves that it can use Finite's
   existing OAuth credential. The current integration does not actually
   install or invoke gws, and current Drive and Docs support is too thin.
4. Add a small Finite connector catalog/status contract, then iteratively ship
   curated native connectors. Use OpenConnector's provider metadata, schemas,
   scopes, and executors as research material rather than making its runtime a
   required production dependency.
5. Keep Composio as a deliberate speed-to-breadth option or benchmark. A
   time-boxed pilot is reasonable if product demand requires many integrations
   quickly, but it changes credential custody, availability, cost, recovery,
   and portability.

The important distinction is:

| Capability | Examples | Correct integration boundary |
| --- | --- | --- |
| Conversation channel | Finite Chat, Telegram, Slack | Hermes platform adapter plus Finite-owned onboarding and status |
| Tool/data connector | Google Workspace, GitHub, Notion, Linear | Agent tools with provider-specific authentication and policy |
| Inference provider | OpenRouter, other model APIs | Model/runtime configuration |

The current page visually groups all three, but they should not be forced
through one connector abstraction.

## Current Finite baseline

The Connections page is a closed implementation, not a connector registry. It
hardcodes Inference, Telegram, and Google Workspace in
[connections-panel.tsx](../apps/dashboard/src/components/connections-panel.tsx).
The dashboard API and finite-agentd mirror those exact cases in
[hosted-agent-controls.ts](../apps/dashboard/src/lib/hosted-agent-controls.ts)
and [connections.rs](../../finite-agentd/src/connections.rs).

Google is therefore the only current general tool/data connector. Telegram is
a Hermes channel: it lets a user talk to the same agent from another messaging
surface. This difference should remain explicit in UI copy, status, auth,
policy, and tests.

The current native Google path has a valuable property: the dashboard performs
OAuth, then finite-agentd persists the credential inside the agent's durable
runtime state and the tool executes locally. Moving to a central connector
service or SaaS vendor would be a real trust and recovery change, not merely a
UI refactor.

There is also some debt to address before the catalog grows:

- Every connection action currently shares the resource key
  agent.connections. The Finite Chat control-plane design calls for
  per-connector keys so unrelated operations do not serialize together.
- Page load asks the live agent for aggregate status. The control-plane design
  prefers projected runtime snapshots for normal status reads.
- Current Google status checks for the token file and email metadata, not
  whether a refresh or harmless API operation still succeeds. A revoked token
  can therefore look connected.
- A typical native tool connector currently requires changes across the
  dashboard, provider-specific auth/API routes, typed runtime commands,
  finite-agentd, runtime dependencies, an agent-facing managed tool/skill when
  needed, and tests.

These are good reasons to introduce a small Finite-owned catalog and status
contract. They are not, by themselves, reasons to outsource execution or
credential custody.

## Slack: useful only as a Hermes channel

### Feasibility

Yes: Slack can work as a Hermes channel. The exact Hermes release Finite pins,
0.18.2, already ships a native Slack platform adapter. It supports Socket Mode,
DMs, channel mentions, thread follow-ups, files, edits, commands, approvals,
and reconnection. Slack threads map naturally to Hermes thread/session
semantics. Slack and Finite Chat would run side by side as separate adapters
feeding the same agent.

Slack should not be presented as another Finite Chat room. It has Slack's
identity, retention, administrator, and plaintext trust boundaries, not
Finite Chat's MLS properties. Participants in a Slack thread can also share
Hermes thread context, which requires an explicit product and prompt-injection
policy.

OpenConnector does not make this channel work. Its Slack provider exposes
outbound/query Web API actions such as posting, replying, searching
conversations, reactions, users, and files. No inbound event, trigger, webhook,
or subscription framework was found in the inspected release. Those actions
could complement a channel later, but they cannot transport the live
conversation.

Composio does have real-time Slack triggers and signed webhook delivery. It
could therefore feed a custom Finite-to-Hermes relay, but it is still not a
Hermes channel adapter by itself. Using it in the message path would add a
vendor hop to a capability Hermes already provides.

### Finite-specific blockers

1. The hosted gateway currently defaults global
   GATEWAY_ALLOW_ALL_USERS to true in
   [run_hermes_gateway.sh](../../finitechat/containers/agent/run_hermes_gateway.sh).
   A Slack-specific allowlist cannot narrow a global allow-all. Finite Chat can
   retain its own allow-all setting because its adapter authenticates the
   encrypted room, but Telegram and Slack need to fail closed through pairing
   or explicit allowlists.
2. The runtime image pins the base Hermes package but does not bake in
   slack-bolt, slack-sdk, and the other optional Slack dependencies. Production
   must not depend on a first-use network install.
3. finite-agentd has no typed Slack schema or private persistence for the bot
   token, app-level token, allowlist, and workspace metadata. Status and errors
   must always redact the tokens, and apply/disconnect must be atomic and
   recoverable.
4. Socket Mode's isolation boundary is the Slack app, not an individual
   app-level token. Slack allows up to ten WebSocket connections for one app
   and may deliver an event to any of them without affinity. Even separate
   app-level tokens for the same Finite-owned app would not isolate agent
   runtimes; allowing several agents to connect that app directly could route
   an event to the wrong agent.

The last item determines the product architecture:

| Option | Result | Assessment |
| --- | --- | --- |
| User creates a dedicated Slack app per agent and provides bot/app tokens | Native Hermes adapter can run entirely inside that agent runtime | Feasible manual/advanced MVP; poor mainstream onboarding |
| Finite owns a central Slack ingress and applies a durable agent-route policy | One product Slack app with a custom signed-HTTP Events API or Socket Mode relay into Hermes | Candidate polished topology; meaningful new service, routing policy, and recovery boundary |
| Composio receives Slack triggers and Finite routes them into Hermes | Avoids owning some Slack ingress/auth machinery | Fastest vendor-assisted experiment; extra dependency, cost, and custom adapter still required |
| Multiple agent runtimes directly connect the same Finite Slack app | Slack can send events to an arbitrary connection in the app's pool, even if the app-level tokens differ | Reject |

A central relay still needs an explicit routing cardinality: one agent per
workspace, sender-to-agent ownership, channel/thread binding, or an explicit
agent selector. A workspace ID alone is ambiguous when multiple Finite agents
share one Slack installation and bot identity. Hermes's built-in
multi-workspace support does not solve this distributed case; it brings
several workspace installations into one Hermes process and one agent loop.

Slack also says Socket Mode apps cannot be listed in the public Slack
Marketplace and recommends the HTTP Events API for deployed production where
feasible. A polished design should therefore compare central signed-HTTP
ingress with central Socket Mode rather than assuming Socket Mode is final.

### Slack recommendation

Green-light a small native Hermes Slack spike only after the global
authorization and image prerequisites are fixed. Start with a sandbox and
either dedicated test-app credentials or a central relay prototype. Do not
adopt OpenConnector or Composio as the Slack channel transport merely because
they expose Slack tools.

## Google: current support and gws compatibility

### What Finite has today

The dashboard obtains a Google authorized-user credential and finite-agentd
writes it into the Hermes home. The managed Google skill then uses a custom
Python wrapper. The skill mentions gws only as an independently installed,
independently authenticated alternative; the production runtime image does not
install it.

Actual capability is uneven:

| Service | Current native support | Current assessment |
| --- | --- | --- |
| Gmail | Search, basic message fetch, send, reply, list labels, add/remove labels | Useful baseline; missing robust MIME/attachments, drafts, pagination, richer thread operations, BCC, and stronger write safety |
| Drive | Metadata search returning IDs and links | Not good enough; no content/export/download, upload, folder operations, sharing, permissions, or comments |
| Docs | Fetch by ID and flatten direct paragraph text | Not good enough; read-only and misses tables, headers, footers, footnotes, edits, and comments |
| Sheets | Read, update, append ranges | Reasonable basic support |
| Calendar | List, create, delete events | Basic support |
| Contacts | Read basic names, emails, and phones | Basic support |
| Apps Script | Scopes granted | No corresponding action |

The current grant includes full Drive plus Apps Script project/deployment
management scopes despite exposing little or no corresponding behavior, while
Docs is read-only. That capability-to-scope ratio should be corrected as part
of any Google work.

### Can it use the same gws credential?

Probably, but this needs a real spike before it becomes a contract.

The official gws CLI supports a credentials-file environment variable and
parses authorized-user JSON containing client ID, client secret, and refresh
token. Finite's google_token.json contains those fields. Code inspection
therefore indicates that gws can likely read the credential already issued by
the dashboard without another user consent flow.

That compatibility is not wired or tested today. A safe adoption requires:

- pinning gws in the runtime image rather than downloading it at first use;
- pointing GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE at the private
  google_token.json written by finite-agentd;
- testing credential import and refresh using synthetic state before touching a
  production credential;
- reconciling exact scopes with the Gmail, Drive, and Docs operations Finite
  intends to expose;
- adding real, harmless canaries for Gmail read, Drive content retrieval, and
  structured Docs read;
- adding explicit approval/idempotency policy for destructive or externally
  visible writes.

OpenConnector and Composio do not use gws internally. Both execute their own
provider actions. OpenConnector also models Gmail, Google Docs, and Google
Drive as separate providers with separate OAuth configuration, so it would not
automatically reuse Finite's single Google Workspace connection. Composio can
import an existing access token, but its documented import flow does not
refresh that token; Finite would have to refresh and update it. This is not a
drop-in reuse of the current Google connection.

### Google recommendation

Keep Google native. First spike gws against a disposable authorized-user
credential in the runtime image and compare its structured results and failure
behavior with the current wrapper. If it passes, make gws the single canonical
Google execution surface and retire duplicate wrapper behavior incrementally.
If it does not, expand the native wrapper using the same OAuth and runtime-local
credential model. Do not introduce a second Google consent flow.

## OpenConnector evaluation

### What is attractive

OpenConnector is an Apache-2.0 TypeScript gateway with a large generated-style
provider/action catalog, SDK, CLI, MCP, HTTP/OpenAPI, local console, OAuth and
API-key handling, encrypted-at-rest credential support, action schemas,
allow/block policy, and redacted run logs. It can run under Node/Docker with
SQLite, on Fly.io, or on Cloudflare Workers with D1/R2.

The inspected catalog is valuable source material for:

- provider IDs, labels, scopes, auth variants, and setup instructions;
- action input/output schemas and lazy-loaded executors;
- OAuth refresh behavior and provider edge cases;
- a repeatable provider directory convention;
- comparing the breadth and ergonomics of a candidate native connector.

This could shorten research for selected native integrations. If Finite copies
code rather than merely learning from it, Apache-2.0 license and NOTICE
obligations must be preserved.

### Why it is not a production fit yet

The project began on 2026-06-30 and reached v1.1.0 on 2026-07-13. Its pace and
catalog breadth are impressive, but two weeks of public history is not enough
operational evidence for a component that would hold user refresh tokens and
execute externally visible actions.

More importantly, its own ConnectServer documentation calls the open-source
runtime a local single-user HTTP server:

- Stored credential identity is service plus connection name; it has no Finite
  principal, machine, or tenant dimension.
- Persistent runtime tokens authenticate callers but are not bound to one
  user, one connection alias, or a per-user action policy. A token reaching a
  shared runtime could select another named connection.
- Action allow/block policy is runtime-global.
- Admin and runtime-token authentication are optional and default off for local
  use. The Node server defaults to loopback, but the supplied Docker
  configuration binds to all interfaces; a production deployment has to set
  and preserve every authentication and network control deliberately.
- Credential encryption is optional. Without
  OOMOL_CONNECT_ENCRYPTION_KEY, the runtime remains usable and warns that it is
  storing plaintext. Losing the encryption key makes the stored credentials
  unusable.
- OAuth is its own flow with bring-your-own client configuration. No supported
  public API for importing Finite's existing Google refresh credential was
  found.
- No general inbound trigger/event framework was found, so it cannot replace a
  Hermes platform adapter.

These facts leave two possible deployment shapes, neither compelling today:

| Deployment | Benefit | Cost/risk |
| --- | --- | --- |
| One OpenConnector per agent runtime | Avoids cross-tenant alias access; credentials stay near the agent | Adds SQLite state and its separately held encryption key to the Recovery Set, plus migrations, process health, and action-runtime operations |
| One shared Finite OpenConnector service | Central operations and catalog | Unsafe without adding tenant-bound identity, ACLs, token scope, routing, audit, backup, and recovery semantics |

OOMOL's hosted ProjectConnector has an external-user concept and is a
different, SaaS-style product path. That should not be confused with the
authorization properties of the open-source self-hosted runtime. Public
pricing, security/compliance evidence, retention and residency, SLA, trigger
support, and credential exit/export were not established in this pass; none
should be inferred from the Apache-licensed runtime.

Provider count is also not a quality guarantee. No provider-specific tests for
Slack, Gmail, Google Docs, or Google Drive were found in the inspected tree.
The evaluation unit should be one provider's auth lifecycle, action semantics,
pagination, files, rate limits, error handling, write safety, tests, and
recovery—not the catalog headline.

### OpenConnector recommendation

Use it as a pinned reference and optional disposable development spike. Do not
put it in the production credential or action path until it has materially more
operational history and Finite has either:

- proven a per-agent deployment and expanded the Recovery Set accordingly; or
- implemented and tested first-class tenant-bound authorization in a shared
  deployment.

## Composio comparison

Composio is a hosted integration platform with a much longer-lived SDK and
catalog, managed authentication, multi-user connection identities, action
execution, tool discovery, and real trigger delivery. Its SDK is open source;
the hosted credential/execution control plane is the product dependency.

| Dimension | OpenConnector OSS | Composio | Curated native Finite |
| --- | --- | --- | --- |
| Fast initial breadth | Very high catalog claim | Very high, mature catalog | Low; one integration at a time |
| Multi-user SaaS model | Insufficient in inspected OSS runtime | First-class user/account connections | Finite-owned and tailored |
| Self-host/control | Apache-2.0 runtime, inspectable | Hosted by default; enterprise deployment options require a commercial agreement | Full |
| Credential custody | Finite if self-hosted; new database/key | Composio service | Agent runtime/Finite |
| Inbound triggers | No general framework found | Yes, including real-time Slack triggers | Per-channel/provider implementation |
| Slack as Hermes channel | No | Not without a custom relay | Native Hermes adapter already exists |
| Existing Finite Google grant | No supported import path found | Access-token import, but Finite must refresh/update it | Already owned; likely reusable by gws |
| Recovery/availability | Finite operates an additional stateful component | External Recovery Authority and vendor outage dependency | Existing runtime recovery model, expanded per connector |
| Lock-in | Moderate; open contracts and code | Higher; hosted auth, execution, trigger, version, and billing contracts | Engineering cost rather than vendor lock-in |
| Ongoing cost | Infrastructure and engineering | Usage-based subscription/API pricing | Engineering and support |
| Operational maturity | Very young as of this snapshot | Substantially more established | Depends on Finite's own tests and operations |

As of this exploration, Composio advertises 20,000 calls/month free; a
$29/month tier with 200,000 calls and $0.299 per additional thousand; and a
$229/month tier with two million calls and $0.249 per additional thousand.
Enterprise VPC/on-premises terms require sales engagement. These terms are
volatile and must be rechecked before any decision. More important than
headline call price are trigger billing, file transit, retries, rate limits,
support, data retention, breach terms, export, and the cost of replacing the
vendor. Composio's Usage API says both successful and failed tool executions
count; public documentation did not make trigger-delivery billing clear.

Composio's current documentation is inconsistent about managed Google apps:
the global managed-auth list includes Gmail, Docs, and Drive, while the
individual Docs and Drive toolkit pages say a managed app is unavailable.
Gmail and Slack pages advertise managed auth. A pilot must resolve this in a
real project and exercise the exact toolkits, auth modes, and triggers Finite
would buy rather than infer quality from aggregate catalog counts.

Composio claims SOC 2 and ISO 27001:2022 compliance and offers enterprise
deployment controls, but this pass did not verify audit artifacts, retention,
residency, subprocessors, incident terms, deletion SLAs, or credential export.
Those are pre-production diligence gates, not check-box assumptions.

### When Composio would be the right choice

Run a Composio pilot if a validated product requirement says Finite must ship
several non-channel integrations faster than a native cadence can support, or
if a central trigger service avoids enough work to justify the dependency.
Require an exit plan and keep the Finite-facing tool contract provider-neutral.

Do not choose it merely to improve Google or enable basic Slack messaging:
Finite already owns the Google credential path, gws is a promising native
execution surface, and Hermes already owns the Slack channel semantics.

## Proposed native evolution

The native path need not mean repeating every UI and status implementation
from scratch. Introduce a deliberately small catalog that normalizes
presentation and lifecycle without pretending all integrations behave alike.

A connector descriptor could contain:

- stable ID, display name, category, and capability badges;
- auth owner and setup route;
- status shape and health-check freshness;
- disconnect/revoke behavior;
- required runtime package/skill and version;
- recovery material by name and location;
- sensitivity and external-write policy;
- maturity state such as experimental, beta, or supported.

Keep provider-specific typed apply/status/disconnect commands behind that
catalog. Keep channel schemas separate from tool/data schemas. Give each
connection its own resource key. Project redacted connection status into the
runtime snapshot rather than issuing a durable command on every page view.

For each new connector:

1. Start from a concrete user workflow, not a provider count.
2. Inspect official API documentation and use OpenConnector as a secondary
   implementation reference.
3. Define the least scopes and the explicit read/write capability set.
4. Keep credentials runtime-local unless a reviewed architecture explicitly
   chooses another Recovery Authority.
5. Add the managed runtime dependency and skill contract at a pinned version.
6. Prove disconnect, revocation, rotation, backup, restore onto an empty target,
   and vendor outage behavior.

## Suggested sequence

### Phase 0: record decisions before implementation

- Decide whether polished Slack onboarding justifies a central Finite ingress,
  or whether a user-created Slack app is acceptable for an experimental mode.
- Decide whether vendor-held connector credentials are ever acceptable and,
  if so, for which connector classes and data sensitivity.
- Choose the next non-Google tool connector from observed user demand.

### Phase 1: make Google genuinely good

- Spike pinned gws with the existing credential file in an isolated runtime.
- Define the Gmail, Drive, and Docs acceptance set and exact scopes.
- Add structured read canaries and write approval/idempotency tests.
- Choose gws or the Python wrapper as the only canonical implementation.

### Phase 2: prove native Slack channel behavior

- Remove global gateway allow-all while keeping Finite Chat behavior intact.
- Bake the exact Slack dependencies into the runtime image.
- Test the Hermes adapter in a sandbox workspace.
- Prototype the chosen token/ingress topology.
- Add typed, redacted, atomic finite-agentd lifecycle only after the topology
  passes.

### Phase 3: establish the connector catalog with one demanded integration

- Add the minimal descriptor/status layer.
- Implement one provider natively end to end.
- Compare its build time, defects, and maintenance burden with the
  OpenConnector reference implementation.

### Phase 4: optional Composio benchmark

- Use a test tenant and a low-risk provider not already solved well natively.
- Exercise auth, refresh, pagination, files, triggers, revoke, export, outage,
  and deletion.
- Compare latency, task success, engineering effort, support, recovery, and
  projected cost with the native connector.

## Evaluation gates

No candidate should be promoted based on a successful OAuth callback and one
happy-path action.

### Google/gws

- Existing Finite authorized-user JSON refreshes successfully without a second
  consent flow.
- Gmail can read representative multipart messages and attachments, preserve
  thread behavior, and gate sends.
- Drive can find and retrieve native Google files and ordinary files with
  pagination.
- Docs returns structured content including tables and can perform only the
  explicitly approved write set.
- Revoked credentials become unhealthy promptly and status never exposes
  secrets.
- The same credential set restores onto an empty target according to the
  Recovery Set.

### Slack channel

- Unauthorized Slack users fail closed; removing global allow-all does not
  regress Finite Chat or Telegram.
- DM, mention, thread follow-up without repeated mention, file ingress, edits,
  approvals, restart, revoke/rotate, reinstall, outage, and reconnect all work.
- Events can never be delivered to the wrong Finite agent in the selected
  multi-workspace topology.
- Bot/app tokens never appear in logs, status, command arguments, errors, or
  results.
- UI states clearly that Slack does not inherit Finite Chat's encryption and
  identity guarantees.

### Any connector platform

- Tenant isolation is tested with two users, two machines, and identically
  named connections.
- Runtime tokens are least-privilege and cannot select another user's alias or
  actions.
- Credential storage, encryption-key rotation, backup, empty-target restore,
  disconnect, provider revocation, and deletion are proven.
- Pagination, rate limits, retries, idempotency, file transit, and partial
  failure are tested against synthetic state and a provider sandbox.
- Destructive or externally visible actions require the intended approval and
  cannot duplicate on retry.
- The provider remains usable or fails intelligibly during connector-platform
  outage.
- Finite has an export/replacement path before accepting a new external
  Recovery Authority.

## Sources

### Finite repository

- [Connections page](../apps/dashboard/src/components/connections-panel.tsx)
- [Hosted connection controls](../apps/dashboard/src/lib/hosted-agent-controls.ts)
- [Google OAuth scopes](../apps/dashboard/src/lib/google-workspace-oauth.ts)
- [finite-agentd connection lifecycle](../../finite-agentd/src/connections.rs)
- [Google Workspace managed skill](../../finite-skills/skills/productivity/google-workspace-finite/SKILL.md)
- [Current Google API wrapper](../../finite-skills/skills/productivity/google-workspace-finite/scripts/google_api.py)
- [Canonical runtime image](../deploy/finite-computer/images/runtime.Dockerfile)
- [Hermes gateway startup](../../finitechat/containers/agent/run_hermes_gateway.sh)
- [Finite Chat Hermes integration](../../finitechat/docs/hermes-integration.md)
- [Agent-owned platform boundary](../../docs/adr/0003-agentd-is-the-agent-owned-platform-boundary.md)
- [Recoverability invariant](../../docs/adr/0001-recoverability-precedes-operator-blindness.md)

### External primary sources

- [OpenConnector repository](https://github.com/oomol-lab/open-connector)
- [OpenConnector credential storage](https://github.com/oomol-lab/open-connector/blob/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/docs/credentials.md)
- [OpenConnector configuration and authentication defaults](https://github.com/oomol-lab/open-connector/blob/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/docs/configuration.md)
- [OpenConnector single-user server boundary](https://github.com/oomol-lab/open-connector/blob/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/src/server/connect-server.ts)
- [OpenConnector runtime schema](https://github.com/oomol-lab/open-connector/blob/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/migrations/0001_runtime.sql)
- [OOMOL ProjectConnector SDK](https://github.com/oomol-lab/connector-sdk)
- [OpenConnector Slack provider](https://github.com/oomol-lab/open-connector/tree/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/src/providers/slack)
- [OpenConnector Gmail provider](https://github.com/oomol-lab/open-connector/tree/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/src/providers/gmail)
- [OpenConnector Google Docs provider](https://github.com/oomol-lab/open-connector/tree/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/src/providers/googledocs)
- [OpenConnector Google Drive provider](https://github.com/oomol-lab/open-connector/tree/d0c9b9d9ae63496d8fcf3b17506fb55ebc657205/src/providers/googledrive)
- [Hermes 0.18.2 Slack guide](https://github.com/NousResearch/hermes-agent/blob/v2026.7.7.2/website/docs/user-guide/messaging/slack.md)
- [Hermes 0.18.2 Slack adapter](https://github.com/NousResearch/hermes-agent/blob/v2026.7.7.2/plugins/platforms/slack/adapter.py)
- [Slack Socket Mode delivery and multiple-connection model](https://docs.slack.dev/apis/events-api/using-socket-mode/#using-multiple-connections)
- [Google Workspace CLI](https://github.com/googleworkspace/cli)
- [gws authorized-user credential loading](https://github.com/googleworkspace/cli/blob/main/crates/google-workspace-cli/src/auth.rs)
- [Composio documentation](https://docs.composio.dev/)
- [Composio triggers](https://docs.composio.dev/docs/triggers)
- [Composio webhook subscriptions](https://docs.composio.dev/reference/api-reference/webhook-subscriptions)
- [Composio existing-connection import](https://docs.composio.dev/docs/importing-existing-connections)
- [Composio managed-auth catalog](https://docs.composio.dev/toolkits/managed-auth)
- [Composio usage accounting](https://docs.composio.dev/reference/api-reference/organization)
- [Composio Slack toolkit](https://docs.composio.dev/toolkits/slack)
- [Composio Gmail toolkit](https://docs.composio.dev/toolkits/gmail)
- [Composio Google Docs toolkit](https://docs.composio.dev/toolkits/googledocs)
- [Composio Google Drive toolkit](https://docs.composio.dev/toolkits/googledrive)
- [Composio pricing](https://composio.dev/pricing)
