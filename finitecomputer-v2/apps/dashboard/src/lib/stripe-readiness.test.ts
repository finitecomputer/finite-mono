import assert from "node:assert/strict";
import test from "node:test";

import { evaluateStripeReadiness, type StripeReadinessSnapshot } from "./stripe-readiness";

const expected = {
  accountId: "acct_finite",
  priceId: "price_finite",
  apiVersion: "2026-04-22.dahlia",
};

test("production readiness accepts the exact live billing contract", () => {
  const result = evaluateStripeReadiness(readySnapshot(), expected);
  assert.equal(result.ready, true, result.checks.filter((check) => !check.passed).map((check) => check.name).join(", "));
});

test("production readiness accepts the existing immutable live webhook version", () => {
  const snapshot = readySnapshot();
  snapshot.webhook!.snapshotApiVersion = "2024-06-20";

  const result = evaluateStripeReadiness(snapshot, expected);
  assert.equal(
    result.ready,
    true,
    result.checks.filter((check) => !check.passed).map((check) => check.name).join(", ")
  );
});

test("production readiness fails closed on account, Price, Portal, webhook, and tax drift", () => {
  const snapshot = readySnapshot();
  snapshot.account.id = "acct_sibling";
  snapshot.price.id = "price_test";
  snapshot.taxDefaultBehavior = "inclusive";
  snapshot.portal!.subscriptionCancelMode = "immediately";
  snapshot.webhook!.eventsFrom = ["@organization_members"];
  snapshot.webhook!.snapshotApiVersion = "2023-10-16";
  snapshot.webhook!.enabledEvents.push("invoice.paid");

  const result = evaluateStripeReadiness(snapshot, expected);
  assert.equal(result.ready, false);
  assert.deepEqual(
    result.checks.filter((check) => !check.passed).map((check) => check.name),
    [
      "account.id",
      "price.id",
      "price.tax_behavior",
      "portal.period_end_cancel",
      "webhook.scope",
      "webhook.api_version",
      "webhook.events",
    ]
  );
});

function readySnapshot(): StripeReadinessSnapshot {
  return {
    account: {
      id: "acct_finite",
      chargesEnabled: true,
      payoutsEnabled: true,
      detailsSubmitted: true,
    },
    product: {
      active: true,
      livemode: true,
      name: "Finite Computer Hosted Agent",
      taxCode: "txcd_10103001",
    },
    price: {
      id: "price_finite",
      active: true,
      livemode: true,
      currency: "usd",
      unitAmount: 20_000,
      type: "recurring",
      interval: "month",
      intervalCount: 1,
      taxBehavior: "unspecified",
    },
    taxDefaultBehavior: "inferred_by_currency",
    portal: {
      active: true,
      livemode: true,
      isDefault: true,
      defaultReturnUrl: "https://finite.computer/dashboard",
      termsSource: "portal",
      privacySource: "portal",
      customerUpdateEnabled: true,
      customerAllowedUpdates: ["address", "email", "name", "phone", "tax_id"],
      invoiceHistoryEnabled: true,
      paymentMethodUpdateEnabled: true,
      subscriptionCancelEnabled: true,
      subscriptionCancelMode: "at_period_end",
      cancellationReasonEnabled: true,
      subscriptionUpdateEnabled: false,
      loginPageEnabled: false,
    },
    webhook: {
      livemode: true,
      status: "enabled",
      type: "webhook_endpoint",
      eventPayload: "snapshot",
      eventsFrom: ["@self"],
      enabledEvents: [
        "checkout.session.completed",
        "customer.subscription.created",
        "customer.subscription.updated",
        "customer.subscription.deleted",
      ],
      snapshotApiVersion: "2026-04-22.dahlia",
      url: "https://finite.computer/api/stripe/webhook",
    },
  };
}
