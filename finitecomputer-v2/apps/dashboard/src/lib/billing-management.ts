import { unstable_rethrow } from "next/navigation";
import type { CoreCustomerBillingAccount } from "./core-client";
import { billingSubscriptionShouldUsePortal } from "./stripe-billing";

export type BillingManagementState = { error?: string };

type BillingManagementDependencies = {
  loadBillingAccount: () => Promise<Pick<CoreCustomerBillingAccount,
    "stripe_customer_id" | "stripe_subscription_id" | "subscription_status"
  > | null | undefined>;
  checkoutDestination: () => Promise<string>;
  portalDestination: (customerId: string) => Promise<string>;
};

// Destination builders retain their throwing contract for onboarding callers.
// Only the Manage billing form converts failed requests into inline feedback.
export async function billingManagementResult(dependencies: BillingManagementDependencies): Promise<
  { destination: string } | { error: string }
> {
  try {
    const account = await dependencies.loadBillingAccount();
    const customerId = account?.stripe_customer_id?.trim();
    const destination = customerId && billingSubscriptionShouldUsePortal(
      account?.subscription_status, account?.stripe_subscription_id
    )
      ? await dependencies.portalDestination(customerId)
      : await dependencies.checkoutDestination();
    return { destination };
  } catch (error) {
    unstable_rethrow(error);
    return { error: "Billing is unavailable right now. Please try again shortly." };
  }
}
