import assert from "node:assert/strict";
import test from "node:test";

import { ensureStripeCheckoutCustomer } from "./stripe-checkout";

test("fresh Checkout uses Core's linked organization and idempotent Stripe customer", async () => {
  const calls: Array<{ parameters: unknown; options: unknown }> = [];
  const stripe = {
    customers: {
      async create(parameters: unknown, options: unknown) {
        calls.push({ parameters, options });
        return { id: "cus_new" };
      },
    },
  };

  const resolved = await ensureStripeCheckoutCustomer({
    stripe,
    existingStripeCustomerId: "",
    provisionalCustomerOrgId: "org_read_side_placeholder",
    customerOrgName: "Paul",
    email: "user@example.test",
    workosUserId: "user_workos",
    async linkCustomer(stripeCustomerId) {
      assert.equal(stripeCustomerId, "cus_new");
      return { customer_org_id: "org_core_canonical" };
    },
  });

  assert.deepEqual(resolved, {
    stripeCustomerId: "cus_new",
    customerOrgId: "org_core_canonical",
  });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0]?.parameters, {
    email: "user@example.test",
    name: "Paul",
    metadata: { finite_workos_user_id: "user_workos" },
  });
  assert.match(
    String((calls[0]?.options as { idempotencyKey?: string }).idempotencyKey),
    /^finite-customer:[0-9a-f]{64}$/u
  );
  assert.equal(JSON.stringify(calls[0]).includes("org_read_side_placeholder"), false);
});

test("an already linked customer stays on its canonical Core organization", async () => {
  let created = false;
  let linked = false;
  const resolved = await ensureStripeCheckoutCustomer({
    stripe: {
      customers: {
        async create() {
          created = true;
          return { id: "unexpected" };
        },
      },
    },
    existingStripeCustomerId: "cus_existing",
    provisionalCustomerOrgId: "org_core_existing",
    customerOrgName: "Paul",
    email: "user@example.test",
    workosUserId: "user_workos",
    async linkCustomer() {
      linked = true;
      return { customer_org_id: "unexpected" };
    },
  });

  assert.deepEqual(resolved, {
    stripeCustomerId: "cus_existing",
    customerOrgId: "org_core_existing",
  });
  assert.equal(created, false);
  assert.equal(linked, false);
});
