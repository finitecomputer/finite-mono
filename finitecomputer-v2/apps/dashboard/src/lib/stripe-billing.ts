import { createHash } from "node:crypto";
import Stripe from "stripe";

export type StripeBillingStatus = {
  configured: boolean;
  missing: string[];
};

const REQUIRED_STRIPE_ENV = [
  "STRIPE_SECRET_KEY",
  "STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID",
  "STRIPE_WEBHOOK_SECRET",
] as const;
const STRIPE_RETURN_ORIGIN_ENV = "FC_DASHBOARD_BASE_URL";

let stripeClient: Stripe | null = null;

export function stripeBillingStatus(env: Record<string, string | undefined> = process.env): StripeBillingStatus {
  const missing: string[] = REQUIRED_STRIPE_ENV.filter((name) => !env[name]?.trim());
  try {
    stripeDashboardReturnUrl("/dashboard", env);
  } catch {
    missing.push(STRIPE_RETURN_ORIGIN_ENV);
  }
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

export function stripeDashboardReturnUrl(
  pathname = "/dashboard",
  env: Record<string, string | undefined> = process.env
) {
  const baseUrl =
    env.NEXT_PUBLIC_APP_URL?.trim() ||
    env.FC_DASHBOARD_PUBLIC_URL?.trim() ||
    env.FC_DASHBOARD_BASE_URL?.trim();
  if (!baseUrl) {
    throw new Error(
      "Stripe return URL requires NEXT_PUBLIC_APP_URL, FC_DASHBOARD_PUBLIC_URL, or FC_DASHBOARD_BASE_URL."
    );
  }
  const base = new URL(baseUrl);
  if (!["http:", "https:"].includes(base.protocol) || base.username || base.password) {
    throw new Error("Stripe return URL base must use http or https.");
  }
  if (env.NODE_ENV === "production" && base.hostname === "localhost") {
    throw new Error("Stripe return URL must not point at localhost in production.");
  }
  const url = new URL(pathname, `${base.origin}/`);
  return url.toString();
}

export function stripeDashboardOnboardingReturnPath(returnMachineId?: string | null) {
  if (!returnMachineId) {
    return "/dashboard";
  }
  const params = new URLSearchParams({
    new: "1",
    machine: returnMachineId,
  });
  return `/dashboard?${params.toString()}`;
}

export function stripeIdempotencyKey(
  operation: "customer" | "checkout",
  stableAttemptId: string
) {
  const attempt = stableAttemptId.trim();
  if (!attempt || attempt.length > 512 || /[\u0000-\u001f\u007f]/u.test(attempt)) {
    throw new Error("Stripe checkout attempt id is invalid.");
  }
  const digest = createHash("sha256").update(attempt, "utf8").digest("hex");
  return `finite-${operation}:${digest}`;
}

export function standardAgentCheckoutMetadata(customerOrgId: string) {
  const id = customerOrgId.trim();
  if (!id) {
    throw new Error("Core customer organization id is required.");
  }
  return {
    clientReferenceId: id,
    checkout: { finite_customer_org_id: id },
    subscription: { finite_customer_org_id: id },
  };
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
