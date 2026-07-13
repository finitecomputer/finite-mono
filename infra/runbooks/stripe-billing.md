# Stripe billing production support

This runbook covers the one live `Finite Computer Hosted Agent` subscription
path. Stripe owns payment state; Core owns entitlement state. Never repair a
billing incident by editing Core's database, changing a Subscription to match a
selected row, or deleting a Customer, invoice, runtime, or recovery material.

## Safety boundary

Before any mutation, record the deployed revision/system closure and switch the
dashboard back to `FC_DASHBOARD_RUNTIME_MODE = "canary"` if new Checkouts must
stop. The rollback is the last known-good committed NixOS closure. Existing
Stripe and Core records remain evidence and are not rollback debris.

Read-only evidence may include Stripe request/event ids, Customer and
Subscription ids, Core organization id, configured Price id, timestamps,
status, and the deployed closure. Keep customer email, payment details, webhook
bodies, API keys, signing secrets, and database credentials out of tickets,
logs copied off-host, and this public repository.

## Read-only readiness audit

Create a temporary live restricted key with only Account, Product, Price,
Stripe Tax settings, Customer Portal configuration, and Event Destination read
permissions. Run from a trusted operator shell without storing the key in a
file:

```sh
STRIPE_READINESS_SECRET_KEY='<temporary-rk_live>' \
STRIPE_EXPECTED_ACCOUNT_ID='<approved-acct-id>' \
STRIPE_EXPECTED_PRICE_ID='price_1TsqWWA50jhCdjMEhQLEBpvR' \
  npm --prefix finitecomputer-v2/apps/dashboard run stripe:readiness
```

The report contains no secret or customer values. Expire the audit key after
the run whether it passes or fails. A failure keeps Checkout dark.

## Paid but not synchronized

1. In Stripe, read the Checkout Session, current Subscription, and delivery for
   the matching event. Confirm the Customer, live Price, and
   `finite_customer_org_id` metadata agree. Do not infer identity from email.
2. On `finite-lat-1`, read the dashboard journal around the event time and
   Core's authenticated billing overview. Do not print environment files.
3. If Stripe delivery is pending, wait. If it failed, fix the proven endpoint
   or application cause, then use Stripe's redelivery for the original event.
   The handler fetches current Subscription state and Core rejects stale
   ordering; do not synthesize an event or update a database row.
4. If payment succeeded but metadata, Customer, Product, or Price differs,
   stop. Keep customer admission dark and escalate for a reviewed refund or
   reconciliation decision.

## Duplicate or out-of-order delivery

Read the event ids and Core's `last_stripe_event_id`/event timestamp. Repeated
delivery of the same event and a stale event should be harmless. If either
changes entitlement incorrectly, stop Checkouts, preserve both event ids, and
roll back the application revision. Do not delete either event or Subscription.

## Wrong Price

The only accepted live Price is `price_1TsqWWA50jhCdjMEhQLEBpvR`, configured
identically in Dashboard and Core. A Subscription containing another Price
must not grant entitlement. Stop new Checkouts, capture the Session and
Subscription ids, and correct the reviewed deployment or Stripe setup. Refund
or cancellation is a separate owner-approved customer action.

## Past due, cancellation, and refunds

- Smart Retries and Stripe's failed-payment email own renewal recovery. Core
  blocks new creation while preserving an existing runtime and its data.
- Customer Portal cancellation is at period end. Confirm the Subscription has
  `cancel_at_period_end=true`; never translate it into runtime teardown.
- Refunds are manual customer-support decisions in Stripe. A refund is not a
  Core mutation and does not authorize retirement or purge.

## Disputes

Handle evidence and response in Stripe. Keep personal/payment evidence in the
approved private support system. A dispute can change Stripe status through
normal events; it never authorizes compute or recovery-data deletion.

## Key or webhook-secret rotation

1. Keep Checkout dark and name the current NixOS closure as rollback.
2. Create the replacement restricted key or Event Destination secret with the
   same reviewed scope. Transfer it directly to root-owned
   `/etc/finite/dashboard.env`; never print either value.
3. Restart only `podman-finite-saas-dashboard.service`, verify active/running,
   local and edge health, then run the read-only readiness audit.
4. Revoke the old key only after the replacement succeeds. For a webhook
   secret, coordinate the endpoint change so one accepted secret covers every
   delivery; otherwise keep Checkout dark and redeliver failed original events
   after the application is ready.
