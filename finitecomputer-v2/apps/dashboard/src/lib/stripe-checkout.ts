import { stripeIdempotencyKey } from "@/lib/stripe-billing";

type StripeCustomerCreator = {
  customers: {
    create(
      parameters: {
        email: string;
        name: string;
        metadata: { finite_workos_user_id: string };
      },
      options: { idempotencyKey: string }
    ): Promise<{ id: string }>;
  };
};

export async function ensureStripeCheckoutCustomer(input: {
  stripe: StripeCustomerCreator;
  existingStripeCustomerId: string;
  provisionalCustomerOrgId: string;
  customerOrgName: string;
  email: string;
  workosUserId: string;
  linkCustomer: (stripeCustomerId: string) => Promise<{ customer_org_id: string }>;
}) {
  const existing = input.existingStripeCustomerId.trim();
  if (existing) {
    return {
      stripeCustomerId: existing,
      customerOrgId: input.provisionalCustomerOrgId,
    };
  }

  // Core is the authority that creates/canonicalizes the billing organization.
  // Do not stamp the read-side provisional id into Stripe before Core links it.
  const customer = await input.stripe.customers.create(
    {
      email: input.email,
      name: input.customerOrgName,
      metadata: { finite_workos_user_id: input.workosUserId },
    },
    {
      idempotencyKey: stripeIdempotencyKey("customer", input.workosUserId),
    }
  );
  const linked = await input.linkCustomer(customer.id);
  if (!linked.customer_org_id?.trim()) {
    throw new Error("Core did not return a customer organization id.");
  }
  return {
    stripeCustomerId: customer.id,
    customerOrgId: linked.customer_org_id,
  };
}
