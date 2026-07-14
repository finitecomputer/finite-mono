# Stripe Production Activation

Status: ACTIVE

Sequence note: Paul requested this production plan on 2026-07-13 after gaining
access to the official Finite Supply Stripe organization, then explicitly
activated it later that day with the direction to rebase onto `main`, build,
test, and ship. Stripe Checkout Readiness is PAUSED with its queue and human
test-mode acceptance preserved. Hosted Web Chat product continuity is accepted
and its run deleted on `main`; the separately proposed Hosted Web Chat
Disaster Recovery empty-target proof remains a hard gate before paid customer
admission.

Owner: Paul

Opened: 2026-07-13

Expires: 2026-08-03

Acceptance: From one dedicated invited production account, a human sees the
exact Finite Computer offer and policies, pays a real $200 USD monthly charge
through live Stripe Checkout, returns to the sealed agent draft, and the live
signed webhook records the matching active subscription in Core before the
draft creates exactly one fresh agent. The Stripe payment, Customer,
Subscription, invoice/receipt, four subscribed event types, Core billing row,
and agent-creation request all retain mutually consistent non-secret
identifiers. No operator edits Core billing state. Paid admission remains
blocked unless Hosted Web Chat Disaster Recovery has also passed its real
off-host, empty-target restore acceptance.

## Problem statement

Billing v0 has a real Checkout, Portal, webhook, and Core entitlement path, but
production is not an environment-variable flip:

- the live Nix dashboard still uses `FC_DASHBOARD_RUNTIME_MODE = "canary"`,
  which deliberately hides the paid path;
- the checked-in `price_1TouEuFwiFww4itkeTQNPYR6` returns `resource_missing`
  when read through the production live key, while the verified live Product's
  default Price is `price_1TsqWWA50jhCdjMEhQLEBpvR`;
- test-mode browser acceptance is still paused and has not been claimed;
- Portal, live webhook, emails, payment methods, tax, dunning, public business
  information, and API-key restrictions are account-scoped Dashboard state;
- Checkout does not yet opt into automatic tax or pin the launch payment-method
  contract; and
- the customer-facing pre-Checkout surface does not yet state $200 USD/month,
  renewal, cancellation, refund, fulfillment, and support terms.

The outcome is one narrow, inspectable paid path. It is not general public
self-service, multi-plan billing, metered billing, or permission to weaken the
recovery gate.

## Authority and constraints

- Paul explicitly made this the sole ACTIVE run on 2026-07-13 and authorized
  implementation, testing, and deployment. Live customer admission and the
  first real charge remain separately gated below.
- Creating or changing live Stripe objects, credentials, Portal settings,
  webhooks, tax registrations, customer communications, subscriptions,
  refunds, or production deployment state requires explicit production-
  mutation authority at the relevant phase.
- Never place an `sk_`, `rk_`, `whsec_`, bank fact, representative fact, or
  customer identifier in this public repository, a task transcript, or a
  screenshot. Public `acct_`, `prod_`, and `price_` identifiers may be recorded
  only where the plan calls for them.
- All live Stripe objects and keys must belong to the same recorded Finite
  Supply member account and live mode. Stripe organization context alone is not
  proof of merchant-account context.
- No Dashboard selection, existing Price, existing webhook, or prefilled field
  is authority to reuse it. Inspect the object and account first; ambiguity
  fails closed without creating a duplicate.
- Billing status can block a new creation entitlement. It never stops,
  retires, purges, or deletes an existing runtime or its recovery material.
- Do not admit a paying customer before the same Recovery Set has restored onto
  an empty target under
  [`ADR 0001`](../adr/0001-recoverability-precedes-operator-blindness.md)
  and the separately authorized
  [Hosted Web Chat Disaster Recovery](hosted-web-chat-disaster-recovery.md)
  run.

## Product and policy defaults

These are the proposed v0 settings. Changing one updates `billing-v0.md`, the
customer disclosures, tests, and Dashboard checklist before Stripe mutation.

| Decision | Billing v0 default |
| --- | --- |
| Offer | One `Finite Computer Hosted Agent` |
| Price | $200 USD per month, flat-rate, quantity 1 |
| Trial | None |
| Renewal | Automatic monthly renewal until canceled |
| Payment methods | Cards and card wallets only; no asynchronous bank/debit methods |
| Promotion codes | Checkout may accept explicitly created Stripe promotion codes; create none for launch |
| Tax behavior | Stripe Tax account default: Automatic; for this USD Price Stripe treats tax as exclusive and adds it when applicable |
| Cancellation | Customer may cancel at period end in the Portal; no automatic runtime teardown |
| Refunds | Manual support decision under the published policy; no automatic refund flow |
| Failed renewal | Stripe Smart Retries and customer email; leave `past_due` for human review after retries |
| Identity | WorkOS remains account identity; a Portal billing-email edit does not change login identity |

Paul must supply or confirm these real values before live setup; the repository
must not invent them:

1. the exact Finite Supply merchant `acct_...` account to use;
2. a monitored support email, support URL, and support phone/address that are
   safe to show customers;
3. the public Terms, Privacy, cancellation, refund, and service-delivery URLs
   (Paul supplied one combined launch policy URL:
   `https://finite.computer/privacy.txt`);
4. whether customers are business-use, personal-use, or mixed for Stripe Tax
   product classification, plus every jurisdiction where Finite is registered
   to collect tax; and
5. whether `FINITE COMPUTER` is the approved customer statement descriptor.

Tax classification and registrations are legal/accounting facts. If Paul
cannot confirm them, stop before creating the live Price or enabling collection
and escalate to Finite's tax adviser. Stripe Tax does not register the business
merely because its Dashboard switch is on.

## Current implementation inventory

Read-only live discovery on 2026-07-13 verified that the supplied Product is
active and live, named `Finite Computer Hosted Agent`, and has active default
Price `price_1TsqWWA50jhCdjMEhQLEBpvR`: flat-rate, recurring, $200 USD every
month. The Price leaves `tax_behavior` unspecified and therefore inherits
Paul's confirmed Stripe Tax account default of **Automatic**; for USD, Stripe
treats that default as tax-exclusive. The Product currently uses Stripe tax
code `txcd_10103001` (SaaS — business use), which still requires the explicit
customer-use confirmation below. This discovery did not mutate Stripe or
authorize a deployment-config change.

Already present:

- authenticated server-side subscription Checkout and Portal redirects;
- stable, endpoint-scoped idempotency keys for Customer and Checkout creation;
- canonical Core organization metadata on the Checkout Session and
  Subscription;
- signed webhook verification at `POST /api/stripe/webhook`;
- handlers for `checkout.session.completed`,
  `customer.subscription.created`, `customer.subscription.updated`, and
  `customer.subscription.deleted`;
- current-Subscription fetch before Core sync, expected-Price enforcement,
  stale-event defense, and inactive-status fail-closed behavior;
- sealed draft preservation and bounded webhook-sync waiting after Checkout;
- unit/browser coverage and an opt-in Stripe test-clock harness; and
- production secret locations by name in `/etc/finite/dashboard.env`.

Still required before live money:

- finish the PAUSED test-mode run and its real browser Checkout acceptance;
- make the product/tax/payment-method and customer-policy contract explicit;
- prove a least-privilege restricted key in a sandbox;
- add an inspection-only readiness command and production support runbook;
- create or verify the live account objects below;
- deploy the same live Price id to Dashboard and Core while paid UI remains
  dark;
- prove live webhook delivery and read-only cross-system reconciliation;
- pass the recovery/admission gates; and
- separately authorize the `canary` to `customer` mode change and first charge.

## Queue

Work top-down after this run is explicitly made ACTIVE. Every item is retained.

### P0 — Finish test-mode readiness

- Resume and exhaust [`Stripe Checkout Readiness`](stripe-checkout-readiness.md):
  dashboard unit tests, browser product flow, root gates, opt-in test-clock E2E,
  and Paul's real test-mode browser Checkout. Synthetic events alone do not
  close it.
- In the official account sandbox, create the same one-product/one-Price
  contract, sandbox Portal configuration, and sandbox webhook destination used
  below. Record sandbox object ids only in ephemeral evidence, not deployment
  config.
- Create a sandbox restricted API key starting from no permissions. Exercise
  every application call and use its request logs to prove the minimum live
  permission set. The test-clock harness uses a separate, test-only key because
  it creates clocks and synthetic subscriptions that production never may.
- Pin and test the Stripe API version used by `stripe@22.1.1`, currently
  `2026-04-22.dahlia`. If the SDK changes before activation, update this plan
  and the webhook destination to the newly tested pinned version.

### P0 — Close production code and disclosure gaps

- Show `Finite Computer Hosted Agent — $200 USD/month`, automatic renewal, no
  trial, tax behavior, cancellation/refund summary, service-delivery summary,
  and support/legal links before the user leaves for Checkout.
- Pin v0 to cards/card wallets in Checkout. Do not grant entitlement from an
  asynchronous payment method unless a later contract handles
  `checkout.session.async_payment_succeeded` and failure states.
- After the tax classification and registrations are confirmed, add and test
  `automatic_tax.enabled = true` for Checkout and the corresponding address/tax
  behavior. Enabling Stripe Tax in the Dashboard alone does not modify an
  API-created Checkout Session.
- Add an inspection-only live-readiness command that reports no secrets and
  checks the Stripe account id/mode, Product/Price name/amount/currency/
  interval/active/tax facts, Portal default configuration, API version, and
  webhook URL/events. Run it with a separate temporary read-only audit key,
  never the production application key. It must never create, update, refund,
  cancel, resend, or delete an object.
- Centralize or mechanically assert the one public live Price id used by
  `infra/nixos/modules/dashboard.nix` and
  `infra/nixos/modules/finite-saas-core.nix`; a mismatch fails CI and deploy
  preflight.
- Add a production support runbook for paid-but-unsynced, duplicate delivery,
  wrong Price, past-due, cancellation, refund, dispute, and key/webhook-secret
  rotation. Repair uses Stripe event redelivery or an authenticated
  reconciliation path, never a direct Core database edit.
- Add monitoring for live webhook non-2xx delivery and a bounded reconciliation
  alert. Do not log Customer email, card data, key material, or webhook bodies.

### P0 — Prove customer data recovery before admission

- Activate and complete Hosted Web Chat Disaster Recovery, including one
  empty-target restore of the real encrypted off-host Recovery Set.
- Confirm the first-admitted-Principal owner-claim posture and the paid cohort's
  stuck-launch boundary. Stripe success never overrides either gate.

### P1 — Configure live Stripe and deploy dark

- Paul performs the exact live Dashboard checklist below under explicit Stripe
  mutation authority and hands off only the named outputs to their declared
  locations.
- Deploy the accepted Dashboard/Core revision with the verified live Price,
  live restricted key, and live endpoint secret, but keep
  `FC_DASHBOARD_RUNTIME_MODE = "canary"` so no customer can start Checkout.
- Run the inspection-only readiness command and the normal production health
  checks. Verify the public webhook URL has valid TLS and no redirect. Do not
  manufacture a live Customer or Subscription as a probe.

### P1 — Activate and request acceptance

- Under a separately named rollout authorization, change the production
  dashboard to `FC_DASHBOARD_RUNTIME_MODE = "customer"`, promote the pinned
  image/config, and verify an unentitled invited account sees payment alongside
  Launch Code while non-invited public admission remains closed.
- Produce the exact Acceptance Request from `docs/runs/README.md`: deployed Git
  revision and image digest, Finite URL, dedicated invited account, Stripe
  member account id, live Price id, expected observation after each Checkout/
  webhook/Core/draft step, stop conditions, rollback boundary, and estimated
  minutes.
- Paul performs the real $200 USD live charge. A coupon, test clock, manual
  Dashboard subscription, synthetic event, or $0 invoice does not satisfy the
  acceptance statement.
- Observe the first invoice/receipt and eventual payout record. Record only
  non-secret ids in the private acceptance evidence; put no customer data in
  this repository.

## Paul's exact Stripe Dashboard checklist

Do this in order. Stop at the first failed check. Stripe maintains separate
sandbox and live objects, Portal configurations, keys, and webhooks, so a green
sandbox is never proof of live configuration.

### 1. Enter the correct merchant account and live mode

- [ ] From the Finite Supply organization switcher, enter the member account
      that legally accepts Finite Computer payments. Do not remain at the
      organization container and do not select a sibling account.
- [ ] Open **Settings → Account details** and record the member `acct_...` id as
      non-secret launch evidence.
- [ ] Switch out of the sandbox/test-data view into **live mode**. Re-check the
      account name and `acct_...` id after switching.
- [ ] Confirm your role can edit business details, products, Billing/Portal,
      payment methods, Workbench webhooks, and restricted API keys. If any
      control is read-only, stop and ask the account administrator for the
      narrow role; do not share a login.

### 2. Activate and secure the account

- [ ] Complete **Settings → Business → Account onboarding** until Stripe shows
      no outstanding business, representative, ownership, identity, website,
      product, or bank requirements for accepting live payments.
- [ ] In **Settings → Payouts**, verify the actual USD bank account and choose
      the intended payout schedule. The proposed v0 default is daily as funds
      become available.
- [ ] In your personal settings, enable phishing-resistant 2FA with a passkey
      or security key. Confirm every live-account administrator has 2FA.
- [ ] In **Settings → Public details**, set and review:
      - public business name: `Finite Supply` unless the legal owner approves a
        more precise customer-facing name;
      - website: `https://finite.computer`;
      - statement descriptor: proposed `FINITE COMPUTER`;
      - the real monitored support email, URL, phone, and address supplied by
        Paul.
- [ ] In **Settings → Branding**, set the approved Finite logo/icon, colors,
      and font. Preview Checkout, receipts, invoices, and Portal branding.
- [ ] In **Settings → Communication preferences**, enable operator alerts for
      successful charges, disputes, payouts/account health, and critical SMS
      alerts. These are operator notifications, not customer emails.

### 3. Confirm the website and policy gate

- [ ] At `https://finite.computer`, verify a customer can see what the hosted
      agent includes, `$200 USD/month`, automatic renewal, whether tax is added,
      delivery timing, cancellation, refund, privacy, Terms, and more than one
      usable support/contact method.
- [ ] Verify the statement descriptor shown on the site matches the Dashboard.
- [ ] Stop if any policy URL is missing or placeholder text remains. Stripe's
      website checklist treats product, currency, contact, refund,
      cancellation, delivery, privacy, promotion, and HTTPS information as
      go-live requirements.

### 4. Set the live tax posture

- [ ] Obtain the owner/tax-adviser decision for business-use, personal-use, or
      mixed SaaS. Do not guess from the selected Dashboard row.
- [ ] In **Tax → Settings**, confirm the head-office address and set the
      approved preset product tax code.
- [ ] Keep the confirmed default Price tax behavior at **Automatic**. For this
      USD Price, Stripe treats Automatic as exclusive and adds applicable tax
      on top of $200.
- [ ] In **Tax → Registrations**, add only jurisdictions where Finite is
      actually registered to collect. A Stripe monitoring suggestion is not a
      registration.
- [ ] Enable live automatic tax only after the accepted Checkout revision sends
      `automatic_tax.enabled = true`. Confirm the matching Product below has the
      approved tax code.

### 5. Reuse or create exactly one live Product and Price

- [ ] In **More → Product catalog** in live mode, search for
      `Finite Computer Hosted Agent`.
- [ ] If an active Product already exists, open it and verify its `acct_...`
      context, name, description, statement descriptor, tax code, and every
      Price. Reuse an existing Price only if all facts below match. Otherwise
      leave it untouched and escalate before creating a duplicate.
- [ ] If no exact Product exists, click **+ Add product** and enter:
      - name: `Finite Computer Hosted Agent`;
      - description: `One hosted Finite Computer agent, billed monthly.`;
      - statement descriptor: `FINITE COMPUTER` if approved;
      - approved SaaS tax code;
      - no shipping or physical-good fields.
- [ ] Add one Price with:
      - pricing model: **Flat-rate**;
      - type: **Recurring**;
      - amount: **200.00 USD**;
      - billing period: **Monthly**;
      - tax behavior: inherit the account default, **Automatic** (exclusive for
        USD);
      - active: **On**;
      - no trial, tiers, usage meter, package quantity, additional currency,
        or setup fee.
- [ ] Make that Price the Product's default Price.
- [ ] Record the resulting `prod_...` and `price_...` with the `acct_...` id in
      private handoff evidence. The `price_...` is the only Stripe value copied
      into public Nix configuration. Do not create a Payment Link.

### 6. Restrict live payment methods

- [ ] In **Settings → Payment methods**, select the configuration used by
      Finite's Stripe-hosted Checkout.
- [ ] Enable **Cards** and card wallets/Link that settle through the card flow.
- [ ] Disable ACH, bank debits/transfers, vouchers, cash-based methods, and any
      other asynchronous method for Billing v0. Do not depend on this switch as
      the only guard; the accepted Checkout code also pins the contract.
- [ ] Confirm Checkout does not offer Buy Now Pay Later or a payment method
      whose completion can occur after `checkout.session.completed`.

### 7. Configure the live Customer Portal

- [ ] Open **Settings → Billing → Customer portal** while still in live mode.
      Do not assume the sandbox Portal configuration copied over.
- [ ] Save this default API Portal configuration:
      - payment-method updates: **On**;
      - invoice history: **On**;
      - billing name, email, billing address, and phone edits: **On**; the email
        is a billing contact only and does not change WorkOS identity;
      - shipping address: **Off**;
      - tax ID editing: **On only when Stripe Tax is active**, otherwise Off;
      - plan switching: **Off**;
      - quantity changes: **Off**;
      - Portal promotion codes: **Off**;
      - cancellation: **On, at end of billing period**;
      - cancellation reason collection: **On**;
      - retention coupons: **Off**;
      - headline: `Manage your Finite Computer billing.`;
      - default redirect: `https://finite.computer/dashboard`;
      - Terms link: Paul's approved public Terms URL.
- [ ] Preview the configuration in the sandbox. The application creates
      authenticated Portal sessions, so do not publish or rely on a shareable
      no-code Portal login link.

### 8. Configure customer emails and failed-payment policy

- [ ] In **Settings → Customer emails / Billing email settings**, enable
      receipts for successful payments and refunds.
- [ ] Enable failed-card-payment emails and expiring-card notifications, with
      the manage-billing link directed to Stripe's Customer Portal.
- [ ] Leave trial-ending emails off because Billing v0 has no trial.
- [ ] In **Billing → Revenue recovery → Retries**, enable **Smart Retries**.
- [ ] After the retry window, choose **leave the subscription past due** for
      human review. Do not auto-delete, purge, or retire a runtime.
- [ ] Keep monthly renewal reminders off unless the published policy or a
      customer's jurisdiction requires them; if enabled, record the chosen
      notice interval in `billing-v0.md`.

### 9. Create the live restricted API key

Do this only after the sandbox restricted-key flow passed and its request logs
proved the permissions.

- [ ] Open **Workbench → API keys**, remain in the verified member account and
      live mode, and click **Create restricted key**.
- [ ] Name it `finite-dashboard-production`; start from zero permissions.
- [ ] Copy the permission set proven in sandbox. The expected application calls
      are:
      - Customers: **Write** (`POST /v1/customers`);
      - Checkout Sessions: **Write** (`POST /v1/checkout/sessions`);
      - Billing Portal Sessions: **Write**
        (`POST /v1/billing_portal/sessions`);
      - Subscriptions: **Read** (`GET /v1/subscriptions/:id`).
      Add no Balance, Payout, Refund, Dispute, PaymentIntent, invoice-mutation,
      product-mutation, or Connect permission. If sandbox logs required an
      additional read permission, record the exact request and add only that
      permission.
- [ ] If finite-lat-1's stable outbound IP has been re-verified read-only, add
      it as the key's sole IP allowlist entry. Do not copy a historical IP
      without verification.
- [ ] Create the key, complete 2FA, and transfer its one-time value directly to
      the production secret operator. Store it only as `STRIPE_SECRET_KEY` in
      root-owned `/etc/finite/dashboard.env` and the team secret vault.
- [ ] In the key's Dashboard note, record the secret location by name, not the
      value. Never paste the key into this repository, a task, Slack, email, or
      an issue.

### 10. Create the live webhook destination

- [ ] Open **Workbench → Webhooks → Create an event destination** in the same
      verified member account and live mode.
- [ ] Select **Your account** (not Connected accounts and not an organization-
      wide destination), **snapshot events**, and API version
      `2026-04-22.dahlia` unless the accepted implementation records a newer
      tested pin.
- [ ] Select only:
      - `checkout.session.completed`;
      - `customer.subscription.created`;
      - `customer.subscription.updated`;
      - `customer.subscription.deleted`.
- [ ] Select **Webhook endpoint** and enter:
      - URL: `https://finite.computer/api/stripe/webhook`;
      - description: `finite-dashboard production billing v0`.
- [ ] Create it, reveal the endpoint signing secret, and transfer the value
      directly to the production secret operator. Store it only as
      `STRIPE_WEBHOOK_SECRET` in root-owned `/etc/finite/dashboard.env` and the
      team secret vault.
- [ ] Do not reuse a Stripe CLI, sandbox, sibling endpoint, or organization
      endpoint `whsec_...`; each destination has a different secret.
- [ ] Confirm the destination summary shows the exact account scope, URL, API
      version, and four event types. Do not manually create a live Customer or
      Subscription merely to make an event appear.

### 11. Create a temporary read-only readiness key

- [ ] In live **Workbench → API keys**, create a second restricted key named
      `finite-billing-readiness-audit-202607` with only the read permissions
      needed for Account, Products, Prices, Customer Portal configurations,
      and webhook endpoints/event destinations.
- [ ] Transfer it directly to the operator running the inspection-only
      readiness command. Keep it in that process environment only; do not add
      it to `/etc/finite/dashboard.env` or a developer `.env` file.
- [ ] Run the readiness command against the expected `acct_...` id and save its
      secret-free report in private launch evidence.
- [ ] Expire the audit key immediately after the report passes. If it fails,
      keep production in canary mode, expire the key, and repair the named
      Dashboard fact under separate mutation authority.

### 12. Hand off configuration without leaking it

| Output | Destination | Public? |
| --- | --- | --- |
| Merchant `acct_...` id | private launch evidence and readiness assertion | Non-secret, but keep customer evidence private |
| Product `prod_...` id | private launch evidence | Non-secret |
| Standard `price_...` id | both Nix service configs through reviewed code | Yes |
| Restricted `rk_live_...` value | vault + `/etc/finite/dashboard.env` as `STRIPE_SECRET_KEY` | **Secret** |
| Endpoint `whsec_...` value | vault + `/etc/finite/dashboard.env` as `STRIPE_WEBHOOK_SECRET` | **Secret** |
| Temporary readiness key | operator process only, then expire | **Secret** |
| Portal settings/API version/events | run evidence and inspection-only preflight | No secret values |

- [ ] Have the deploy operator confirm the two secret names are present without
      printing their values or shell history.
- [ ] Have engineering replace/verify the Price id in both Dashboard and Core
      configuration. Do not point one service at sandbox and the other at live.
- [ ] Keep production in canary mode until the dark deploy, readiness preflight,
      recovery gate, and explicit activation authorization all pass.

## Evaluation design

### Automated gates

- `just web-check`
- `cd finitecomputer-v2/apps/dashboard && npm run test:browser`
- `just stripe-billing-clock` with an explicitly supplied sandbox harness key
- focused tests for cards-only Checkout, automatic tax, price disclosure,
  complete/incomplete configuration, idempotent retries, signed/invalid
  webhooks, wrong Price, duplicate delivery, stale delivery, inactive status,
  cancellation-at-period-end, and saved/lost draft return
- Core workspace tests for Price matching, entitlement creation, inactive
  blocking without teardown, and stale/non-current subscription events
- inspection-only live readiness command with an `acct_...` allowlist and no
  mutation capability
- Nix evaluation/build proving Dashboard and Core receive the same public live
  Price id while secrets remain host-only

### Human gates

1. Paul's existing test-mode browser acceptance from the paused readiness run.
2. A non-developer verifies the disclosures, Checkout, cancel path, receipt,
   and Portal in the sandbox.
3. The separately activated disaster-recovery run proves the real off-host
   empty-target restore.
4. Paul executes the final live Acceptance Request and pays the real $200 USD.
5. An operator observes a successful Stripe payment/invoice, 2xx webhook
   delivery, active matching Core billing row, one consumed entitlement, and
   one agent creation request without changing durable state by hand.

## Stop and rollback boundaries

Stop before charge if the account id/mode, Price facts, tax posture, payment
methods, API version, endpoint scope, or secret provenance is ambiguous; if
public policies are missing; if the recovery gate has not passed; or if any
secret appears in a transcript.

Stop after charge if Checkout succeeds but the live event is non-2xx, the
Subscription does not contain the exact live Price, metadata lacks the
canonical Core organization id, Core is inactive or points at another
Subscription, the draft cannot resume, or more than one creation request or
Customer appears. Capture read-only Stripe request/event ids and application
logs, leave the customer's durable state untouched, and use the support
runbook. Never repair with a Core database edit.

The launch rollback is to return the dashboard to `canary`, promote the last
known-good dashboard image/config, and stop new Checkouts. Do not delete the
live webhook, Price, Customer, Subscription, invoice, payment, runtime, or
recovery material as a rollback shortcut. Cancellation/refund, if required,
follows the published policy under separate explicit authority.

## Official Stripe references

- [Set up and verify a live account](https://docs.stripe.com/get-started/account/set-up)
- [Account checklist](https://docs.stripe.com/get-started/account/checklist)
- [Integration go-live checklist](https://docs.stripe.com/get-started/checklist/go-live)
- [Website checklist](https://docs.stripe.com/get-started/checklist/website)
- [Manage Products and Prices](https://docs.stripe.com/products-prices/manage-prices)
- [Configure the Customer Portal](https://docs.stripe.com/customer-management/configure-portal)
- [Receive Stripe events with webhooks](https://docs.stripe.com/webhooks)
- [Restricted API keys](https://docs.stripe.com/keys/restricted-api-keys)
- [Stripe API key security](https://docs.stripe.com/keys-best-practices)
- [Set up Stripe Tax](https://docs.stripe.com/tax/set-up)
- [Customer emails and failed payments](https://docs.stripe.com/billing/revenue-recovery/customer-emails)
- [Smart Retries](https://docs.stripe.com/billing/revenue-recovery/smart-retries)

## Governing repository documents

- [`docs/runs/README.md`](README.md)
- [`docs/runs/stripe-checkout-readiness.md`](stripe-checkout-readiness.md)
- [`docs/runs/hosted-web-chat-disaster-recovery.md`](hosted-web-chat-disaster-recovery.md)
- [`finitecomputer-v2/docs/billing-v0.md`](../../finitecomputer-v2/docs/billing-v0.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`infra/nixos/README.md`](../../infra/nixos/README.md)
