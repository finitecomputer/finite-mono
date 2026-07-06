# Billing v0

Status: active v2 product contract.

Date: 2026-07-02.

## Problem Statement

finitecomputer-v2 needs a real money path before the dashboard can be a
self-serve SaaS product. Billing v0 should let a WorkOS-authenticated user pay
$200/month for one hosted Hermes agent, use Stripe promo codes for whiteglove
or org-sponsored seats, and make Core enforce agent creation and Finite Private
limits.

Billing must not become a second runtime control plane. Stripe owns payment
methods, invoices, coupons, promo codes, and subscription lifecycle events.
Core owns customer organization state, entitlements, runtime-scoped Finite
Private keys, and usage decisions.

## Product Flow

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
9. Existing runtimes are never stopped, destroyed, or volume-deleted
   automatically if billing becomes `past_due`, `canceled`, `unpaid`, or
   `paused`. New agent creation is blocked, and any deeper account action must
   go through a human-reviewed grace/support process.

## Source Of Truth

Stripe owns:

- Products and Prices
- Checkout Sessions
- Billing Portal Sessions
- payment method collection
- invoices, taxes, receipts, coupons, and promotion codes
- subscription status webhooks

Core owns:

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

## Promo Codes

Stripe Checkout is created with `allow_promotion_codes: true`. Whiteglove or
org-paid seats should use Stripe coupons and promotion codes, including 100%
off codes when appropriate, so the user still sees the product value and Stripe
remains the billing ledger.

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
not issue stop, destroy, recover, provider volume deletion, Finite Private key
revocation, or data retention actions from Stripe status alone.

Legacy `off2026` launch-code creation remains only for explicit bridge paths.
The normal v2 dashboard create-agent flow sends no launch code and requires
active billing. Subscription lapses must not delete an existing launch-code
entitlement for a bridge org.

## Finite Private Limits

Core creates the default `finite-private-generous` profile with:

- burst window: 18,000 seconds
- burst limit: 5,000,000 units
- weekly limit: 25,000,000 units

Every Finite Private reservation checks both burst and weekly limits before
upstream work. Denied weekly requests return `weekly_limit_exceeded` and do not
create reservations.

## Destroy Offboarding

Destroy is a runtime lifecycle operation, not account deletion. When a destroy
operation succeeds, Core:

- marks the runtime offline
- clears public runtime URLs
- marks Hermes unavailable
- deactivates runtime links
- removes the runtime relay credential
- revokes active Finite Private API keys scoped to that runtime or project

The subscription and Stripe customer remain in place. Account offboarding,
refunds, cancellation, and retention policy belong to a later billing support
flow.

Destroy must never be triggered automatically for non-payment in Billing v0.
Early users get a generous grace/support process, and provider volume deletion
requires an explicit destroy lifecycle operation.

## Evaluation Design

Billing v0 is accepted when:

- Core tests prove unpaid users cannot create no-launch-code agents.
- Core tests prove active Stripe billing grants one no-launch-code agent
  entitlement.
- Core tests prove inactive Stripe billing blocks new agent creation.
- Core tests prove inactive Stripe billing does not stop, destroy, delete, or
  revoke the already-running runtime.
- Core tests prove Finite Private burst and weekly limits deny before upstream
  work.
- Core tests prove destroy offboards runtime-scoped credentials and Finite
  Private keys.
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
