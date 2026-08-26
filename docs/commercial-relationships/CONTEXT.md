# Commercial Relationships

This context describes Finite's internal understanding of prospective,
sponsored, paying, former, and partner organizations. It is independent of the
tool used to maintain that understanding. Twenty is the selected structured
implementation under ADR-0008, but the vocabulary and authority boundaries do
not depend on Twenty. This context does not replace Stripe as the authority for
Stripe money or Core as the authority for product access and runtime state.

## Language

**Commercial Relationship Register**:
The internal source of truth for curated organization identity, commercial
roles, actual arrangements, and narrative relationship context.
_Avoid_: sales CRM, billing ledger, Stripe replacement

**Customer Commercial Subledger**:
The customer-specific record of Charges, Incoming Payments, refunds,
sponsorship, Supplier Costs, and Customer External Spend. It preserves detail
needed for customer economics and later accounting reconciliation without
claiming to be Finite's general ledger.
_Avoid_: chart of accounts, authoritative P&L, revenue-recognition engine

**Commercial Organization**:
One stable real-world company, nonprofit, institution, or other external
organization. The record persists as the organization moves among prospect,
sponsored customer, paying customer, former customer, sponsor, and partner
roles.
_Avoid_: Core Customer Organization, Stripe Customer, Account Auth organization

**Commercial Account**:
The billing and service-ownership entity through which one Commercial
Organization owns purchased services. A Commercial Account belongs to one
Commercial Organization, while another Commercial Account may bear payment
responsibility for its Arrangement.
_Avoid_: Account Auth login, Core Customer Organization, assuming owner and payer are identical

**Contact**:
One person known to Finite who may be associated with a Commercial Organization
or temporarily remain unassigned. A Contact is part of the shared relationship
directory and is not itself a customer Account.
_Avoid_: creating a customer Organization for every person, duplicating an address book

**Shared Rolodex**:
A human- and agent-readable view over Commercial Organizations and Contacts,
including their relationship context. It is not a separately maintained data
store.
_Avoid_: second contact database, copied organization identity

**Interaction Snapshot**:
A read-only, refresh-labeled Twenty summary of one meeting or other meaningful
interaction whose full narrative record lives in FiniteBrain. It records the
date, participants, Commercial Organization links, short generated summary, and
stable Brain Page link.
_Avoid_: second editable meeting note, authoritative transcript

**Commercial Role**:
A time-varying part a Commercial Organization plays in relation to Finite or
another Commercial Organization, such as beneficiary, payer, sponsor, or
partner.
_Avoid_: permanent organization type, access-control role

**Opportunity**:
A small record of one possible purchase or funding decision for a Commercial
Organization or Account. It carries a short description, stage, possible
Package, and link to Organization Wants. Its stages are exploring, proposal
drafted, proposal sent, won, lost, or paused.
_Avoid_: forecast probability, automatic follow-up, treating a meeting note as a sale

**Commercial Arrangement**:
The actual understanding under which Finite provides value to one or more
Commercial Organizations and one or more Commercial Organizations bear payment
responsibility. An Arrangement may be informal and may contain incomplete or
bespoke terms.
_Avoid_: requiring a signed contract, assuming catalog pricing, Stripe Subscription

**Offering**:
A recognizable kind of value Finite provides, such as a hosted agent, Agent
Camp, or inference service. An Offering describes what is provided rather than
how it is implemented.
_Avoid_: deployment host, invoice, bespoke customer price

**Offering Line**:
One promised or purchased item within a Commercial Arrangement. A line records
the actual agreed terms and may be one-time, recurring, usage-based, or included
within a larger purchased bundle.
_Avoid_: inferred standard price, payment allocation without source evidence

**Purchased Package**:
One actual purchase within a Commercial Arrangement that groups Offering Lines
under shared commercial terms or a shared price. Included lines need not be
assigned invented individual prices. An Arrangement may contain multiple
separately purchased Packages.
_Avoid_: assuming a standardized catalog package, allocating a shared price without evidence

**Contribution**:
Non-cash value provided by a partner or other Commercial Organization, recorded
separately from cash revenue. It may include an explicitly identified agreed or
estimated value and the cost it offsets.
_Avoid_: payment, cash collected, silently blending contributed value into revenue

**Fulfillment Path**:
How an Offering is actually delivered, such as the first-class platform or a
legacy system. Offerings with the same commercial meaning may have different
Fulfillment Paths.
_Avoid_: duplicating an Offering solely because its implementation differs

**Organization Wants**:
The simple narrative record of a Commercial Organization's stated needs,
desired outcomes, and proposal-relevant context. Meetings may add sourced notes
to Organization Wants without changing commercial or financial facts.
_Avoid_: automatically inferred purchase, unapproved arrangement change

**Lifetime Net Cash**:
Cash collected for a Commercial Organization or Arrangement less refunds and
other cash reversals, derived from authoritative financial events with
currencies preserved.
_Avoid_: recurring price multiplied by elapsed months, contribution value

**Monthly Recurring Revenue (MRR)**:
The derived monthly value of active recurring commercial terms, independently
of whether the current payment has arrived. Each price term has an effective
start and optional end; a price change ends the old term and adds a new one.
MRR is reported separately from cash collected and from variable or one-time
consideration.
_Avoid_: Lifetime Net Cash, last month's variable usage, one-time Package price

**Incoming Payment**:
Fiat or other value actually received by Finite from a paying Commercial
Account. It records the native amount and asset and links to the Arrangement,
Package, charge, or source record when known. The paying Account may differ from
the Account that owns the purchased services.
_Avoid_: invoice, amount promised, customer payment directly to a third party

**Charge**:
An amount a Commercial Account is expected or requested to pay. A Charge may be
supported by an invoice, agreement, message, or manual note and remains
distinct from any value actually received.
_Avoid_: Incoming Payment, assuming every informal charge has an invoice

**Payment Allocation**:
The explicit application of all or part of an Incoming Payment to one or more
Charges. An Incoming Payment may be partially allocated or remain unallocated
when historical information is incomplete.
_Avoid_: inventing an allocation, rewriting the Incoming Payment

**Supplier Cost**:
Value Finite pays to an external supplier, such as OpenRouter. It may be linked
to the Commercial Accounts, Packages, or Offering Lines whose fulfillment
caused the cost, but it remains distinct from an Incoming Payment. The Register
does not decide when accounting recognizes the cost as an expense.
_Avoid_: netting against customer revenue, customer-paid external spend

**Customer External Spend**:
Value a customer pays directly to a third-party supplier for something related
to Finite's service. It may be recorded for relationship and total-cost context
but is neither Finite revenue nor a Finite Supplier Cost.
_Avoid_: Incoming Payment, pass-through revenue, Finite cash outflow

**Pass-Through Charge**:
An amount Finite charges a Commercial Account to recover a linked Supplier
Cost. The Supplier Cost, Pass-Through Charge, and eventual Incoming
Payment remain separate events even when their amounts are equal.
_Avoid_: collapsing supplier payment and customer reimbursement into one transaction

**Reconciliation Warning**:
A visible marker that a mapping or commercial fact is unknown, incomplete, or
ambiguous and needs human attention. It does not attempt automated truth
resolution.
The MVP renders the marker inline and does not add a dedicated warning queue or
view.
_Avoid_: evidence-scoring engine, silently choosing among conflicting values

**Source Reference**:
A lightweight pointer to the evidence used for a material commercial fact,
such as a Brain Page, invoice, payment record, agreement, message, or dated
statement from an authorized team member. Twenty's ordinary creator and
timestamp fields provide basic provenance around the record.
_Avoid_: field-level lineage, conflicting-source engine, custom event store,
compliance ledger

**Customer Economics**:
Operational views of consideration received, sponsorship, outstanding Charges,
Supplier Costs, Customer External Spend, and attributable margin. These views
are not Finite's profit-and-loss statement.
_Avoid_: P&L, revenue recognition, complete company profitability

## Relationships

- A **Commercial Organization** keeps one identity while its **Commercial
  Roles** change over time.
- In the first version, Commercial Roles are simple multi-select labels. The
  dated Opportunities, Arrangements, Packages, and Payments preserve the useful
  history without a separate temporal role subsystem.
- A **Commercial Account** belongs to one **Commercial Organization** and owns
  purchased services; payment responsibility may belong to another Commercial
  Account.
- A Commercial Organization normally has one **Commercial Account**; additional
  Accounts exist only for genuinely distinct billing or service-ownership
  relationships, not for every Package or Project.
- A prospect needs only a Commercial Organization, Contacts, and optionally an
  Opportunity. Its Commercial Account is created only when billing, service
  ownership, or sponsorship becomes real.
- The **Shared Rolodex** is derived from **Commercial Organizations** and
  **Contacts**.
- A **Commercial Arrangement** names beneficiaries and payment responsibility
  separately; a sponsor may pay for value received by another Commercial
  Organization.
- An **Opportunity** may become a **Commercial Arrangement** when won; agents
  may add Organization Wants but do not automatically change Opportunity stage
  or create the Arrangement. The won Opportunity remains as history and links
  to the resulting Arrangement or Purchased Package.
- A **Commercial Arrangement** contains one or more **Offering Lines**.
- A **Purchased Package** groups Offering Lines sold together under shared
  commercial terms; separately priced purchases may use separate Packages.
- A recurring Package with one shared price contributes to MRR once. Its
  included Offering Lines describe delivery but do not add invented MRR.
- An **Offering Line** identifies an **Offering** independently of its
  **Fulfillment Path**.
- An Offering Line's delivery status is planned, active, completed, or
  cancelled. Payment state is represented separately. Only active Offering
  Lines appear as value the Account is currently receiving.
- A Commercial Organization is a current customer when one of its Accounts owns
  an active Offering Line. Payer and sponsor roles remain separate and do not
  change which Account owns the active service.
- A **Contribution** is reported separately from **Lifetime Net Cash**.
- An **Incoming Payment** contributes to the payer's amount-paid total even
  when it settles services owned by a different Commercial Account.
- A **Charge** records what is due independently of whether an **Incoming
  Payment** arrives; a **Payment Allocation** links the two only when known.
- A sponsored Account's history identifies the paying Account and funded
  Arrangement without attributing the payer's cash to the sponsored
  Commercial Organization.
- Sponsored MRR can be viewed by payer or beneficiary, but each underlying
  recurring price term contributes only once to Finite's global MRR.
- A reimbursement flow preserves its **Supplier Cost**, **Pass-Through
  Charge**, and **Incoming Payment** separately.
- **Customer External Spend** may inform customer economics but never enters
  Finite cash revenue or expense totals.
- A digital-asset **Incoming Payment** preserves its native units, network,
  receipt time, transaction reference, and receipt-time reporting-currency
  value when known; later market prices never rewrite the original receipt.
- Customer summaries use USD as the reporting currency while preserving each
  financial event's native currency or asset and conversion evidence.
- Variable usage displays the latest known actual amount together with its
  period. The register does not present that amount as an invented recurring
  run rate.
- MRR excludes one-time purchases, variable usage without a minimum recurring
  commitment, Contributions, Customer External Spend, and Pass-Through Charges.
- Incomplete history uses a **Reconciliation Warning** and an unknown value,
  never a fabricated zero.
- **Organization Wants** may inform package and proposal preparation but do not
  prove a purchase or authorize a change to a Commercial Arrangement.

## Authority boundaries

- The **Commercial Relationship Register** owns curated organization identity,
  roles, relationships, narrative context, and the terms of informal or
  otherwise unrepresented arrangements.
- The **Customer Commercial Subledger** owns customer-specific operational
  transaction detail but not double-entry accounting, fiscal periods, revenue
  recognition, or Finite's authoritative P&L.
- Stripe owns Stripe customers, prices, subscriptions, invoices, cash
  collection, credits, disputes, and refunds.
- Finite Computer **Core** owns Account Auth-linked admission, entitlements,
  Projects, Hosting Tiers, and runtime state.
- A manual financial fact belongs in the Register only when no upstream ledger
  represents it, and must name its source document or other evidence.
- A dated statement from an authorized team member may be the Source Reference
  for an informal term or otherwise undocumented manual fact. An unsupported
  payment fact retains a Reconciliation Warning until corroborated.
- Calculated views are rebuilt from their source facts; humans correct the
  source fact or mapping rather than editing a calculated total.

## Implementation boundary

- Twenty is the structured source of truth for the **Commercial Relationship
  Register** and **Customer Commercial Subledger**.
- Finite operates one private internal Twenty workspace. Commercial
  Organizations and Contacts are records in that workspace; customers do not
  receive Twenty workspaces or access during the internal phase.
- The schema is delivered as a versioned Finite Twenty app in `finite-mono`.
  Durable objects, fields, relations, stages, roles, and core views are declared
  in code rather than depending on production click-configuration. Finite does
  not fork Twenty for this module.
- Twenty's standard Company, Person, and Opportunity records provide the CRM
  and **Shared Rolodex** surfaces. Twenty Company maps to **Commercial
  Organization**, Person maps to **Contact**, and Opportunity maps to
  **Opportunity**. **Commercial Account** and the remaining commercial
  vocabulary use Finite-owned Twenty objects.
- FiniteBrain owns meeting notes, **Organization Wants**, relationship
  narrative, research, and proposal work. It may contain generated,
  refresh-labeled Twenty summaries but not a second editable copy of structured
  commercial facts.
- Twenty may show an **Interaction Snapshot** for Brain-owned narrative. It
  never becomes the editable meeting authority.
- Typed Twenty operations are the only structured mutation path for people and
  agents. Financial, payer, Arrangement, Package, allocation, and Opportunity
  stage changes require the workflow's human confirmation.
- An unambiguous user instruction is sufficient confirmation for the facts it
  states. The invoked agent writes those facts and reports the change; it
  pauses before an ambiguous financial overwrite rather than expanding the
  instruction's authority.
- Agents correct ordinary records in place with an updated Source Reference.
  An erroneously recorded Incoming Payment is marked void rather than silently
  deleted; this does not introduce an append-only accounting system.
- In the first version, unattended Brain agents have read-only Twenty access
  and append sourced Organization Wants only in Brain. A user-invoked
  commercial update agent performs the ordinary Twenty data entry through the
  typed operations; humans are not expected to maintain the register by
  clicking through forms. Twenty autonomous agents and automatic mailbox
  ingestion remain disabled so that two systems do not independently mutate
  relationship records.
- Twenty's automatic creator and create/update timestamps, plus a lightweight
  optional Source Reference on material commercial facts, are the initial provenance
  mechanism. Finite does not build a custom append-only change log, event store,
  or compliance-grade audit system for this register.
- The MVP has no duplicate-detection, alias-matching, or record-merge subsystem.
  The invoked agent simply searches for an obvious existing Organization before
  creating one and does not perform uncertain automatic merges.
- After a write, the invoked agent reports the records it created or changed
  and any unresolved facts. The MVP adds no separate audit or approval UI.
- Twenty initially remains a separately deployed internal service linked from
  Brain Pages. Embedding Twenty's interface inside the Brain is deferred.
- This boundary is recorded in
  [`ADR-0008`](../adr/0008-use-twenty-for-structured-commercial-relationships.md).

## Pilot sequence

- NED is the first real vertical slice. It is intentionally simpler than the
  HRF sponsorship case: one partner Organization and one Account are enough to
  prove the ordinary path before modeling the exception-heavy path. NED has a
  historical won Opportunity and Purchased Package for an Agent Camp, plus a
  distinct exploring Opportunity for follow-on work. The open Opportunity has
  no invented value before a real price or proposal exists.
- Agent Camp is one Offering. Camp size, cohort, dates, location, and bespoke
  terms belong on the Purchased Package or Offering Line rather than creating a
  new Offering for every camp permutation.
- The Commercial Organization page is the ordinary front door. NED's one
  Commercial Account is summarized inline; separate Account navigation becomes
  prominent only when sponsorship or multiple Accounts make it useful.
- Existing FiniteBrain context may seed relationship identity, Contacts,
  Interaction Snapshots, and Organization Wants for NED. Commercial terms,
  Charges, and Incoming Payments are entered only from their actual source
  evidence; event notes do not prove a purchase or payment.
- The NED MVP implements only the objects and fields required for this ordinary
  purchase path. Sponsorship, OpenRouter, Bitcoin, Contributions, and Payment
  Allocations remain documented extensions until their respective pilot cases
  require them.
- The initial global views are the Organization directory, open Opportunity
  pipeline, and current-customer view. Reconciliation Warnings appear inline,
  not in a dedicated global view.
- HRF is a later acceptance case for sponsor/beneficiary separation, not the
  first record used to shape the basic Twenty experience.
- The NED slice is accepted when an agent can answer what NED bought, what it
  paid, what it currently receives, who matters there, and what Opportunity
  remains, with each material answer traceable to a Source Reference.
- Twenty becomes the sole structured authority only after the NED records are
  reconciled and the pinned deployment passes export, backup, and empty-target
  restore testing.
