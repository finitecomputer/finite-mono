# Tooling decision for Finite's Commercial Relationship Register

Date: 2026-08-25

This note evaluates whether Finite should build the agreed Commercial
Relationship Register and Customer Commercial Subledger inside FiniteBrain,
adopt an open-source CRM, or begin in a spreadsheet-like database. It uses the
agreed domain model in
[`docs/commercial-relationships/CONTEXT.md`](../commercial-relationships/CONTEXT.md)
and the source-level review of
[`trycompai/crm`](2026-08-25-trycompai-crm-finitebrain-fit.md).

The comparison uses current first-party documentation, source repositories,
licenses, and a read-only operating probe of Finite's Organization Brain. It
distinguishes features a tool has natively from structures Finite could build
with custom objects or tables.

## Decision

**Do not abandon Brain dogfooding. Build one bounded, Brain-native
record-per-Page pilot.** The current Brain is not already a CRM or relational
database, but its encrypted Page storage, ordinary Markdown Working Tree,
client-derived indexes, sync, search, and agent access are credible foundations
for this small, low-write-volume register.

This recommendation is conditional in an important way: Finite must build a
small structured application surface, not place one giant Markdown table in a
Brain and call it a CRM. Each record should be its own typed Page, mutations
should go through validating commands or forms, and tables and totals should be
rebuildable client-side projections. The pilot should prove the difficult
sponsorship and payment examples before the system becomes authoritative.

If Finite does not want to allocate product engineering to that pilot, use
**Grist Community**, not Google Sheets, as the structured source of truth and
keep Brain as the narrative and agent layer. Grist is the lightest credible
self-hosted relational spreadsheet, and it can represent the agreed schema
without lookup-formula contortions.

If Finite decides it wants a conventional CRM application shell more than it
wants Brain dogfooding, **Twenty is the strongest existing product to
customize**. It is materially more suitable than `trycompai/crm`: its custom
objects and relations can model the whole register, while its standard Company,
Person, Opportunity, note, and activity surfaces provide the familiar CRM
parts. It would still require a custom Finite application for the entire
commercial subledger.

Do not adopt ERPNext unless Finite deliberately chooses to operate a real
accounting system. Do not adopt `trycompai/crm` for this model.

## Why a Brain-native pilot is credible

FiniteBrain already supplies the parts that are strategically distinctive:

- A Brain is an encrypted, Folder-scoped knowledge system, and agents edit a
  durable local projection as ordinary Markdown before syncing encrypted
  changes ([FiniteBrain README](../../finite-brain/README.md)).
- The Product Client opens Folder Keys locally, materializes and edits Pages,
  and builds search and graph state without making the server a plaintext
  knowledge authority ([FiniteBrain context](../../finite-brain/CONTEXT.md#product-client)).
- The current search index is explicitly derived from readable Markdown and
  points back to original Pages rather than becoming a second authority
  ([Hybrid Wiki Search](../../finite-brain/CONTEXT.md#hybrid-wiki-search)).
- The portable Working Tree already reserves `datasets/` for manifests,
  schemas, samples, and query recipes. It does not claim that this convention
  is a database, but it gives structured data a natural home
  ([Portable v1, datasets](../../finite-brain/docs/specs/finitebrain-portability-spec.md#133-llm-wiki-and-agent-layer)).
- A read-only probe on 2026-08-25 successfully listed Finite's Organization
  Brain, opened it into a temporary Working Tree, and materialized its ordinary
  Markdown Pages. Existing relationship and meeting material therefore already
  has a real Brain location; the proposed integration is not hypothetical.

The current limitations are equally concrete:

- Portable v1 persists encrypted Folder Objects whose supported readable
  content is a Markdown Page. It has no general relational-record object type
  ([Folder Object plaintext](../../finite-brain/docs/specs/finitebrain-portability-spec.md#45-folder-object-plaintext)).
- The Product Client and CLI expose Pages, search, graph, access, and sync, but
  no CRM grid, custom-object builder, foreign-key editor, or accounting-entry
  workflow. `datasets/` is a directory convention, not a query engine.
- Sync uses optimistic concurrency rather than a CRDT. Concurrent edits to the
  same object can conflict
  ([Portable v1 sync](../../finite-brain/docs/specs/finitebrain-portability-spec.md#9-brain-record-index-and-sync)).
- Folder access is binary rather than viewer/editor-granular. This is acceptable
  for the stated trusted private Org Brain, but it should not be generalized
  into a future customer-facing authorization claim
  ([Portable v1 access](../../finite-brain/docs/specs/finitebrain-portability-spec.md#61-folder-access)).
- FiniteBrain's own README warns that a server ciphertext backup is not a usable
  recovery path without a tested Recovery Principal that can reopen Folder Keys
  on an empty replacement client. That restore must be proven before the Brain
  becomes the sole durable commercial authority
  ([recovery warning](../../finite-brain/README.md#identity)).

These limitations do not disqualify the use case. They determine the module
shape.

## Recommended Brain module shape

Use one dedicated Commercial Relationships Folder. Keep the existing Portable
v1 Page type and place a small versioned record envelope in Markdown
frontmatter. Do not add a new server-visible plaintext object type.

### Canonical records

Use one Page per independently edited record:

- Commercial Organization;
- Contact;
- Commercial Account;
- Opportunity;
- Commercial Arrangement;
- Purchased Package, with its Offering Lines kept together when they share one
  price;
- Charge;
- Incoming Payment, including its allocations;
- refund or reversal;
- Supplier Cost;
- Customer External Spend;
- Contribution; and
- a simple Organization Wants Page.

Each record needs a stable opaque ID, record kind, schema version, timestamps,
and explicit IDs for relationships. Human-readable names remain content, not
identifiers. A financial event should be append-oriented: preserve the original
event and use a correction or reversal rather than silently rewriting history.

Keeping allocations inside the Incoming Payment record makes one payment and
its allocation set one optimistic-concurrency unit. Keeping an actual package
and its included lines together avoids inventing separate prices and prevents a
half-updated package. One-record-per-Page avoids the high-conflict behavior of a
single shared table Page.

### Derived client index

Build a disposable local relational index over decrypted records. It should
provide:

- grid and record-detail views;
- organization and Contact Rolodex views;
- payer versus beneficiary views;
- current and historical sponsorship;
- packages and fulfillment paths;
- lifetime consideration received by payer;
- services owned by beneficiary;
- Charge and payment-allocation status;
- Supplier Cost, pass-through recovery, and Customer External Spend views; and
- visible reconciliation warnings.

The index and calculated totals are never edited. Delete it and rebuild it from
Pages to prove that the authority boundary is real. This follows the existing
Brain pattern in which decrypted search and graph state are client-derived
([ADR 0005](../../finite-brain/docs/adr/0005-derive-graph-and-replay-from-client-decrypted-indexes.md)).

### One mutation surface for people and agents

The human forms and agent tools should call the same typed commands. In the
pilot, an agent may append sourced Wants or meeting notes. It should not change
an Arrangement, price, payer, Charge, or Incoming Payment without a human
confirmation.

Arbitrary Markdown remains readable and portable, but a malformed structured
Page must appear as a reconciliation warning rather than silently entering
totals. This matches the user's preference for a red warning over a complex
truth-resolution system.

Stripe remains authoritative for Stripe financial events, and Core remains
authoritative for Account Auth-linked admission, Projects, and runtime state.
The module stores their stable IDs and derived refreshes; humans do not edit
copied subscription or runtime values. Manual and legacy commercial facts are
Brain-owned when no upstream system represents them.

### Pilot acceptance cases

The pilot should not become an open-ended CRM-platform project. It passes only
if one small fixture can answer all of these correctly:

1. A sponsor pays for services owned by several beneficiary Accounts.
2. A beneficiary later becomes its own payer without losing history.
3. One purchased Package has several included Offering Lines and one price.
4. A first-class and a legacy agent use the same Offering but distinct
   Fulfillment Paths.
5. Finite pays a supplier, charges the customer for recovery, and later receives
   payment without netting away either leg.
6. A customer pays the supplier directly, creating Customer External Spend but
   no Finite revenue or Supplier Cost.
7. A Bitcoin payment preserves native units, transaction reference, and
   receipt-time reporting value.
8. An in-kind Contribution stays separate from cash received.
9. An incomplete historical event remains visible with a warning and does not
   fabricate zero.
10. The full index is deleted and rebuilt, and a backed-up Brain is reopened on
    an empty replacement client.

If this vertical slice feels awkward despite working, stop and move the same
schema to Grist. The record IDs and explicit relationships make that exit
tractable.

## Decision matrix

Ratings describe fit for Finite's agreed model, not general product quality.

| Candidate | What fits natively | What Finite must build | License and paid boundary | Operations | Brain/agent seam | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| **Brain-native module** | Encrypted Pages, Working Tree, sync, search, graph, knowledge links, existing Org Brain | Typed record format, validating commands/forms, relational index, grid/detail views, imports and rollups | First-party; no additional vendor license | Reuses Brain, but adds client code and requires tested empty-target recovery | Best: narrative, structured facts, meetings, and proposals share one authority | **Recommended bounded pilot** |
| **`trycompai/crm`** | Company, Contact, Deal, activities, evidence, agent queue | Almost every subledger entity and all sponsorship/package/payment semantics; dynamic fields do not create the required object graph | MIT | Three applications plus Postgres; vendor-oriented Vercel/eve stack; no public recovery runbook | Strong autonomous-agent design, weak scoped service identity | **Design donor only** |
| **Twenty** | Company, Person, Opportunity, notes/tasks, custom objects/fields/relations, REST/GraphQL, webhooks | Accounts, Arrangements, Packages/Lines, sponsorship, Charges, Payments, allocations, costs, external spend, BTC fields | AGPL core; free self-host includes Pro features; SSO, row-level permissions, and AI usage data require paid Organization | Official Compose, Postgres/Redis/application services, and documented backup/restore | Strong APIs; MCP and native agents/skills, though agent app extensions are alpha | **Best CRM application shell** |
| **Grist Community** | Typed relational tables, references/reference lists, formulas, grids/forms, access rules, REST API, webhooks | The entire commercial vocabulary, validation rules, and Brain connector | Apache-2.0 Community; full edition adds proprietary automation, notification, administration, OAuth/MCP, and newer AI features | One-container start is easy; persistent documents and optional snapshot storage still require backup operation | Good API/webhook seam; narrative remains in Brain | **Best immediate structured fallback** |
| **Baserow OSE** | Linked tables, formulas, lookups/rollups, REST API, webhooks, grid UI | The full CRM and subledger model and transaction checks | MIT open-source edition; premium/enterprise directories are separately licensed; granular RBAC and audit are paid | More application/worker infrastructure than Grist; Postgres-backed | Good API seam; no special Brain advantage | **Credible, but behind Grist** |
| **Frappe CRM / Framework** | Leads, Organizations, Contacts, Deals and products; DocTypes, child tables, roles, REST API, webhooks | Custom DocTypes for the subledger and a polished CRM UI for them | CRM is AGPL-3.0; Frappe Framework is MIT | Supported self-hosting, but a broader Frappe stack and upgrade discipline | API-capable; materially weaker native AI story than Twenty | **Framework option, not first choice** |
| **ERPNext** | Sales and purchase invoices, payments, partial/unallocated allocations, supplier payments, multi-currency, GL and P&L | Sponsorship/beneficiary ownership, customer-paid external spend, Bitcoin wallet/valuation detail | GPL-3.0 | Full ERP and accounting operation | API-capable but not Brain-native | **Only if adopting real accounting** |
| **Google Sheets** | Immediate collaborative grid and formulas | Every relationship, validation rule, stable ID convention, audit behavior, and agent boundary | Proprietary hosted service | No Finite deployment, but ongoing formula and schema fragility | API possible; narrative and structure remain split | **Throwaway sketch only** |

## Existing tools in more detail

### `trycompai/crm`

The earlier source review remains decisive. Upstream is a young, opinionated
HubSpot replacement with Company, Contact, Deal, Activity, evidence, and agent
execution concepts. Its dynamic fields attach to the standard CRM objects; they
do not supply the first-class Accounts, Arrangements, Packages, financial
events, allocations, and supplier-cost graph required here. Adopting it would
mean changing its core schema while also operating its three-process stack.

Its evidence ledger, durable work queue, idempotency, and explicit agent
capabilities remain excellent design references. Its MIT license is permissive,
but permissive licensing does not cure the domain and operational mismatch. See
the [pinned source review](2026-08-25-trycompai-crm-finitebrain-fit.md) for the
full evidence.

### Twenty

Twenty is the strongest answer to “is there a better open-source CRM?” It has
standard CRM records and can define new objects, add fields to standard objects,
and create bidirectional relationships with stable identifiers
([Twenty app data model](https://docs.twenty.com/developers/extend/apps/data/overview)).
Its REST and GraphQL surfaces adapt to the custom model, and its Metadata API
can change that model programmatically
([Twenty APIs](https://docs.twenty.com/developers/extend/api)). Webhooks cover
custom objects as well as standard objects
([Twenty webhooks](https://docs.twenty.com/developers/extend/webhooks)).

That means Finite could implement every agreed entity as a custom Twenty object
and model payment allocation as a join object. This is customization, not
native accounting. Twenty does not intrinsically know why payer and beneficiary
differ, what a pass-through recovery means, or whether Bitcoin valuation is
complete.

Twenty's official self-host path includes backup and restore guidance
([Docker Compose](https://docs.twenty.com/developers/self-host/capabilities/docker-compose)).
The core repository uses AGPL-3.0
([license](https://github.com/twentyhq/twenty/blob/main/LICENSE)); free
self-hosting includes the Pro feature set, while SSO, row-level permissions, and
AI usage data are paid Organization features
([plans](https://docs.twenty.com/user-guide/billing/capabilities/pricing-plans)).
Those paid permission features are not required for the stated trusted internal
team, but the license and private-extension obligations should be reviewed
before embedding a Finite-branded proprietary application.

Twenty also has the cleanest existing agent seam: APIs, scoped API-key roles,
MCP, and app-defined skills and agents. The app-defined agent feature is
explicitly alpha, and self-hosted code execution is disabled by default in
production unless an operator chooses a safe driver
([skills and agents](https://docs.twenty.com/developers/extend/apps/logic/skills-and-agents),
[self-host setup](https://docs.twenty.com/developers/self-host/capabilities/setup)).

Choose Twenty if the team wants a polished standalone CRM and accepts the
Brain becoming its narrative client. Do not choose it merely to avoid writing a
small Brain grid: Finite would still write the commercial application and would
gain another authority, deployment, backup path, and integration boundary.

### Grist Community

Grist is the strongest answer to “is there something better than a Google
Sheet?” Its Community edition is an Apache-2.0 self-hostable relational
spreadsheet with typed columns, references, Python formulas, forms, and
fine-grained access rules
([Grist core and license](https://github.com/gristlabs/grist-core),
[references](https://support.getgrist.com/references-lookups/),
[access rules](https://support.getgrist.com/access-rules/)). It exposes records,
tables, columns, webhooks, and other resources through a bearer-authenticated
REST API
([API](https://support.getgrist.com/api/),
[webhooks](https://support.getgrist.com/webhooks/)).

One Grist document with explicit tables can model the full agreed graph and
provide the human grid immediately. Community can run in one container with a
persistent volume. The documents themselves are SQLite databases; production
operation still needs backups, and snapshot storage can be configured against
an S3-compatible store
([self-hosting and storage](https://support.getgrist.com/self-managed/)).

The important license caveat is open-core scope. `grist-core` is Apache-2.0,
but the normal image also contains inert source-available full-edition code; a
clean `grist-oss` image exists. Full-edition features include additional admin,
automation, email, OAuth/MCP, and AI functionality
([official repository](https://github.com/gristlabs/grist-core#features-not-in-grist-core)).

Grist has no native Customer, Opportunity, sponsorship, payment allocation, or
accounting semantics. Finite owns all of those rules. That is acceptable for a
prototype and is still cleaner than Google Sheets, but it is why Grist is the
fallback rather than the strategic recommendation.

### Baserow

Baserow's open-source edition is MIT, while `premium/` and `enterprise/` use
separate licenses
([license](https://github.com/baserow/baserow/blob/develop/LICENSE)). It has
linked tables, lookup and rollup fields, formulas, database tokens, OpenAPI, and
webhooks
([field model](https://baserow.io/user-docs/baserow-field-overview),
[API](https://baserow.io/docs/apis/rest-api)). This can express the schema just
as Grist can.

Its meaningful caveat is that granular table/field role control and audit logs
are paid features
([permissions](https://baserow.io/user-docs/permissions-overview),
[audit](https://baserow.io/user-docs/admin-panel-audit-logs)). That does not
block the trusted internal use case, but it leaves Baserow with no decisive
advantage over Grist for this pilot.

### Frappe CRM and ERPNext

Frappe CRM is a genuine open-source, self-hostable CRM with Leads, Organizations,
Contacts, Deals, products, activities, and a focused sales interface
([product introduction](https://docs.frappe.io/crm/introduction),
[Deal model](https://docs.frappe.io/crm/deal),
[installation](https://docs.frappe.io/crm/introduction/installation)). Frappe
Framework DocTypes can create the missing relational records; each DocType gets
REST CRUD, role permissions, and webhooks
([DocTypes](https://docs.frappe.io/framework/user/en/basics/doctypes),
[REST](https://docs.frappe.io/framework/user/en/api/rest),
[permissions](https://docs.frappe.io/framework/user/en/basics/users-and-permissions)).

The catch is product surface: Frappe CRM's polished frontend is centered on its
standard sales records. The custom subledger DocTypes naturally appear in
Frappe Desk unless Finite builds additional UI. That is a credible framework
choice, not an off-the-shelf fit.

ERPNext is the only shortlisted tool that natively owns most accounting
behavior. A Payment Entry supports customer receipts, supplier payments,
partial or multiple-invoice allocations, advances, multiple currencies, and
unallocated payments; submitting it creates GL entries
([Payment Entry](https://docs.frappe.io/erpnext/payment-entry)). Sales Invoices
and Purchase Invoices create receivable/payable and income/cost effects
([Sales Invoice](https://docs.frappe.io/erpnext/sales-invoice)). ERPNext is
GPL-3.0 and has been developed for many years
([repository](https://github.com/frappe/erpnext)).

That native accounting is exactly why ERPNext is currently the wrong scope.
Finite would be operating a chart of accounts, posting rules, close process,
reconciliation, and accounting authority. Sponsorship still needs
customization because a normal invoice and payment are party-centered, and
customer-paid external spend is not Finite's accounting transaction. Revisit
ERPNext only if the team decides the new system should become the actual books,
with an accountant involved in the design.

## Explicit disqualifiers

- **EspoCRM and SuiteCRM:** credible mature CRMs, but their core advantage is
  conventional CRM breadth. The exact subledger remains custom, and EspoCRM's
  invoicing, purchases, payments, allocations, multi-currency, and reporting
  move into paid extensions
  ([EspoCRM Sales Pack payments](https://docs.espocrm.com/extensions/sales-pack/payments/)).
  They offer no Brain or agent advantage over Twenty.
- **NocoDB:** versions after the January 2026 license change use NocoDB's
  Sustainable Use License rather than an OSI open-source license
  ([current license](https://github.com/nocodb/nocodb/blob/develop/LICENSE.md)).
  Exclude it when genuine open source is a requirement.
- **Teable:** the core applications use AGPL-3.0 with an additional restriction
  on modifying, replacing, or removing brand assets
  ([license](https://github.com/teableio/teable/blob/develop/LICENSE)). Its
  fuller AI self-hosting adds a sandbox plane, registry, object storage, and
  gateway infrastructure. It has no current advantage large enough to justify
  the license and operating complexity for this pilot.
- **Dolibarr:** mature ERP/CRM breadth, but no clearer payer-versus-beneficiary
  model, Brain seam, or product advantage than the stronger shortlisted CRM and
  ERP choices.

## Final recommendation

The user's desire to roll this into the Brain is reasonable. In fact, the
expanded requirement makes Brain more valuable than it appeared when the need
looked like three customer columns: meetings, Wants, a shared Rolodex, customer
history, package proposals, sponsorship, and structured customer economics all
need to meet in one agent-readable context.

The correct caution is not “use a Sheet because building is inappropriate.” It
is “do not accidentally build Salesforce or an ERP.” Build the narrow Brain
module above. Treat the table UI and relational index as a projection over
encrypted Page records, and keep Stripe/Core authority boundaries intact.

Use Grist Community if the team wants data entry immediately without funding
that module. Use Twenty if the team consciously prefers a standalone CRM
product with custom Finite objects. Neither is a better strategic fit than a
successful Brain-native pilot; both are better operational fallbacks than a
large Google Sheet.
