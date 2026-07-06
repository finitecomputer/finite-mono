import Stripe from "stripe";

export type StripeBillingStatus = {
  configured: boolean;
  missing: string[];
};

const REQUIRED_STRIPE_ENV = [
  "STRIPE_SECRET_KEY",
  "STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID",
] as const;

let stripeClient: Stripe | null = null;

export function stripeBillingStatus(env: Record<string, string | undefined> = process.env): StripeBillingStatus {
  const missing = REQUIRED_STRIPE_ENV.filter((name) => !env[name]?.trim());
  return {
    configured: missing.length === 0,
    missing,
  };
}

export function requireStripeClient() {
  const status = stripeBillingStatus();
  if (!status.configured) {
    throw new Error(`Stripe billing is not configured: ${status.missing.join(", ")}`);
  }
  if (!stripeClient) {
    stripeClient = new Stripe(requiredEnv("STRIPE_SECRET_KEY"), {
      appInfo: {
        name: "finitecomputer-v2",
      },
    });
  }
  return stripeClient;
}

export function standardAgentPriceId() {
  return requiredEnv("STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID");
}

export function stripeWebhookSecret() {
  return requiredEnv("STRIPE_WEBHOOK_SECRET");
}

export function stripeDashboardReturnUrl(pathname = "/dashboard") {
  const baseUrl =
    process.env.NEXT_PUBLIC_APP_URL?.trim() ||
    process.env.FC_DASHBOARD_PUBLIC_URL?.trim() ||
    process.env.FC_DASHBOARD_BASE_URL?.trim();
  if (!baseUrl) {
    throw new Error(
      "Stripe return URL requires NEXT_PUBLIC_APP_URL, FC_DASHBOARD_PUBLIC_URL, or FC_DASHBOARD_BASE_URL."
    );
  }
  const url = new URL(pathname, baseUrl);
  if (!["http:", "https:"].includes(url.protocol)) {
    throw new Error("Stripe return URL base must use http or https.");
  }
  if (process.env.NODE_ENV === "production" && url.hostname === "localhost") {
    throw new Error("Stripe return URL must not point at localhost in production.");
  }
  return url.toString();
}

export function billingSubscriptionShouldUsePortal(
  subscriptionStatus: string | null | undefined,
  subscriptionId: string | null | undefined
) {
  if (!subscriptionId?.trim()) {
    return false;
  }
  return !["canceled", "incomplete_expired"].includes(subscriptionStatus ?? "");
}

export function isoFromStripeUnix(value: number | null | undefined) {
  return typeof value === "number" ? new Date(value * 1000).toISOString() : null;
}

function requiredEnv(name: string) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`${name} is required.`);
  }
  return value;
}
