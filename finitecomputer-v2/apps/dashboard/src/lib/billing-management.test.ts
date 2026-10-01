import { redirect } from "next/navigation";
import assert from "node:assert/strict";
import { test } from "node:test";
import { billingManagementResult } from "./billing-management";

const unavailable = { error: "Billing is unavailable right now. Please try again shortly." };
const currentAccount = {
  stripe_customer_id: " cus_test ",
  stripe_subscription_id: "sub_current",
  subscription_status: "active" as const,
};

for (const [stage, failure] of [
  ["checkout", "Payment is unavailable right now."],
  ["core", "Core request failed: private response"],
  ["portal", "Stripe provider error: private response"],
  ["checkout", "Stripe did not return a Checkout URL."],
] as const) {
  test(`Manage billing returns safe feedback for ${stage}: ${failure}`, async () => {
    const result = await billingManagementResult({
      loadBillingAccount: async () => {
        if (stage === "core") throw new Error(failure);
        return stage === "portal" ? currentAccount : null;
      },
      checkoutDestination: async () => { throw new Error(failure); },
      portalDestination: async () => { throw new Error(failure); },
    });
    assert.deepEqual(result, unavailable);
  });
}

test("missing customer uses checkout and preserves its successful destination", async () => {
  assert.deepEqual(await billingManagementResult({
    loadBillingAccount: async () => ({ ...currentAccount, stripe_customer_id: "  " }),
    checkoutDestination: async () => "https://checkout.example/session",
    portalDestination: async () => { assert.fail("must not open portal"); },
  }), { destination: "https://checkout.example/session" });
});

for (const subscription_status of [
  "incomplete", "trialing", "active", "past_due", "unpaid", "paused", null, undefined,
] as const) {
  test(`existing subscription uses portal without duplicate checkout: ${subscription_status}`, async () => {
    assert.deepEqual(await billingManagementResult({
      loadBillingAccount: async () => ({ ...currentAccount, subscription_status }),
      checkoutDestination: async () => { assert.fail("must not start checkout"); },
      portalDestination: async (id) => {
        assert.equal(id, "cus_test");
        return "https://billing.example/session";
      },
    }), { destination: "https://billing.example/session" });
  });
}

for (const subscription_status of ["canceled", "incomplete_expired"] as const) {
  test(`retained customer with ${subscription_status} subscription uses paid Checkout`, async () => {
    assert.deepEqual(await billingManagementResult({
      loadBillingAccount: async () => ({ ...currentAccount, subscription_status }),
      checkoutDestination: async () => "https://checkout.example/session",
      portalDestination: async () => { assert.fail("terminal subscription must not open portal"); },
    }), { destination: "https://checkout.example/session" });
  });
}

for (const stripe_subscription_id of [null, undefined, "", "   "]) {
  test(`retained customer without subscription uses paid Checkout: ${stripe_subscription_id}`, async () => {
    assert.deepEqual(await billingManagementResult({
      loadBillingAccount: async () => ({ ...currentAccount, stripe_subscription_id }),
      checkoutDestination: async () => "https://checkout.example/session",
      portalDestination: async () => { assert.fail("missing subscription must not open portal"); },
    }), { destination: "https://checkout.example/session" });
  });
}

test("Core failure never starts checkout or portal", async () => {
  assert.deepEqual(await billingManagementResult({
    loadBillingAccount: async () => { throw new Error("Core unavailable"); },
    checkoutDestination: async () => { assert.fail("must not start checkout"); },
    portalDestination: async () => { assert.fail("must not open portal"); },
  }), unavailable);
});

for (const stage of ["core", "checkout", "portal"] as const) {
  test(`preserves Next.js redirects from ${stage}`, async () => {
    await assert.rejects(billingManagementResult({
      loadBillingAccount: async () => {
        if (stage === "core") redirect("/sign-in");
        return stage === "portal" ? currentAccount : null;
      },
      checkoutDestination: async () => redirect("https://checkout.example/session"),
      portalDestination: async () => redirect("https://billing.example/session"),
    }), (error: unknown) => error instanceof Error && error.message === "NEXT_REDIRECT");
  });
}
