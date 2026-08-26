# `trycompai/crm`: source-level review and FiniteBrain fit

Date: 2026-08-25

Upstream snapshot: the `release` branch at
[`6d4793dd6d7aeea91aa6a034e00b17d7408a2d08`](https://github.com/trycompai/crm/tree/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08),
audited on 2026-08-25. That revision is the merge that put `v1.15.3` on
`release`; its tree is byte-identical to both the `v1.15.3` tag and `main` at
[`3c3e07a424f761c2a2f09c05f111dd1b61a29c94`](https://github.com/trycompai/crm/tree/3c3e07a424f761c2a2f09c05f111dd1b61a29c94).

This review uses primary sources only: the upstream source, schema, migrations,
first-party documentation, release metadata, and a small number of upstream
issues that expose current operational or product boundaries. The upstream
repository was read at the pinned revision above; current GitHub activity
figures are explicitly dated where used.

## Executive answer

`trycompai/crm` is a valuable reference implementation for **a Brain-native
Relationship Ledger**, but it is not a good candidate to embed wholesale inside
FiniteBrain or to make the canonical store for an Organization Brain.

The transferable core is unusually strong:

- a compact company/contact/deal/activity model with owner, stage, archive,
  custom-field, saved-view, and relationship semantics;
- a durable work queue in which scheduled work is a row with a reason, budget,
  due time, lease, attempts, and outcome;
- an evidence ledger that separates an observation from a fact, preserves its
  source and disposition, and prevents an agent from silently replacing
  human-authored data;
- typed event, trigger, action, scope, audit, and immutable agent-version
  concepts; and
- one programmatic surface shared by the UI and external clients: tRPC domain
  procedures exposed through a generated REST/OpenAPI bridge.

Those pieces are directly relevant to a Brain-native relationship system. The
upstream product is itself designed around the idea that the CRM is where an
agent keeps structured notes, with a separate durable agent process doing the
research and follow-up work ([README lines
43-60](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L43-L60)).

The non-transferable boundary is just as important. Upstream deliberately has
one shared workspace, no tenant key on CRM records, and no record-level access
control. Any authenticated user may reach the record routers; owner/admin/member
roles govern a subset of administrative operations rather than the CRM data
surface ([API rules lines
34-55](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L34-L55),
[auth middleware lines
11-25](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/trpc/middlewares/auth.middleware.ts#L11-L25),
[workspace roles lines
8-38](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/organization.ts#L8-L38)).
Its Postgres contains plaintext CRM data, mailbox bodies, OAuth tokens, agent
transcripts, and enrichment material, and the upstream security policy says the
operator can read all of it ([security policy lines
14-42](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L14-L42)).
That conflicts with FiniteBrain's Folder-scoped encrypted-content boundary, in
which the server must not need Page paths, titles, links, or contents in
plaintext ([FiniteBrain portability spec](../../finite-brain/docs/specs/finitebrain-portability-spec.md#1-product-boundary)).

### Recommendation

Build a deep **Relationship Ledger module surfaced through the Organization
Brain**:

1. Keep the permitted FiniteBrain Folder authoritative for encrypted
   observations, explicit human decisions, relationship briefs, meeting
   preparation, and the durable history of stages, owners, and associations.
2. Write those facts through typed, signed Relationship Ledger commands rather
   than treating arbitrary Markdown edits as valid CRM mutations. Use opaque,
   stable ids; make ambiguity fail closed; and retain the evidence behind every
   projection.
3. Build a disposable, Folder-partitioned Relationship Index for tables,
   filters, pipeline views, uniqueness diagnostics, and agent queries. Rebuild
   it from Brain content just as FiniteBrain already rebuilds local search and
   graph state from decrypted Pages.
4. Give a small transactional Work Coordinator only the state that genuinely
   needs transactions: provider cursors, idempotency keys, due-work leases,
   attempts, and automation delivery. Its loss must not erase relationship
   knowledge, settled decisions, or pipeline history.
5. Do not copy upstream authentication, tenancy assumptions, provider-token
   storage, Vercel/eve deployment, telemetry destination, or purge policy.

This is still “CRM in the Brain” at the product and agent surface. It simply
avoids both bad extremes: asking unvalidated free-form Markdown to act like a
transactional pipeline database, and rebuilding a plaintext CRM as a second
knowledge authority. If the first slice needs an immediately transactional deal
pipeline, a separate CRM projection is a valid bridge, but it should be treated
as a migration stage with an explicit convergence plan rather than the
long-term source of truth.

## 1. What the product actually is

### Product thesis

The project calls itself an agent-first CRM. Its main inversion is that the
agent is a separately deployed, scheduled worker and the CRM is its structured
notebook, not a form database with a request/response chatbot attached. Agent
work continues after the browser closes, is selected from a work queue, has a
budget, and can schedule its own next look ([README lines
43-60](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L43-L60),
[agent dispatch rules lines
46-64](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L46-L64)).

Its originating plan is more specific: it was conceived as an “Agentic CRM
(HubSpot replacement)”—a lightweight, opinionated CRM for Comp AI's own sales
team ([build plan lines
1-10](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/crm-plan.md#L1-L10)).
That origin explains both the disciplined opinionated model and the missing
general-purpose tenancy, portability, and enterprise authorization features.

It is not attempting to be a Salesforce- or HubSpot-complete system. The core
workflow is:

- file companies, contacts, and deals;
- attach contacts to companies and deals;
- move deals through a fixed pipeline and record stage history;
- store calls, emails, meetings, notes, tasks, stage changes, and enrichment as
  activities;
- ingest forward-only mailbox/calendar activity and marketing-site form
  submissions;
- enrich or research records with a durable agent; and
- let a human accept or dismiss disputed facts.

The record types and their core fields are visible in the Prisma schema:
companies own contacts, deals, activities, email threads, calendar events, and
dynamic field values ([schema lines
280-339](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L280-L339));
contacts carry identity, employment, social, provenance, enrichment, activity,
and relationship links ([schema lines
352-404](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L352-L404));
deals require a company and owner and carry stage, amount, currency, close, and
participant state ([schema lines
916-957](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L916-L957)).

### Domain model

| Area | Upstream model | Important behavior |
| --- | --- | --- |
| Accounts | `Company` | Active domains are unique; archive is soft-delete; company may have a primary contact. |
| People | `Contact` | Active emails are unique; contact optionally belongs to a company; identity and research facts are separate. |
| Opportunities | `Deal` + `DealContact` | A deal belongs to one company and owner; people attach through a role-bearing join. |
| Timeline/work | `Activity` | Note, call, email, meeting, task, stage change, and enrichment share one record-linked timeline. |
| Communications | `EmailThread`, `EmailMessage`, `CalendarEvent`, `CalendarAttendee` | Mail and calendar are stored as normalized first-class records, not only flattened notes. |
| Custom schema | `FieldDefinition`, `FieldOption`, `FieldValue` | Company/contact/deal fields may be typed, ordered, archived, shown in sheets/tables/filters, and marked agent-fillable. |
| Views | `SavedView` | Filters are JSON; a view has an owner and may be shared. |
| Provenance | `RecordSource`, `ContactFact`, `CompanyEnrichment` | Record ingress and researched assertions are explicit. |
| Agent operations | `AgentTask`, `AgentConversation`, `AgentDefinition`, `AgentVersion`, `AgentTrigger`, `AgentRun`, `AgentAction`, `AgentAuditEvent` | Background work, chat, immutable deployment, execution, side effects, and audit are modeled independently. |

These are not only UI concepts. The schema gives activities explicit authors
and record links ([schema lines
1109-1144](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1109-L1144)),
dynamic fields typed storage and one value per field/record ([schema lines
992-1088](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L992-L1088)),
and saved views owned/shared state ([schema lines
1091-1107](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1091-L1107)).

The source enum distinguishes manual, import, email, calendar, and website
tracking ingress ([schema lines
199-205](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L199-L205)).
However, `IMPORT` is currently a model affordance, not evidence of a shipped
importer; see the integration gaps below.

## 2. Architecture and technology stack

The repository is a Bun/Turborepo TypeScript monorepo with three independently
deployed applications and shared packages:

- `apps/app`: Next.js 16 App Router, React 19, TanStack Query, tRPC client,
  Tailwind, and shared UI;
- `apps/api`: NestJS 11, `nestjs-trpc`, Better Auth, Swagger/OpenAPI, Prisma, and
  optional Redis-backed caching;
- `apps/agent`: eve durable-agent application with authored tools, skills,
  schedules, subagents, and a deny-all network sandbox;
- `packages/db`: Prisma schema, migrations, data helpers, and the Postgres client;
- `packages/auth`: Better Auth configuration, OAuth, SSO, API keys, singleton
  workspace, and connection permissions; and
- `packages/validation`, `packages/ui`, `packages/env`, and
  `packages/telemetry`: shared contracts and infrastructure.

The upstream README describes the same stack and process layout ([README lines
143-175](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L143-L175));
the pinned package manifests show Bun 1.3.12, TypeScript 5.9, Next 16.3,
NestJS 11, Better Auth 1.6, Prisma/Postgres, eve 0.29, and tRPC 11
([root package](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/package.json#L1-L47),
[web package](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/app/package.json#L14-L49),
[API package](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/package.json#L25-L62),
[agent package](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/package.json#L21-L35)).

The architectural seam is good: the API records that work is due by inserting an
`AgentTask`; it does not call an LLM or perform enrichment. The separate agent
leases the row and owns research, identity matching, evidence evaluation, and
writes ([API rules lines
20-32](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L20-L32)).
This is the most important upstream architecture to preserve conceptually.

The front end consumes a generated tRPC router type, while the API derives a
REST/OpenAPI bridge from the same procedures. `GET /openapi.json` therefore
describes the live router surface, and `/rest` translates wire format without
creating a second business-logic implementation ([API rules lines
129-164](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L129-L164),
[application bootstrap lines
50-114](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/create-app.ts#L50-L114)).

## 3. Storage, schema, and lifecycle

### Transactional storage

Postgres is the sole transactional store for application-owned CRM/API state;
eve separately supplies durable agent-session runtime, and Blob may hold image
bytes. Prisma migrations define 56 migration directories at the pinned snapshot,
and the local Compose file starts only Postgres 17 with a named volume
([migration tree](https://github.com/trycompai/crm/tree/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/migrations),
[Compose file](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docker-compose.yml#L1-L22)).
Redis is optional cache/counter coordination rather than a source of truth; the
project warns that per-instance counters can violate limits on a multi-instance
deployment when Redis is absent ([environment lines
156-164](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/.env.example#L156-L164)).
Images may be mirrored into Vercel Blob, while the relational rows keep their
URLs ([agent rules lines
25-44](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L25-L44)).

Several integrity choices are worth adapting:

- active company domains and contact emails use partial unique constraints, so
  an archived record does not block a new active record with the same identity
  ([schema lines
334-339](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L334-L339),
  [398-403](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L398-L403));
- mailbox thread/message identifiers and calendar event keys are unique, making
  overlapping forward-only sync safe to replay ([schema lines
1179-1227](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1179-L1227),
  [1230-1280](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1230-L1280));
- agent runs, actions, submissions, and events carry unique idempotency or
  sequence keys ([schema lines
779-884](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L779-L884)); and
- the agent queue makes work durable with due time, lease, attempts, budget,
  outcome, and subject ([schema lines
466-497](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L466-L497)).

### Archive, purge, and retention

Companies, contacts, and deals archive by setting `archivedAt`; purge is a
separate destructive path. A scheduled job permanently purges expired archived
records using a configurable retention period, defaulting to 180 days. A user
may also purge immediately from the archived view ([API rules lines
232-258](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L232-L258)).

This must not be copied blindly into Brain. A CRM purge may cascade through
activities, email, calendar links, facts, agent events, or Brain materializations
with different access and recovery contracts. In Finite, a structured record,
its encrypted evidence Page, and its external-provider source should have an
explicit tombstone and retention relationship; deletion of one must not
implicitly authorize deletion of the others.

### Recovery gap

The upstream repository documents migrations, drift detection, retention, and
record restoration from the soft-delete state, but it contains no database
backup, point-in-time recovery, off-host copy, export, RPO/RTO, or empty-target
restore contract. That is an inference from the documented deployment and setup
surface, not a claim about whatever the maintainers may operate privately. The
public deployment instructions name three apps plus Postgres and a cron, but no
backup plane ([README lines
340-356](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L340-L356)).

For Finite this is a hard blocker to treating an adopted CRM store as durable
user data. FiniteBrain itself states that ciphertext backup is insufficient
without a tested key-recovery path, and that recovery must reopen a Folder on an
empty replacement client ([FiniteBrain README](../../finite-brain/README.md#identity),
[system trust boundaries](../system-flow-and-trust-boundaries.md#security-posture-and-priority)).

## 4. Authentication, workspace model, and authorization

### Sign-in and identity

Better Auth backs sessions in Postgres. Email/password login is disabled;
Google and Microsoft social OAuth are configured in code, OIDC SSO may be added
as a database row, and the sign-in allow-list accepts controlled domains or
specific addresses. Session lifetime is seven days with a daily update and a
five-minute cookie cache; rate limiting is database-backed ([auth configuration
lines 72-120](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/auth.ts#L72-L120),
[sign-up guard lines
249-301](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/auth.ts#L249-L301)).

Google sign-in requests Gmail read-only and Calendar read-only; Microsoft
requests `Mail.Read` ([scope definitions lines
12-25](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/scopes.ts#L12-L25)).
This couples product login to mailbox access for social-login users, even though
SSO users may connect a mailbox later. That coupling is convenient for this
product but should not become a Finite account or Brain identity primitive.

### One singleton workspace, not multitenancy

The schema contains Better Auth `Organization`, `Member`, and `Invitation`
tables, but CRM records have no `organizationId`. Code always resolves the
literal singleton `WORKSPACE_ID`, signs a new user into that workspace, and
makes the first enrolled user owner ([organization helper lines
40-103](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/organization.ts#L40-L103),
[schema organization lines
1479-1539](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1479-L1539)).

Owner/admin/member roles do exist, but only a small set of workspace and
connection settings consult them. Normal company, contact, deal, activity, and
field routers use `AuthMiddleware`, whose complete check is “does this request
have a user?” ([company router lines
35-84](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/companies/companies.router.ts#L35-L84),
[contact router lines
33-73](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/contacts/contacts.router.ts#L33-L73),
[auth middleware lines
11-25](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/trpc/middlewares/auth.middleware.ts#L11-L25)).
Upstream's own policy summarizes the result: all signed-in internal users can
read and write every record ([security policy lines
16-27](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L16-L27)).

That is incompatible with FiniteBrain's Folder boundary. A CRM Folder may be
shared with a subset of an Organization Brain, and an agent has no authority
merely because it is an agent: current Brain membership, Folder access, and a
Folder Key Grant are still required ([FiniteBrain portability spec](../../finite-brain/docs/specs/finitebrain-portability-spec.md#2-trust-model)).

### API keys

The project supports `crm_` API keys in `x-api-key`, expiring between one and
365 days. The Better Auth plugin is configured to turn API keys into sessions
([API-key constants](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/api-keys.ts#L1-L9),
[auth plugin lines
234-246](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/auth/src/auth.ts#L234-L246)).
The CRM routers then use the same session-existence check. Although the API-key
table has a `permissions` string, no centralized CRM scope check is visible in
the request path at this snapshot, and per-key rate limiting is explicitly
disabled in the plugin configuration ([schema lines
1556-1584](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1556-L1584)).
Finite should therefore design explicit operation, record-kind, Folder, and
principal scopes rather than reuse the upstream API-key model.

## 5. CRM workflows and programmatic surface

### Human workflows

The active routers support:

- search/list/detail/create/update/archive/restore/purge for companies, contacts,
  and deals;
- bulk owner assignment, bulk stage or company changes, enrichment, archive,
  restore, and purge;
- deal participant attachment, detachment, and role;
- activity timelines, task creation/completion, and “my tasks”;
- custom-field creation, edit, ordering, archive, restore, delete, coverage, and
  agent backfill; and
- acceptance or dismissal of agent-proposed contact facts.

These are direct code paths in the company, contact, deal, activity, and field
routers ([companies router](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/companies/companies.router.ts#L42-L188),
[contacts router](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/contacts/contacts.router.ts#L40-L178),
[deals router](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/deals/deals.router.ts#L44-L203),
[activities router](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/activities/activities.router.ts#L35-L84),
[fields router](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/fields/fields.router.ts#L29-L126)).

The fixed deal pipeline has seven stages from demo booked through closed won or
lost ([schema lines
171-179](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L171-L179)).
Finite should not hard-code those exact stages into Brain; the useful concept is
an ordered, versioned pipeline vocabulary with explicit terminal states and
stage-change events.

### Import, export, API, and webhook state

The API story is stronger than the data-portability story:

- all tRPC procedures are exposed under a generated REST bridge and OpenAPI
  document;
- external clients may authenticate with a session cookie or API key;
- mailbox, currency, telemetry, tracking-retention, and archive jobs use
  separately guarded internal routes; and
- website tracking has an anonymous, bounded collector.

The bridge implementation and API-key security scheme are first-class in
`createApp` ([lines
38-114](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/create-app.ts#L38-L114)).
The mailbox cron refuses to run when `CRON_SECRET` is absent and uses a
constant-time bearer comparison ([sync controller lines
24-100](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/sync/sync.controller.ts#L24-L100)).

At the pinned snapshot, there is **no production CSV/HubSpot importer, bulk
exporter, generic webhook receiver, or webhook trigger authoring path**. Evidence:

- the active API module inventory contains no import/export/webhook module
  ([AppModule lines
41-82](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/app.module.ts#L41-L82));
- the schema's `IMPORT` source exists, but no router uses it as an import surface;
  and
- `AgentTriggerType` contains `WEBHOOK`, but the current validated deployable
  manifest accepts only manual, schedule, and CRM-event triggers
  ([schema lines
234-239](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L234-L239),
  [manifest lines
49-77](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/validation/src/agent-manifest.ts#L49-L77)).

That gap matters for replacing a conventional CRM. Finite should treat
round-trip export, deterministic re-import, external IDs, association import,
and provider cursor portability as first-slice requirements, not later polish.

## 6. Integrations and ingestion

### Mail and calendar

Gmail, Google Calendar, and Outlook share a normalization pipeline. The first
sync is forward-only: Gmail records a history cursor, Calendar starts at now,
and Outlook records the current time. The normalized writer is the only writer
of email threads, messages, and email activities; RFC Message-ID based threading
allows Gmail and Outlook copies of the same conversation to converge when the
provider supplies enough headers ([API rules lines
166-196](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L166-L196),
[environment lines
149-178](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/environment.md#L149-L178)).

The matching pipeline deliberately filters internal users, suppressed contacts
and domains, machine addresses, and automated local parts before creating a CRM
person or company ([API rules lines
198-215](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/api.md#L198-L215)).
This is a good reusable seam: every ingestion path should converge on one
identity/matching policy rather than independently deciding who a person is.

### Website tracking and form capture

The first-party tracker records page activity and form submissions for one
install's own sites; its principal purpose is to turn a qualified form submission
into a contact. It strips query strings, excludes sensitive form fields and card
shapes, stores no IP address, rate-limits in the database, preserves rejected
submissions with a reason, and bounds automated contact creation
([tracking design lines
1-10](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/tracking.md#L1-L10),
[76-118](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/tracking.md#L76-L118),
[143-174](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/tracking.md#L143-L174)).

This belongs in a CRM acquisition module, not in FiniteBrain core. A Brain may
receive the resulting source note and contact narrative; it should not become an
anonymous analytics collector.

### Slack and research vendors

Slack is a shared connection for agent actions and identity matching, with
separate bot and user grants because private-channel invitation requires a user
token. The code models a connection as a capability and an automation as an
agent; integrations become taggable builder resources rather than separate
automation editors ([connections lines
10-28](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/connections.md#L10-L28),
[89-121](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/connections.md#L89-L121)).

Optional agent providers are Context.dev for brand/LinkedIn data, Perplexity for
cited web research, GitHub for account matching/rate limit, Vercel Blob for
image durability, and Vercel AI Gateway for models. The agent is designed to run
without the optional research providers and uses CRM mailbox/calendar evidence
when they are absent ([environment lines
106-147](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/environment.md#L106-L147)).

For Finite, connection grants must stay product-scoped and principal-bound.
Provider tokens should not be copied into Brain Pages or the ordinary CRM
projection; the Brain should store only connection references, capability
summaries, and evidence produced under an audited grant.

## 7. AI and automation model

### Evidence ledger, not model self-confidence

This is the upstream project's most reusable idea. The agent does not submit a
self-assigned confidence score. It records typed observations such as a reply,
signature block, LinkedIn employer/name match, GitHub identity, meeting
attendance, cited web claim, or contradiction. Code assigns weights, requires a
primary source for the highest band, and caps contradicted evidence
([evidence model lines
3-78](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/agent/lib/evidence.ts#L3-L78),
[93-133](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/agent/lib/evidence.ts#L93-L133)).

`ContactFact` stores field, value, score band, evidence JSON, method, source URL,
session, disposition, human decider, and observation/supersession times
([schema lines
406-448](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L406-L448)).
The write path refuses blank/unsupported facts, previously dismissed values,
duplicates, and overwrites of human-authored fields; verified evidence or a
currently blank field may apply automatically, while a conflict becomes a human
proposal ([fact write path lines
64-179](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/agent/lib/facts.ts#L64-L179),
[183-237](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/agent/lib/facts.ts#L183-L237)).

For Brain, generalize this into:

`observation -> source/evidence -> proposed assertion -> human/agent decision -> current projection`

The current CRM row should be a projection, not the only surviving fact. Brain
Pages can retain the human-readable evidence and narrative; the structured layer
can retain the assertion ledger and enforce deterministic resolution.

### Durable tasks and scheduling

Work is stored, not implied by cron. `AgentTask` rows carry reason, priority,
budget, attempts, due time, lease, session, and outcome. Dispatch leases due rows
with `FOR UPDATE SKIP LOCKED`, separates direct non-model work from research
sessions, retries abandoned leases, and exposes stale/unlinked work in health
state ([agent rules lines
46-92](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L46-L92),
[budget and scheduling lines
222-232](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L222-L232)).

This maps well to Finite: “revisit Acme in 14 days because the contract renewal
will be known” should be durable CRM state with a reason, not prose hidden in a
Brain log or a cron expression.

### Agent builder, permissions, and execution

Custom agents are persisted as definitions with immutable versions, typed
manifests, triggers, runs, actions, and audit events. A version chooses selected
records or the whole workspace, declares integrations and allowed action types,
and is deployed only after a human review; runtime tools revalidate the manifest
instead of trusting its prose ([agent rules lines
304-372](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L304-L372),
[manifest lines
19-108](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/validation/src/agent-manifest.ts#L19-L108)).

The active CRM event vocabulary is still small—company/contact/deal creation and
deal stage/open/close transitions—but it is centralized and typed
([event catalog](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/src/crm-events.ts#L1-L47)).
That is a good extension seam. Finite should expand a similarly versioned event
catalog only as business workflows require it, with Folder/principal authority
attached to each event and action.

### Agent isolation

The general research agent receives a filesystem sandbox with deny-all network
egress on Vercel, Docker, and microsandbox backends ([sandbox
definition](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/agent/sandbox/sandbox.ts#L1-L9)).
The sandbox is not given `DATABASE_URL`; CRM access goes through authored tools,
while web fetch/search happens outside the sandbox in controlled runtime paths
([agent rules lines
283-302](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/agent.md#L283-L302)).

The design is sound, but the implementation depends on eve and Vercel's runtime
model. Finite should adapt the policy—no ambient database, network, or Folder
authority—not import the agent runtime.

## 8. Deployment and operations

### Deployment shape

Production is designed around three deployments plus Postgres: Next.js web,
NestJS API, and eve agent. They share `DATABASE_URL` and auth secrets; a scheduler
calls mailbox sync, while the agent has its own schedule ([README lines
340-356](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L340-L356)).
The repository is Vercel-oriented: serverless API build, Vercel cron config,
Vercel AI Gateway/OIDC, Vercel Blob, and Vercel Sandbox are first-class. The
public Compose file covers local Postgres only, not a complete production stack.

The API cron schedule runs mailbox sync every five minutes, rates daily,
telemetry daily, tracking retention daily, and archive pruning daily
([Vercel schedule](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/vercel.json#L1-L24)).
The project notes that minute-level Vercel cron requires a paid plan and may
silently degrade on Hobby ([environment lines
171-178](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/environment.md#L171-L178)).

### Operational warnings in first-party docs

The maintainers document several real deployment hazards:

- `vercel env pull` can place production credentials in `.env.local`; the docs
  say a laptop applied eleven migrations to production on 2026-08-01
  ([setup lines
87-99](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/setup.md#L87-L99));
- preview deployments currently share the production database, so schema-change
  previews either fail on missing tables or, before a guard was added, could
  migrate production ahead of live code ([setup lines
101-114](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/setup.md#L101-L114)); and
- recorded migrations and actual schema once diverged after a database push,
  leaving a production table without a required column while deploys reported no
  pending migrations ([setup lines
116-136](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/setup.md#L116-L136)).

These candid notes are a positive maintenance signal, but the underlying
deployment model is not safe enough to adopt as Finite infrastructure. Finite's
CRM domain module needs isolated preview/test databases, CI-built artifacts,
digest-pinned deployment, explicit migration gates, platform status probes, and
the repository's existing backup/rollback discipline.

## 9. Maturity and maintenance signals

As of 2026-08-25, GitHub reports the repository was created on 2026-07-31. It is
therefore a very young project, despite substantial activity ([GitHub repository
metadata](https://api.github.com/repos/trycompai/crm)). The pinned history has
225 commits and is concentrated in two human contributors (132 and 31 commits),
plus 57 automation commits; the public [contributors
endpoint](https://api.github.com/repos/trycompai/crm/contributors?per_page=100&anon=1)
is the dated primary source for that concentration.

Release automation cut 22 visible releases from `v1.0.0` on 2026-08-06 through
`v1.15.3` on 2026-08-21, often several in one day
([GitHub releases](https://github.com/trycompai/crm/releases),
[changelog lines
1-39](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/CHANGELOG.md#L1-L39)).
That is evidence of active work and rapid correction, not of long-term stability
or production soak.

Positive signals:

- CI runs type checking, Biome, a custom static-analysis pass, and tests against
  real Postgres on pushes and pull requests ([CI workflow lines
1-69](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/.github/workflows/ci.yml#L1-L69)).
- The repository has extensive integration tests around mailbox sync, deletion,
  agent lifecycle, durable task execution, tracking, authorization, and schema
  behavior ([API tests](https://github.com/trycompai/crm/tree/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/test),
  [agent tests](https://github.com/trycompai/crm/tree/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/agent/test),
  [database tests](https://github.com/trycompai/crm/tree/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/test)).
- First-party docs record failure modes and invariants close to the code rather
  than presenting only a product README.
- Releases are tagged and the default `release` branch is intended to remain on
  the last shipped tag ([README lines
209-212](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L209-L212)).

Caution signals:

- a fresh-clone `bun run dev` race remains open and can terminate the whole dev
  stack on first startup ([issue #99](https://github.com/trycompai/crm/issues/99));
- the documented private vulnerability-reporting channel was reported as not
  enabled and that issue remains open ([issue #104](https://github.com/trycompai/crm/issues/104));
- the security policy says only `main` is supported, while the README says
  operators should run the tagged `release` branch—a small but meaningful support
  contract inconsistency ([security policy lines
62-70](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L62-L70),
  [README lines
209-212](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L209-L212)); and
- an open request for server-enforced owner-based access correctly notes that a
  fix would need to cover lists, detail, mutation, search, aggregates, history,
  bulk operations, and agent tools—not only UI filtering
  ([issue #182](https://github.com/trycompai/crm/issues/182)).

Conclusion: treat it as a fast-moving, thoughtfully engineered young codebase
and design source, not as a drop-in mature system of record.

## 10. License and reuse constraints

The repository is MIT-licensed. Finite may use, copy, modify, merge, publish,
distribute, sublicense, and sell copies, provided the copyright and permission
notice remain in copies or substantial portions. The software is provided
without warranty ([LICENSE lines
1-20](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/LICENSE#L1-L20)).

Practical constraints:

1. Preserve the Comp AI copyright and MIT notice in any copied or substantially
   derived source. Record provenance at file or component granularity.
2. MIT on this repository does not grant rights to third-party trademarks,
   hosted services, vendor data, or API terms. The dependency and service stack
   includes eve, Context.dev, Perplexity, GitHub, Google, Microsoft, Slack,
   PostHog, Vercel AI Gateway, Vercel Blob, and Vercel Sandbox; each requires an
   independent license/terms/privacy review before shipping a derived product.
3. Avoid copying brand assets, marketing copy, default upstream telemetry
   destination, or provider-specific onboarding merely because the source code
   is MIT.
4. Prefer adapting concepts and small deep modules over forking the entire
   application. A fork would inherit the fast release cadence and force Finite to
   track security and schema changes across a large TypeScript/Vercel surface.

This is an engineering reading of the license text, not legal advice.

## 11. Security, privacy, and recovery assessment

### Strong upstream controls worth preserving

- Empty `ALLOWED_SIGN_IN` fails closed, and the security guide warns against
  allowing a whole consumer-mail domain ([security policy lines
20-27](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L20-L27)).
- CRM routes use server-side session middleware; cron routes require a secret and
  fail closed; Helmet is enabled; DTO validation strips and rejects unknown
  fields ([application bootstrap lines
21-35](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/apps/api/src/create-app.ts#L21-L35)).
- External image/site fetches reject internal, link-local, loopback, multicast,
  and invalid hosts on every redirect, bounding an important SSRF path
  ([safe fetch lines
8-48](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/src/safe-fetch.ts#L8-L48),
  [88-171](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/src/safe-fetch.ts#L88-L171)).
- Google/Microsoft mailbox scopes are read-only, the agent sandbox has deny-all
  egress, and the shell receives no database credential.
- Usage telemetry is server-side, property-allowlisted, disableable, and
  documented to exclude record values, identities, prompts, mail, amounts,
  secrets, and IP addresses ([telemetry lines
16-43](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/telemetry.md#L16-L43),
  [287-304](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/telemetry.md#L287-L304)).
- Agent side effects are typed, pre-ledgered, idempotent, and pinned to a
  human-approved immutable version rather than granted ad hoc by a prompt.

### Material risks for a Finite adoption

1. **No CRM data authorization boundary inside the workspace.** An owner field
   is attribution, not access control. Every signed-in member—and an API key that
   becomes that member's session—can reach all CRM records.
2. **Plaintext operator visibility.** Database, environment, and logs are inside
   the operator trust boundary. This is an explicit upstream assumption, not a
   vulnerability ([security policy lines
29-30](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L29-L30)).
3. **Large sensitive database blast radius.** The same database holds CRM
   records, full email bodies, calendar material, OAuth access/refresh/id tokens,
   Slack user grants, third-party research keys, agent transcripts/attachments,
   action history, website events, and form fields
   ([account token schema lines
129-147](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L129-L147),
   [mail schema lines
1204-1227](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L1204-L1227),
   [agent attachment schema lines
604-651](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/packages/db/prisma/schema.prisma#L604-L651)).
4. **Mailbox privacy and controller obligations.** The agent reads bodies,
   attendees, and signature blocks belonging to people who never signed up.
   Upstream places data-controller responsibility on the deployer
   ([security policy lines
32-42](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L32-L42)).
5. **Third-party egress.** Optional research calls may carry a name, email domain,
   employer, URL, or derived question. Finite's private-inference and retrieval
   claims require an explicit vendor-by-vendor data-flow decision rather than
   importing these defaults. Separately, agent conversations reach the selected
   model through Vercel AI Gateway; it is therefore reasonable to infer that
   prompts are processed outside the CRM deployment even when research keys are
   absent ([stack lines
   149-157](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/README.md#L149-L157)).
   The repository does not establish provider retention, residency, DPA, or
   training terms; those must be resolved for Finite's chosen inference lane.
6. **The security summary is incomplete relative to current telemetry.** It says
   that with optional vendor keys unset nothing leaves except Google APIs, while
   the current install telemetry is on by default and sends to a fixed Comp AI
   PostHog host unless disabled ([security policy lines
   39-42](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/SECURITY.md#L39-L42),
   [telemetry lines
   1-14](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/telemetry.md#L1-L14)).
   The telemetry specification is detailed and its payload is deliberately
   content-free, but deployment privacy review must use the current code and
   telemetry spec rather than that sentence in `SECURITY.md`.
7. **No public recoverability contract.** Database persistence and archive
   restore are not backup/restore evidence. CRM replacement requires tested
   empty-target recovery and portable export before cutover.
8. **Destructive retention spans linked evidence.** Automated 180-day purge is
   reasonable for a standalone CRM but cannot silently govern encrypted Brain
   Pages, source notes, or legal/contract knowledge.
9. **Connection secrets are workspace-global in places.** A Slack reconnection
   can replace the shared workspace connection, affecting every agent
   ([connections lines
30-58](https://github.com/trycompai/crm/blob/6d4793dd6d7aeea91aa6a034e00b17d7408a2d08/docs/connections.md#L30-L58)).

## 12. Extension seams: what to adapt and what to leave behind

### Adapt or reimplement

| Concept | Why it fits Brain | Finite-specific change |
| --- | --- | --- |
| Company/contact/deal/activity graph | Supplies the relationship and pipeline vocabulary missing from a general wiki. | Use Organization/Folder-scoped stable IDs and configurable pipeline definitions. |
| Evidence/assertion ledger | Makes agent-authored knowledge reviewable, sourced, reversible, and non-destructive. | Evidence points to encrypted Brain source notes or authorized external objects; resolution is Folder/principal aware. |
| Durable `AgentTask` lease model | Correct home for follow-up, research, refresh, and failed-work visibility. | Run in Finite's agent/runtime boundary with status probes and recovery; do not depend on eve/Vercel. |
| Central ingestion/matching policy | Prevents Gmail, Calendar, forms, imports, and agents from creating conflicting identities. | Make ambiguity fail closed; preserve provider IDs and merge history; add reversible merge/split. |
| Dynamic field definitions | Lets each org evolve its CRM vocabulary without schema churn. | Version definitions and bind visibility/editability to Folder access and principal capability. |
| Event/trigger/action manifest | Clear seam for automations and human approval. | Scope every read/action to an Agent Principal plus Brain Folder grants; use Finite's audit and runtime. |
| Idempotency and immutable agent versions | Essential for retries and trustworthy side effects. | Include source Brain revision, skill revision, principal, and connection grant in the execution record. |
| Generated API contract | Gives agents, UI, importers, and integrations one domain surface. | Use a Finite-owned protocol/API; separate read models from mutation capabilities. |
| Archive-before-purge UX | Good human recovery affordance. | Define linked Brain tombstones, retention, export, and backup semantics first. |

### Keep separate or redesign

| Upstream component | Why not transplant it |
| --- | --- |
| Better Auth singleton workspace | Conflicts with Finite account auth, Nostr Member Identity, Agent Principal identity, and Folder grants. |
| Record-wide “any signed-in user” access | Breaks Organization Brain compartmentalization and makes cross-Folder leakage likely. |
| Next/Nest/tRPC UI stack as a whole | Large maintenance fork with little leverage if Finite already owns dashboard, Product Client, runtime, and identity surfaces. |
| eve/Vercel agent runtime | Finite already has an Agent Runtime and managed skill/revision model; duplicating durable-agent authorities is worse than adapting the task/action contracts. |
| OAuth tokens and mailbox bodies in the general CRM database | Expands plaintext blast radius; connection custody and normalized knowledge should have separate stores and access policies. |
| Website tracker in Brain core | Acquisition telemetry is a CRM connector, not encrypted knowledge infrastructure. |
| Upstream PostHog destination | Forks should opt in deliberately and route only approved product telemetry. |
| Automatic CRM purge governing Brain content | CRM lifecycle and knowledge retention are related but distinct user-data authorities. |
| Vendor-specific Context/Perplexity defaults | Finite needs an explicit private-inference/retrieval and data-egress policy. |

## 13. Proposed Finite shape

### Authoritative seams

| Module | Owns | Does not own |
| --- | --- | --- |
| Relationship Ledger | Typed observations, assertions, human decisions, stable opaque entity ids, association/stage/owner history, and deterministic reduction rules | Provider credentials, due-work leases, or derived query state |
| FiniteBrain Folder adapter | Encrypted Ledger records, source notes, relationship briefs, meeting prep, correspondence summaries, human narrative, and Folder sharing | OAuth tokens, provider cursors, or job leases |
| Relationship Index | Rebuildable entity projections, uniqueness diagnostics, tables, filters, pipeline views, and agent queries over currently readable Folders | Canonical facts, backup, authorization, or irreversible mutation authority |
| Work Coordinator | Provider cursors, idempotency keys, due-work leases, attempts, delivery state, and operational health | The only copy of relationship facts, decisions, pipeline history, or source evidence |
| Connection adapters | OAuth grants/tokens, provider scopes, revocation, ingestion health, and normalized observations | General Brain content or unrestricted Ledger mutation |
| Agent Runtime | Research, synthesis, proposal generation, and approved actions under principal/Folder capability | Ambient database, Brain, mailbox, or network authority |

### Materialization contract

A typed Relationship Ledger command should produce an append-only observation,
decision, or state-transition record in the permitted Brain Folder. A trusted
client then reduces those records into human-readable Pages and a disposable
local Relationship Index, for example:

```text
CRM/
  ledger/observations/<opaque-event-id>.md
  ledger/decisions/<opaque-event-id>.md
  Accounts/<opaque-account-id>.md
  People/<opaque-person-id>.md
  Opportunities/<opaque-opportunity-id>.md
  raw/google-mail/thread-<stable-id>.md
  raw/calendar/event-<stable-id>.md
  _index.md
  log.md
```

The filenames are illustrative, not a protocol proposal. FiniteBrain currently
treats each Folder as an LLM wiki scope and requires source notes to stay with
their Folder ([FiniteBrain README](../../finite-brain/README.md#agent-rules)).
Ledger records should carry their Brain Folder/Object identity and revision,
source identity or provider revision, acting principal, schema version, and an
idempotency key. A Brain edit that represents a relationship mutation must go
through a typed command, validation, and conflict check; arbitrary Markdown edit
must not silently become a deal-stage or ownership mutation. Compaction may
produce new projections, but it must not destroy the observations and decisions
needed to replay current state.

### First useful slice

1. **Read-only relationship registry.** Import a small synthetic set of
   companies, contacts, deals, and activities with stable opaque ids. Generate
   Brain Pages and source notes, then prove the Relationship Index can be
   deleted and rebuilt exactly from the Brain Folder.
2. **Assertion ledger.** Add observations, evidence, proposals, explicit human
   decisions, and replayable current projections. Start with account/contact
   briefs and meeting preparation, where Brain adds the most value.
3. **One inbound connector.** Normalize one provider through a single identity
   policy; preserve provider source ids, consent/revocation state, and a minimal
   source note rather than copying every mailbox body by default.
4. **Durable follow-up.** Add reasoned due intents to the Ledger and lease them
   through the Work Coordinator with bounded attempts, budgets, and visible
   failure state. No external side effects yet.
5. **Approved actions.** Add a small typed event/action catalog with immutable
   versions, explicit Folder/principal scope, idempotency, and audit.
6. **Replacement gate.** Prove concurrent writers, revocation-driven index
   deletion, portable export, empty-target server restore, independent Folder
   key recovery on a replacement client, schema migration, and exact projection
   replay before retiring an existing CRM.

Do not start with a pixel, an agent builder, full mailbox ingestion, or a clone of
the upstream UI. The decisive experiment is narrower: can an agent and a human
maintain one account's structured lifecycle and encrypted evidence without
duplicated authority or irreversible drift?

## 14. Decision summary

| Option | Result |
| --- | --- |
| Fork and run `trycompai/crm` as Finite's CRM | Fastest demo, but imports a second auth/agent/deployment stack, plaintext operator access, unscoped record APIs, and no public recovery contract. Not recommended. |
| Treat upstream CRM as a companion service and link summaries into Brain | Viable short-term experiment. Keeps transactional behavior, but still requires a connector, Folder-aware authorization, token isolation, export/restore, and privacy review. |
| Implement a Finite-owned Relationship Ledger plus a rebuildable local index and narrow Work Coordinator | Best long-term fit. Reuse the domain/evidence/task/action concepts, keep durable relationship knowledge and decisions encrypted in Brain, and isolate only the genuinely transactional operational state. Recommended. |
| Store all CRM state as arbitrary Brain Markdown only | Attractive simplicity, but weak for validation, uniqueness diagnostics, concurrent semantic updates, cursors, idempotency, pipeline transitions, and durable job leasing. Brain can be the authority only when typed Ledger commands and deterministic replay sit between ordinary files and CRM semantics. |

The key insight to take from upstream is not its TypeScript stack. It is the
separation of **observations, evidence, decisions, current projections, durable
work, and approved actions**. FiniteBrain can make those artifacts more private,
portable, and agent-native if the Relationship Ledger is the deep module:
callers learn a small typed interface while validation, evidence resolution,
replay, conflict handling, and projection rebuilding stay inside its
implementation.

## Concise transplant / avoid list

**Transplant:** the company/contact/deal/activity graph; evidence and assertion
ledger; durable leased task queue; centralized identity-matching policy; dynamic
fields; typed events/triggers/actions; immutable agent versions; idempotency;
archive-before-purge UX; and one generated API contract.

**Avoid:** a wholesale application fork; singleton Better Auth workspace;
workspace-wide record access and session-equivalent API keys; co-locating OAuth
tokens, mailbox bodies, CRM rows, and agent transcripts in one plaintext store;
the eve/Vercel runtime as a new authority; upstream telemetry and website
tracking in Brain core; and any purge or cutover before portable export plus a
tested empty-target restore.
