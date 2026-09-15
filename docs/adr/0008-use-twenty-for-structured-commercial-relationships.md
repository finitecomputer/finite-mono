---
status: accepted
---

# Use Twenty for Finite Business and Brain for narrative context

Finite will self-host Twenty as **Finite Business**, the internal business hub.
Its first module is the structured Commercial Relationship Register and
Customer Commercial Subledger. Finite will build a versioned Twenty app whose
data model follows `docs/commercial-relationships/CONTEXT.md`, while the
Organization Brain remains the durable home for meeting notes, Organization
Wants, relationship narrative, research, and proposal work.

Finite operates one private internal Twenty workspace; customers receive no
Twenty workspace or access in this phase. The Finite app declares its durable
schema and core views in source. Finite does not fork Twenty or depend on
production click-configuration for its data model.

Twenty's standard Company, Person, and Opportunity records supply the familiar
CRM and shared-Rolodex surfaces. Finite-owned objects and relations represent
Commercial Accounts, payer responsibilities, Arrangements, Purchased Packages
and Offering Lines, Charges, Incoming Payments and allocations, Supplier Costs,
Customer External Spend, Contributions, fulfillment paths, and reconciliation
warnings. Payer and beneficiary remain separate relations; a sponsor payment is
never attributed as cash paid by the beneficiary.

Twenty does not replace upstream authorities. Stripe owns Stripe customers,
prices, subscriptions, invoices, payments, credits, disputes, and refunds. Core
owns Account Auth-linked admission, entitlements, Projects, Hosting Tiers, and
runtime state. Twenty stores stable source-system references and rebuildable
projections of those facts. It is authoritative only for curated commercial
relationships and manual or legacy commercial facts that no upstream ledger
represents.

Brain and Twenty do not maintain two editable copies of structured commercial
facts. Brain may contain generated, refresh-labeled summaries and stable links
to Twenty records. Agents may append sourced meeting context and Organization
Wants in Brain. Structured writes to payer, price, Package, Arrangement, Charge,
Payment, allocation, or Opportunity stage go through typed Twenty operations
and require the human confirmation defined by the workflow. An unambiguous
human instruction confirms the facts it states; the agent writes them and
reports what changed, but pauses before an ambiguous financial overwrite.

In the first version, unattended Brain agents may read Twenty but may not write
structured Twenty records. Ordinary entry is nevertheless agent-driven: a
human invokes a commercial update agent, which writes through the typed Twenty
operations. Humans are not expected to maintain the register through manual
form entry. Twenty autonomous agents and automatic mailbox ingestion remain
disabled. Twenty may display refresh-labeled interaction metadata and a short
generated summary that links to the authoritative Brain meeting Page.

Provenance remains deliberately lightweight. Finite uses Twenty's standard
record creator and create/update timestamps and allows one Source Reference on
a material commercial record when useful. It does not add field-level lineage,
a conflicting-source engine, an event-sourced mutation ledger, a custom
append-only audit database, or a compliance tracking subsystem. The MVP also
does not add automated duplicate detection or record merging; agents merely
search for an obvious existing Organization before creating one.

Twenty initially remains a separate internal service linked from Brain Pages;
Finite does not embed the Twenty interface into the Brain MVP. The first pilot
ships only the NED ordinary-purchase path and three global views: the
Organization directory, open Opportunity pipeline, and current customers.
Warnings remain inline rather than gaining a dedicated reconciliation queue.
After each user-invoked write, the agent reports what changed and any unresolved
facts without requiring a separate approval or audit interface.

This system reports Customer Economics, not Finite's authoritative P&L. It does
not own a chart of accounts, double-entry journal, fiscal close, revenue
recognition, tax treatment, or complete company expenses. A future accounting
system may consume or reconcile the subledger without changing the commercial
vocabulary.

Monthly Recurring Revenue is a derived commercial metric, not cash collection
or accounting revenue. It normalizes effective-dated active recurring price
terms, counts a shared Package price only once, and excludes one-time charges,
uncommitted variable usage, Contributions, Customer External Spend, and
Pass-Through Charges. Sponsored MRR may be viewed by payer or beneficiary but
is counted only once globally. Non-USD terms keep a distinct sourced monthly
USD normalization; calculated MRR is not a writable input. A projection-only
refresh updates Company totals as effective dates pass without rewriting source
facts. Account cash history starts unreconciled, so the cash projection remains
unknown until a sourced completeness check makes an empty payment history a
meaningful zero. Projection reads paginate completely or fail before writing a
possibly partial total.

## Considered options

- **Brain-native structured records:** rejected for the first implementation
  because Brain currently has encrypted Page sync and agent Working Trees but
  no human grid, typed record validator, relational query engine, or
  domain-specific forms. Brain remains the narrative and agent layer, and a
  later Brain-native structured view may consume the same domain model.
- **Grist Community or Google Sheets:** rejected as the target because they
  provide flexible tables but not the CRM-shaped Company, Person, Opportunity,
  activity, and application-extension surfaces the team wants.
- **`trycompai/crm`:** retained as a design donor, not adopted; its young,
  opinionated HubSpot-replacement model and operating stack would still require
  replacing most of the domain and subledger.
- **ERPNext:** deferred unless Finite explicitly adopts authoritative accounting
  and operates a general ledger with accounting oversight.

## Consequences

- The Finite Twenty app, schema evolution, connector, tests, and deployment
  definitions land in `finite-mono`; the third-party source is not forked
  casually or made another Source Authority.
- Production requires a pinned Twenty release, reviewed license obligations,
  secret-free configuration, scoped machine credentials, monitored upgrades,
  and a proven backup and empty-target restore before the register becomes the
  sole authority for manual commercial facts.
- The private deployment runs beside Grafana on the dedicated monitoring VM,
  not in the coupled product cluster. It has its own PostgreSQL, Redis, local
  file storage, encryption keys, off-host Recovery Set, and restore drill;
  monitoring and Finite Business share only the host and its single Caddy edge.
- Stripe/Core imports are idempotent and read-only with respect to their source
  facts. Meeting and proposal automation cannot bypass the typed Twenty write
  boundary.
- The initial Finite Business implementation remains internal dogfood. Its
  broader name is not authority to absorb every business system, and it does
  not imply a customer-facing product or upstream contribution to Twenty.
