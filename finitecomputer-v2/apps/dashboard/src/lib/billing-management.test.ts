import { redirect } from "next/navigation";
import assert from "node:assert/strict";
import { test } from "node:test";
import { billingManagementResult } from "./billing-management";

const unavailable = { error: "Billing is unavailable right now. Please try again shortly." };

for (const [stage, failure] of [
  ["checkout", "Payment is unavailable right now."],
  ["core", "Core request failed: private response"],
  ["portal", "Stripe provider error: private response"],
  ["checkout", "Stripe did not return a Checkout URL."],
] as const) {
  test(`Manage billing returns safe feedback for ${stage}: ${failure}`, async () => {
    const result = await billingManagementResult({
      loadCustomerId: async () => {
        if (stage === "core") throw new Error(failure);
        return stage === "portal" ? "cus_test" : null;
      },
      checkoutDestination: async () => { throw new Error(failure); },
      portalDestination: async () => { throw new Error(failure); },
    });
    assert.deepEqual(result, unavailable);
  });
}

test("missing customer uses checkout and preserves its successful destination", async () => {
  assert.deepEqual(await billingManagementResult({
    loadCustomerId: async () => "  ",
    checkoutDestination: async () => "https://checkout.example/session",
    portalDestination: async () => { assert.fail("must not open portal"); },
  }), { destination: "https://checkout.example/session" });
});

test("existing customer uses portal and preserves its successful destination", async () => {
  assert.deepEqual(await billingManagementResult({
    loadCustomerId: async () => " cus_test ",
    checkoutDestination: async () => { assert.fail("must not start checkout"); },
    portalDestination: async (id) => {
      assert.equal(id, "cus_test");
      return "https://billing.example/session";
    },
  }), { destination: "https://billing.example/session" });
});

test("Core failure never starts checkout or portal", async () => {
  assert.deepEqual(await billingManagementResult({
    loadCustomerId: async () => { throw new Error("Core unavailable"); },
    checkoutDestination: async () => { assert.fail("must not start checkout"); },
    portalDestination: async () => { assert.fail("must not open portal"); },
  }), unavailable);
});

for (const stage of ["core", "checkout", "portal"] as const) {
  test(`preserves Next.js redirects from ${stage}`, async () => {
    await assert.rejects(billingManagementResult({
      loadCustomerId: async () => {
        if (stage === "core") redirect("/sign-in");
        return stage === "portal" ? "cus_test" : null;
      },
      checkoutDestination: async () => redirect("https://checkout.example/session"),
      portalDestination: async () => redirect("https://billing.example/session"),
    }), (error: unknown) => error instanceof Error && error.message === "NEXT_REDIRECT");
  });
}
