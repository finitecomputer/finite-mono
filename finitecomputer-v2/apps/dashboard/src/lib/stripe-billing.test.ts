import assert from "node:assert/strict";
import test from "node:test";

import {
  billingSubscriptionShouldUsePortal,
  stripeBillingStatus,
  stripeDashboardReturnUrl,
} from "./stripe-billing";

test("stripeBillingStatus requires secret key and standard price", () => {
  assert.deepEqual(stripeBillingStatus({}), {
    configured: false,
    missing: ["STRIPE_SECRET_KEY", "STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID"],
  });

  assert.deepEqual(
    stripeBillingStatus({
      STRIPE_SECRET_KEY: "secret",
      STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: "price_standard",
    }),
    {
      configured: true,
      missing: [],
    }
  );
});

test("stripeDashboardReturnUrl uses the public app URL", () => {
  const previousNextPublicAppUrl = process.env.NEXT_PUBLIC_APP_URL;
  const previousDashboardPublicUrl = process.env.FC_DASHBOARD_PUBLIC_URL;
  const previousDashboardBaseUrl = process.env.FC_DASHBOARD_BASE_URL;

  process.env.NEXT_PUBLIC_APP_URL = "https://finite.computer";
  delete process.env.FC_DASHBOARD_PUBLIC_URL;
  delete process.env.FC_DASHBOARD_BASE_URL;

  try {
    assert.equal(
      stripeDashboardReturnUrl("/dashboard?billing=success"),
      "https://finite.computer/dashboard?billing=success"
    );
  } finally {
    if (previousNextPublicAppUrl === undefined) {
      delete process.env.NEXT_PUBLIC_APP_URL;
    } else {
      process.env.NEXT_PUBLIC_APP_URL = previousNextPublicAppUrl;
    }
    if (previousDashboardPublicUrl === undefined) {
      delete process.env.FC_DASHBOARD_PUBLIC_URL;
    } else {
      process.env.FC_DASHBOARD_PUBLIC_URL = previousDashboardPublicUrl;
    }
    if (previousDashboardBaseUrl === undefined) {
      delete process.env.FC_DASHBOARD_BASE_URL;
    } else {
      process.env.FC_DASHBOARD_BASE_URL = previousDashboardBaseUrl;
    }
  }
});

test("stripeDashboardReturnUrl accepts the deployed dashboard base URL env", () => {
  const previousNextPublicAppUrl = process.env.NEXT_PUBLIC_APP_URL;
  const previousDashboardPublicUrl = process.env.FC_DASHBOARD_PUBLIC_URL;
  const previousDashboardBaseUrl = process.env.FC_DASHBOARD_BASE_URL;

  delete process.env.NEXT_PUBLIC_APP_URL;
  delete process.env.FC_DASHBOARD_PUBLIC_URL;
  process.env.FC_DASHBOARD_BASE_URL = "https://finite.computer";

  try {
    assert.equal(stripeDashboardReturnUrl("/dashboard"), "https://finite.computer/dashboard");
  } finally {
    if (previousNextPublicAppUrl === undefined) {
      delete process.env.NEXT_PUBLIC_APP_URL;
    } else {
      process.env.NEXT_PUBLIC_APP_URL = previousNextPublicAppUrl;
    }
    if (previousDashboardPublicUrl === undefined) {
      delete process.env.FC_DASHBOARD_PUBLIC_URL;
    } else {
      process.env.FC_DASHBOARD_PUBLIC_URL = previousDashboardPublicUrl;
    }
    if (previousDashboardBaseUrl === undefined) {
      delete process.env.FC_DASHBOARD_BASE_URL;
    } else {
      process.env.FC_DASHBOARD_BASE_URL = previousDashboardBaseUrl;
    }
  }
});

test("stripeDashboardReturnUrl fails when no public dashboard URL is configured", () => {
  const previousNextPublicAppUrl = process.env.NEXT_PUBLIC_APP_URL;
  const previousDashboardPublicUrl = process.env.FC_DASHBOARD_PUBLIC_URL;
  const previousDashboardBaseUrl = process.env.FC_DASHBOARD_BASE_URL;

  delete process.env.NEXT_PUBLIC_APP_URL;
  delete process.env.FC_DASHBOARD_PUBLIC_URL;
  delete process.env.FC_DASHBOARD_BASE_URL;

  try {
    assert.throws(
      () => stripeDashboardReturnUrl("/dashboard"),
      /Stripe return URL requires/
    );
  } finally {
    if (previousNextPublicAppUrl === undefined) {
      delete process.env.NEXT_PUBLIC_APP_URL;
    } else {
      process.env.NEXT_PUBLIC_APP_URL = previousNextPublicAppUrl;
    }
    if (previousDashboardPublicUrl === undefined) {
      delete process.env.FC_DASHBOARD_PUBLIC_URL;
    } else {
      process.env.FC_DASHBOARD_PUBLIC_URL = previousDashboardPublicUrl;
    }
    if (previousDashboardBaseUrl === undefined) {
      delete process.env.FC_DASHBOARD_BASE_URL;
    } else {
      process.env.FC_DASHBOARD_BASE_URL = previousDashboardBaseUrl;
    }
  }
});

test("stripeDashboardReturnUrl rejects localhost in production", () => {
  const previousNextPublicAppUrl = process.env.NEXT_PUBLIC_APP_URL;
  const previousDashboardPublicUrl = process.env.FC_DASHBOARD_PUBLIC_URL;
  const previousDashboardBaseUrl = process.env.FC_DASHBOARD_BASE_URL;
  const previousNodeEnv = process.env.NODE_ENV;

  process.env.NEXT_PUBLIC_APP_URL = "http://localhost:3000";
  delete process.env.FC_DASHBOARD_PUBLIC_URL;
  delete process.env.FC_DASHBOARD_BASE_URL;
  process.env.NODE_ENV = "production";

  try {
    assert.throws(
      () => stripeDashboardReturnUrl("/dashboard"),
      /must not point at localhost/
    );
  } finally {
    if (previousNextPublicAppUrl === undefined) {
      delete process.env.NEXT_PUBLIC_APP_URL;
    } else {
      process.env.NEXT_PUBLIC_APP_URL = previousNextPublicAppUrl;
    }
    if (previousDashboardPublicUrl === undefined) {
      delete process.env.FC_DASHBOARD_PUBLIC_URL;
    } else {
      process.env.FC_DASHBOARD_PUBLIC_URL = previousDashboardPublicUrl;
    }
    if (previousDashboardBaseUrl === undefined) {
      delete process.env.FC_DASHBOARD_BASE_URL;
    } else {
      process.env.FC_DASHBOARD_BASE_URL = previousDashboardBaseUrl;
    }
    if (previousNodeEnv === undefined) {
      delete process.env.NODE_ENV;
    } else {
      process.env.NODE_ENV = previousNodeEnv;
    }
  }
});

test("billingSubscriptionShouldUsePortal prevents duplicate checkout for current subscriptions", () => {
  for (const status of [
    "incomplete",
    "trialing",
    "active",
    "past_due",
    "unpaid",
    "paused",
    null,
    undefined,
  ]) {
    assert.equal(billingSubscriptionShouldUsePortal(status, "sub_current"), true, String(status));
  }

  assert.equal(billingSubscriptionShouldUsePortal("canceled", "sub_old"), false);
  assert.equal(billingSubscriptionShouldUsePortal("incomplete_expired", "sub_old"), false);
  assert.equal(billingSubscriptionShouldUsePortal("active", ""), false);
  assert.equal(billingSubscriptionShouldUsePortal("active", "   "), false);
  assert.equal(billingSubscriptionShouldUsePortal("active", null), false);
});
