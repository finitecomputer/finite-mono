export const FINITE_STRIPE_WEBHOOK_URL = "https://finite.computer/api/stripe/webhook";
export const FINITE_STRIPE_EVENTS = [
  "checkout.session.completed",
  "customer.subscription.created",
  "customer.subscription.updated",
  "customer.subscription.deleted",
] as const;

export type StripeReadinessSnapshot = {
  account: {
    id: string;
    chargesEnabled: boolean;
    payoutsEnabled: boolean;
    detailsSubmitted: boolean;
  };
  product: {
    active: boolean;
    livemode: boolean;
    name: string;
    taxCode: string | null;
  };
  price: {
    id: string;
    active: boolean;
    livemode: boolean;
    currency: string;
    unitAmount: number | null;
    type: string;
    interval: string | null;
    intervalCount: number | null;
    taxBehavior: string | null;
  };
  taxDefaultBehavior: string | null;
  portal: {
    active: boolean;
    livemode: boolean;
    isDefault: boolean;
    defaultReturnUrl: string | null;
    hasTermsUrl: boolean;
    hasPrivacyUrl: boolean;
    customerUpdateEnabled: boolean;
    customerAllowedUpdates: string[];
    invoiceHistoryEnabled: boolean;
    paymentMethodUpdateEnabled: boolean;
    subscriptionCancelEnabled: boolean;
    subscriptionCancelMode: string;
    cancellationReasonEnabled: boolean;
    subscriptionUpdateEnabled: boolean;
    loginPageEnabled: boolean;
  } | null;
  webhook: {
    livemode: boolean;
    status: string;
    type: string;
    eventPayload: string;
    eventsFrom: string[];
    enabledEvents: string[];
    snapshotApiVersion: string | null;
    url: string | null;
  } | null;
};

export type StripeReadinessCheck = {
  name: string;
  passed: boolean;
  detail: string;
};

export function evaluateStripeReadiness(
  snapshot: StripeReadinessSnapshot,
  expected: { accountId: string; priceId: string; apiVersion: string }
) {
  const checks: StripeReadinessCheck[] = [];
  const check = (name: string, passed: boolean, detail: string) => {
    checks.push({ name, passed, detail });
  };
  const portal = snapshot.portal;
  const webhook = snapshot.webhook;

  check("account.id", snapshot.account.id === expected.accountId, snapshot.account.id);
  check("account.charges", snapshot.account.chargesEnabled, String(snapshot.account.chargesEnabled));
  check("account.payouts", snapshot.account.payoutsEnabled, String(snapshot.account.payoutsEnabled));
  check("account.details", snapshot.account.detailsSubmitted, String(snapshot.account.detailsSubmitted));

  check("product.live", snapshot.product.livemode, String(snapshot.product.livemode));
  check("product.active", snapshot.product.active, String(snapshot.product.active));
  check(
    "product.name",
    snapshot.product.name === "Finite Computer Hosted Agent",
    snapshot.product.name
  );
  check(
    "product.tax_code",
    snapshot.product.taxCode === "txcd_10103001",
    snapshot.product.taxCode ?? "missing"
  );

  check("price.id", snapshot.price.id === expected.priceId, snapshot.price.id);
  check("price.live", snapshot.price.livemode, String(snapshot.price.livemode));
  check("price.active", snapshot.price.active, String(snapshot.price.active));
  check("price.currency", snapshot.price.currency === "usd", snapshot.price.currency);
  check("price.amount", snapshot.price.unitAmount === 20_000, String(snapshot.price.unitAmount));
  check("price.type", snapshot.price.type === "recurring", snapshot.price.type);
  check("price.interval", snapshot.price.interval === "month", snapshot.price.interval ?? "missing");
  check("price.interval_count", snapshot.price.intervalCount === 1, String(snapshot.price.intervalCount));
  check(
    "price.tax_behavior",
    effectiveTaxBehavior(snapshot) === "exclusive",
    `${snapshot.price.taxBehavior ?? "unspecified"} via ${snapshot.taxDefaultBehavior ?? "no default"}`
  );

  check("portal.default", Boolean(portal?.isDefault), String(portal?.isDefault ?? false));
  check("portal.live", Boolean(portal?.livemode), String(portal?.livemode ?? false));
  check("portal.active", Boolean(portal?.active), String(portal?.active ?? false));
  check(
    "portal.return_url",
    portal?.defaultReturnUrl === "https://finite.computer/dashboard",
    portal?.defaultReturnUrl ?? "missing"
  );
  check("portal.terms", Boolean(portal?.hasTermsUrl), String(portal?.hasTermsUrl ?? false));
  check("portal.privacy", Boolean(portal?.hasPrivacyUrl), String(portal?.hasPrivacyUrl ?? false));
  check(
    "portal.customer_update",
    Boolean(portal?.customerUpdateEnabled) &&
      ["address", "email", "name", "phone"].every((value) =>
        portal?.customerAllowedUpdates.includes(value)
      ),
    portal?.customerAllowedUpdates.sort().join(",") ?? "missing"
  );
  check("portal.invoice_history", Boolean(portal?.invoiceHistoryEnabled), String(portal?.invoiceHistoryEnabled ?? false));
  check("portal.payment_method", Boolean(portal?.paymentMethodUpdateEnabled), String(portal?.paymentMethodUpdateEnabled ?? false));
  check(
    "portal.period_end_cancel",
    Boolean(portal?.subscriptionCancelEnabled) && portal?.subscriptionCancelMode === "at_period_end",
    `${portal?.subscriptionCancelEnabled ?? false}/${portal?.subscriptionCancelMode ?? "missing"}`
  );
  check("portal.cancel_reason", Boolean(portal?.cancellationReasonEnabled), String(portal?.cancellationReasonEnabled ?? false));
  check("portal.plan_switching_off", portal?.subscriptionUpdateEnabled === false, String(portal?.subscriptionUpdateEnabled ?? "missing"));
  check("portal.shareable_login_off", portal?.loginPageEnabled === false, String(portal?.loginPageEnabled ?? "missing"));

  check("webhook.present", Boolean(webhook), String(Boolean(webhook)));
  check("webhook.live", Boolean(webhook?.livemode), String(webhook?.livemode ?? false));
  check("webhook.enabled", webhook?.status === "enabled", webhook?.status ?? "missing");
  check("webhook.type", webhook?.type === "webhook_endpoint", webhook?.type ?? "missing");
  check("webhook.payload", webhook?.eventPayload === "snapshot", webhook?.eventPayload ?? "missing");
  check("webhook.scope", sameStrings(webhook?.eventsFrom ?? [], ["@self"]), (webhook?.eventsFrom ?? []).join(",") || "missing");
  check("webhook.url", webhook?.url === FINITE_STRIPE_WEBHOOK_URL, webhook?.url ?? "missing");
  check(
    "webhook.api_version",
    webhook?.snapshotApiVersion === expected.apiVersion,
    webhook?.snapshotApiVersion ?? "missing"
  );
  check(
    "webhook.events",
    sameStrings(webhook?.enabledEvents ?? [], [...FINITE_STRIPE_EVENTS]),
    (webhook?.enabledEvents ?? []).sort().join(",") || "missing"
  );

  return { ready: checks.every((candidate) => candidate.passed), checks };
}

function effectiveTaxBehavior(snapshot: StripeReadinessSnapshot) {
  if (snapshot.price.taxBehavior === "exclusive") return "exclusive";
  if (snapshot.price.taxBehavior === "inclusive") return "inclusive";
  if (snapshot.taxDefaultBehavior === "exclusive") return "exclusive";
  if (snapshot.taxDefaultBehavior === "inclusive") return "inclusive";
  if (snapshot.taxDefaultBehavior === "inferred_by_currency") {
    return ["usd", "cad"].includes(snapshot.price.currency) ? "exclusive" : "inclusive";
  }
  return "unknown";
}

function sameStrings(actual: string[], expected: string[]) {
  return (
    actual.length === expected.length &&
    [...actual].sort().every((value, index) => value === [...expected].sort()[index])
  );
}
