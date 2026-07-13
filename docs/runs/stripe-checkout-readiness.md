# Stripe Checkout Readiness

Status: PAUSED (2026-07-13 — Paul explicitly activated Stripe Production
Activation for build, test, and shipment)

Sequence note: On 2026-07-13, Paul explicitly accepted the shipped Hosted Web
Chat product-continuity outcome, directed that run closed and deleted, and
moved its remaining empty-target recovery proof into the no-authority proposed
run [Hosted Web Chat Disaster Recovery](hosted-web-chat-disaster-recovery.md).
This run briefly resumed, then Paul explicitly activated
[Stripe Production Activation](stripe-production-activation.md) later that day.
Its queue and human test-mode Checkout acceptance remain prerequisites in that
ACTIVE run.

Owner: Paul

Opened: 2026-07-11

Expires: 2026-07-25

Acceptance: An invited test account chooses payment without a Launch Code,
completes one real Stripe test-mode Checkout, returns to the saved agent draft,
and the signed webhook synchronizes one standard paid entitlement into Core so
that draft can create one fresh agent. Paul performs the final browser step;
automated webhook or test-clock evidence does not claim it passed.

## Authority and boundaries

This run hardens the already-settled Billing v0 path after the Electron parity
engineering gate. It may change repository code, tests, and local harnesses.
It does not create or modify Stripe objects, WorkOS state, production secrets,
webhook endpoints, Portal configuration, deployed services, or customer
admission without separate explicit authorization.

Passing this run is not permission to admit customers. The paid invited-cohort
backup/restore, untrusted owner-claim, and stuck-launch gates remain separate.

## Queue

Work top-down. Every retained item is required.

### P0 — Fail closed and keep one billing identity

- Treat Checkout as configured only when the secret key, standard recurring
  Price, webhook signing secret, and usable public return origin are all
  present.
- Make Stripe customer and Checkout creation idempotent for the stable signed
  onboarding attempt, with endpoint-scoped keys.
- Use the canonical organization id returned by Core after Stripe-customer
  linking for Checkout and Subscription metadata. Never stamp a synthesized
  read-side organization id into the paid ledger.
- Add focused negative, retry, and fresh-direct-Checkout regressions.

### P0 — Repair the real webhook-to-Core harness

- Authenticate customer Core routes with the existing WorkOS fixture JWT.
  Keep webhook sync, Runner, and Finite Private usage credentials distinct and
  route-scoped.
- Exercise missing and invalid signatures, `checkout.session.completed`,
  active entitlement, inactive creation blocking without teardown, and stale
  event ordering through the real webhook handler and Core store.
- Keep the Stripe test-clock harness opt-in and secret-safe. It may create test
  objects only when a caller explicitly supplies test credentials.

### P1 — Browser and handoff

- Prove incomplete Stripe configuration hides payment while a complete
  non-canary configuration offers payment alongside Launch Code.
- Preserve the sealed onboarding draft across Checkout return, wait honestly
  for webhook sync, complete it once entitlement arrives, and recover safely
  when the draft or return state is lost.
- Run the dashboard, Core, and root gates relevant to Billing v0.
- Record live/test Stripe configuration, deployment, and signing needs as true
  external handoffs rather than weakening readiness or fabricating evidence.

### P1 — Acceptance Request

- Deploy the accepted revision under separate production-mutation authority,
  then produce the exact Acceptance Request defined in `README.md`: revision,
  URL, invited test account, expected observation after each Checkout/webhook/
  draft-resume step, stop conditions, and estimated minutes.
- Paul completes that request and the acceptance statement at the top of this
  run. Synthetic events, unit tests, and a test clock do not claim acceptance.

## Out of scope

- Production/customer admission, public landing changes, or removing canary
  mode.
- Creating Prices, webhooks, Portal configuration, customers, subscriptions,
  or deployments without explicit external-mutation authorization.
- Backup/restore implementation, owner-claim redesign, stuck-launch cleanup,
  cancellations/refunds, retention, or multi-agent plan changes.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`finitecomputer-v2/CONTEXT.md`](../../finitecomputer-v2/CONTEXT.md)
- [`finitecomputer-v2/docs/billing-v0.md`](../../finitecomputer-v2/docs/billing-v0.md)
- [`finitecomputer-v2/docs/identity-boundary-v1.md`](../../finitecomputer-v2/docs/identity-boundary-v1.md)
- [`finitecomputer-v2/docs/vertical-slice-v1-prd.md`](../../finitecomputer-v2/docs/vertical-slice-v1-prd.md)
- [`infra/README.md`](../../infra/README.md)
