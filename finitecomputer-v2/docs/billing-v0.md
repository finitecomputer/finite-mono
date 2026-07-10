# Billing v0

Status: active v2 product contract.

Date: 2026-07-02.

## Problem Statement

finitecomputer-v2 needs a real money path before the dashboard can be a
self-serve SaaS product, plus a deliberate non-Stripe access path for
white-glove training and other approved sponsored use. Billing v0 should let a
WorkOS-authenticated user pay $200/month for one hosted Hermes agent or redeem
a Core-owned Launch Code, while Core enforces agent creation and Finite Private
limits for both paths.

Billing must not become a second runtime control plane. Stripe owns payment
methods, invoices, coupons, promotion codes, and subscription lifecycle
events. Core owns Launch Codes, customer organization state, entitlements,
runtime-scoped Finite Private keys, and usage decisions.

## Paid Customer Flow

1. User signs in through WorkOS.
2. Dashboard asks Core for `/api/core/v1/me/billing`.
3. If the customer organization does not have active billing, dashboard shows a
   billing setup panel instead of agent creation.
4. The user starts Stripe Checkout for the standard hosted-agent recurring
   price. Checkout allows promotion codes.
5. Stripe sends subscription webhooks to `/api/stripe/webhook`.
6. The dashboard webhook verifies the Stripe signature, fetches the current
   Subscription from Stripe, checks that it contains the standard hosted-agent
   Price, and syncs the subscription to Core.
7. Core grants one no-launch-code agent creation entitlement when the
   subscription is `active` or `trialing`.
8. The dashboard shows the create-agent form.
9. Existing runtimes are never stopped, retired, purged, or volume-deleted
   automatically if billing becomes `past_due`, `canceled`, `unpaid`, or
   `paused`. New agent creation is blocked, and any deeper account action must
   go through a human-reviewed grace/support process.

## Sponsored Launch Code Flow

1. A dashboard administrator whose validated WorkOS session selects Finite's
   configured internal operator organization asks Core
   to generate a named Launch Code Batch containing the exact number of
   individually single-use codes needed for the sponsored cohort and an
   explicit expiry.
2. Core records the batch, stores only the material needed to verify future
   redemptions, and returns the plaintext codes once.
3. The dashboard offers a one-time copy/download handoff. Later admin views
   show batch metadata, revocation state, and redemption status but never the
   plaintext codes. The organizer distributes the codes outside Finite; this
   flow does not maintain a participant roster or send invitations.
4. The user signs in through WorkOS.
5. The dashboard accepts one Launch Code and sends it to Core with
   the agent-creation request.
6. Core atomically binds the first successful redemption to one Account Auth
   organization and records the resulting sponsored agent-creation
   entitlement. A retry of the same idempotent creation request returns the
   same result; another organization cannot redeem that code.
7. The entitlement permits the same bounded agent-creation workflow as the
   paid path without creating or implying a Stripe customer, subscription,
   payment, coupon, or promotion.
8. The resulting Agent Runtime follows the same product, lifecycle, data, and
   recovery contracts as a paid runtime. Launch Code access is not a second
   runtime architecture.

## Source Of Truth

Stripe owns:

- Products and Prices
- Checkout Sessions
- Billing Portal Sessions
- payment method collection
- invoices, taxes, receipts, coupons, and promotion codes
- subscription status webhooks

Core owns:

- Launch Code Batch issuance, validation, revocation, and sponsored-access audit
- `customer_orgs`
- `customer_billing_accounts`
- `agent_creation_entitlements`
- agent creation requests and runtime links
- Finite Private grant/key state
- Finite Private burst and weekly usage limits

Dashboard owns only adapter code between the signed-in account, Stripe, and
Core. It is not allowed to grant entitlements without Core recording them.

## Stripe Setup

Create one recurring Stripe Price for the standard hosted agent plan:

- Product name: `Finite Computer Hosted Agent`
- Price: `$200/month`
- Currency: `USD`
- Trial: none

The amount and interval live in Stripe, not in code.

Required dashboard environment:

```sh
STRIPE_SECRET_KEY=<stripe-secret-key>
STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID=<stripe-price-id>
STRIPE_WEBHOOK_SECRET=<stripe-webhook-signing-secret>
FC_DASHBOARD_BASE_URL=https://finite.computer
```

`FC_DASHBOARD_BASE_URL` must be the public dashboard origin so Checkout and the
Billing Portal return users to the right place. `NEXT_PUBLIC_APP_URL` and
`FC_DASHBOARD_PUBLIC_URL` are accepted aliases for local experiments, but the
finite.computer deploy should use the same base URL env as the rest of the
dashboard.

Local development can use the Stripe CLI to forward webhooks:

```sh
stripe listen --forward-to localhost:3000/api/stripe/webhook
```

Use the printed webhook signing secret as `STRIPE_WEBHOOK_SECRET`.

The current dashboard Checkout flow is a server-side redirect. It does not
need `NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY` yet.

## Test Clock E2E

Use the dashboard-owned Stripe test-clock harness before treating billing
changes as safe:

```sh
cd apps/dashboard
STRIPE_SECRET_KEY=<stripe-test-secret-key> \
STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID=<stripe-test-price-id> \
npm run test:stripe-billing-clock
```

The harness starts disposable Postgres and Core services locally, creates a
Stripe test clock, creates a send-invoice subscription against the configured
hosted-agent Price, sends signed webhook payloads through the real dashboard
webhook route, and asserts Core billing state after:

- `active`
- `past_due`
- `canceled`
- a deliberately stale active update arriving after cancellation

It is intentionally opt-in because it creates Stripe test-mode objects and
requires the test secret key. By default the harness deletes its Stripe test
clock and local services. Set `FC_STRIPE_BILLING_E2E_KEEP_SERVICES=1` or
`FC_STRIPE_BILLING_E2E_KEEP_STRIPE=1` only while debugging.

## Stripe Promotion Codes

Stripe Checkout is created with `allow_promotion_codes: true`. Stripe coupons
and promotion codes discount a paid-customer subscription, including 100% off
when the relationship still belongs in the Stripe billing ledger. They are
distinct from Core Launch Codes and are not required for white-glove training
or other approved sponsored access.

## Webhook Contract

Dashboard handles these Stripe events:

- `checkout.session.completed`
- `customer.subscription.created`
- `customer.subscription.updated`
- `customer.subscription.deleted`

For Checkout sessions and subscriptions, metadata must include:

```text
finite_customer_org_id
```

Core sync records:

- Stripe customer id
- Stripe subscription id
- Stripe price id
- subscription status
- current period end from the first subscription item
- cancel-at-period-end flag
- last Stripe event id

Stripe does not guarantee webhook ordering. The dashboard webhook must fetch the
current Subscription for every `customer.subscription.*` event before syncing
Core. Core ignores subscription events for a non-current Subscription id unless
the current subscription is terminal and the incoming subscription is active or
trialing. This prevents delayed old events or accidental second subscriptions
from flipping billing state backwards.

Core must be configured with the standard hosted-agent Price id
(`STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID` or
`FC_CORE_STANDARD_STRIPE_PRICE_ID`). Active or trialing subscriptions only grant
an agent entitlement when the synced Stripe Price id matches that configured
Price. Wrong-product subscriptions fail closed.

## Entitlement Rules

`active` and `trialing` subscriptions allow one new hosted agent runtime.

Inactive subscription states block new agent creation:

- `incomplete`
- `incomplete_expired`
- `past_due`
- `canceled`
- `unpaid`
- `paused`

Inactive subscription states do not mutate existing runtime state. Core must
not issue stop, Runtime Retirement, recover, Purge User Data, provider volume
deletion, Finite Private key revocation, or data retention actions from Stripe
status alone.

Launch Code creation is a supported path for white-glove training and other
approved sponsored access. The paid-customer path sends no Launch Code and
requires active billing. A Launch Code never represents paid billing, and a
Stripe subscription lapse must not silently delete an independently granted
Launch Code entitlement.

Every Launch Code belongs to a named batch, is individually single-use, and is
bound to one Account Auth organization on first successful redemption. Only a
WorkOS-authenticated administrator in Finite's configured internal operator
organization may ask Core to issue or revoke a batch. The operator
organization is not a Core Customer Organization. A batch has an explicit code
count and expiry, defaults to seven days, and may be configured for at most 30
days. Indefinite batches are not allowed; the internal canary uses a 24-hour
batch. A batch may revoke its remaining unredeemed codes. Expiry or revocation
never stops an existing Agent Runtime or removes an already redeemed
entitlement. Core returns plaintext codes only in the issuance response, then
exposes only batch identity, redemption time, and the receiving organization;
plaintext code values never appear in source, logs, later reads, or ordinary
audit output. An optional operator CLI may call the same Core API, but database
edits and provider shell access are not issuance paths.

Keep this implementation deliberately small: Core needs only the batch facts,
single-use code verification/redemption facts, issuance/list/revoke operations,
and the existing agent-creation path. Do not build a campaign engine,
participant directory, invitation mailer, scheduling system, analytics stack,
or separate entitlement service for Launch Codes.

Current implementation gap: Core accepts the repository-visible, hard-coded
`off2026` value for any new organization and grants one agent-creation
entitlement. It has no campaign record, expiry, aggregate redemption cap, or
issuance/revocation policy. Treating that public constant as a secret is not an
acceptable long-term Launch Code contract.

## Finite Private Limits

Finite Private limits are a runaway guard, not a price meter or customer
budget. Their purpose is to interrupt an agent that loops on inference
continuously instead of allowing it to hammer the shared service indefinitely.
Launch and billing decisions do not attempt to translate usage units into
dollars.

Core creates the default `finite-private-generous` profile with:

- burst window: 18,000 seconds
- burst limit: 5,000,000 units
- weekly limit: 25,000,000 units

Every Finite Private reservation checks both burst and weekly limits before
upstream work. Denied weekly requests return `weekly_limit_exceeded` and do not
create reservations.

## Runtime Retirement And Data Purge

Runtime Retirement is a lifecycle operation, not account deletion or data
deletion. When retirement succeeds, Core:

- marks the runtime offline
- clears public runtime URLs
- marks Hermes unavailable
- deactivates runtime links
- removes the runtime relay credential
- revokes active Finite Private API keys scoped to that runtime or project
- retains a restore-verified Recovery Snapshot through the declared retention
  period

The subscription and Stripe customer remain in place. Account offboarding,
refunds, cancellation, and retention policy belong to a later billing support
flow.

Runtime Retirement must never be triggered automatically for non-payment in
Billing v0. Early users get a generous grace/support process, and provider
volume deletion is not part of retirement.

Purge User Data is a later and separately authorized irreversible operation.
Billing state never authorizes it. Before purge, Core requires the retention
period to expire, a recent empty-target restore for the Recovery Snapshot, an
offered user export, explicit user confirmation, and a purpose-bound purge
authorization. Until that contract exists, the current provider `destroy` path
must preserve recovery material or remain unavailable.

## Evaluation Design

Billing v0 is accepted when:

- Core tests prove unpaid users cannot create no-launch-code agents.
- Core tests prove active Stripe billing grants one no-launch-code agent
  entitlement.
- Core tests prove inactive Stripe billing blocks new agent creation.
- Core tests prove inactive Stripe billing does not stop, retire, purge,
  delete, or revoke the already-running runtime.
- Core tests prove Finite Private burst and weekly limits deny before upstream
  work.
- Core tests prove Runtime Retirement offboards runtime-scoped credentials and
  Finite Private keys while preserving restorable recovery material.
- Core tests prove no Stripe state, stop, or retirement transition can invoke
  Purge User Data.
- Stripe test-clock E2E proves the webhook-to-Core path for active, past-due,
  canceled, and stale out-of-order subscription updates against Core on
  Postgres.
- Dashboard tests pass.
- Dashboard production build passes.
- CI runs Rust fmt, clippy, workspace tests, dashboard lint, dashboard tests,
  and dashboard build.

## Open Decisions

- Whether standard billing eventually allows more than one hosted agent per
  customer organization.
- Exact customer-facing cancellation and data retention policy.
- The human-reviewed grace/support process for unpaid or past-due accounts.
