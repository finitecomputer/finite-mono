import { unstable_rethrow } from "next/navigation";

export type BillingManagementState = { error?: string };

type BillingManagementDependencies = {
  loadCustomerId: () => Promise<string | null | undefined>;
  checkoutDestination: () => Promise<string>;
  portalDestination: (customerId: string) => Promise<string>;
};

// Destination builders retain their throwing contract for onboarding callers.
// Only the Manage billing form converts failed requests into inline feedback.
export async function billingManagementResult(dependencies: BillingManagementDependencies): Promise<
  { destination: string } | { error: string }
> {
  try {
    const customerId = (await dependencies.loadCustomerId())?.trim();
    const destination = customerId
      ? await dependencies.portalDestination(customerId)
      : await dependencies.checkoutDestination();
    return { destination };
  } catch (error) {
    unstable_rethrow(error);
    return { error: "Billing is unavailable right now. Please try again shortly." };
  }
}
