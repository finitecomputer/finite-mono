import Stripe from "stripe";

import { STRIPE_API_VERSION } from "../src/lib/stripe-billing";
import {
  evaluateStripeReadiness,
  FINITE_STRIPE_WEBHOOK_URL,
  type StripeReadinessSnapshot,
} from "../src/lib/stripe-readiness";

async function main() {
  const secretKey = requiredEnv("STRIPE_READINESS_SECRET_KEY");
  if (!secretKey.startsWith("rk_live_")) {
    throw new Error("STRIPE_READINESS_SECRET_KEY must be a temporary live restricted key.");
  }

  const expectedAccountId = requiredEnv("STRIPE_EXPECTED_ACCOUNT_ID");
  const expectedPriceId = requiredEnv("STRIPE_EXPECTED_PRICE_ID");
  const publicLegalLinksConfirmed =
    process.env.STRIPE_PORTAL_PUBLIC_LEGAL_LINKS_CONFIRMED?.trim() === "1";
  const stripe = new Stripe(secretKey, { apiVersion: STRIPE_API_VERSION });

  const [account, price, taxSettings, portalPage, destinationPage] = await Promise.all([
    stripe.accounts.retrieve(null),
    stripe.prices.retrieve(expectedPriceId, { expand: ["product"] }),
    stripe.tax.settings.retrieve(),
    stripe.billingPortal.configurations.list({ limit: 100 }),
    stripe.v2.core.eventDestinations.list({
      include: ["webhook_endpoint.url"],
      limit: 100,
    }),
  ]);

  if (typeof price.product === "string" || price.product.deleted) {
    throw new Error("The expected live Price does not expand to an active Product.");
  }

  const portal = portalPage.data.find((candidate) => candidate.is_default) ?? null;
  const destination =
    destinationPage.data.find(
      (candidate) => candidate.webhook_endpoint?.url === FINITE_STRIPE_WEBHOOK_URL
    ) ?? null;
  const snapshot: StripeReadinessSnapshot = {
    account: {
      id: account.id,
      chargesEnabled: account.charges_enabled,
      payoutsEnabled: account.payouts_enabled,
      detailsSubmitted: account.details_submitted,
    },
    product: {
      active: price.product.active,
      livemode: price.product.livemode,
      name: price.product.name,
      taxCode: expandableId(price.product.tax_code),
    },
    price: {
      id: price.id,
      active: price.active,
      livemode: price.livemode,
      currency: price.currency,
      unitAmount: price.unit_amount,
      type: price.type,
      interval: price.recurring?.interval ?? null,
      intervalCount: price.recurring?.interval_count ?? null,
      taxBehavior: price.tax_behavior,
    },
    taxDefaultBehavior: taxSettings.defaults?.tax_behavior ?? null,
    portal: portal
      ? {
          active: portal.active,
          livemode: portal.livemode,
          isDefault: portal.is_default,
          defaultReturnUrl: portal.default_return_url,
          termsSource: legalLinkSource(
            portal.business_profile.terms_of_service_url,
            publicLegalLinksConfirmed
          ),
          privacySource: legalLinkSource(
            portal.business_profile.privacy_policy_url,
            publicLegalLinksConfirmed
          ),
          customerUpdateEnabled: portal.features.customer_update.enabled,
          customerAllowedUpdates: portal.features.customer_update.allowed_updates,
          invoiceHistoryEnabled: portal.features.invoice_history.enabled,
          paymentMethodUpdateEnabled: portal.features.payment_method_update.enabled,
          subscriptionCancelEnabled: portal.features.subscription_cancel.enabled,
          subscriptionCancelMode: portal.features.subscription_cancel.mode,
          cancellationReasonEnabled:
            portal.features.subscription_cancel.cancellation_reason.enabled,
          subscriptionUpdateEnabled: portal.features.subscription_update.enabled,
          loginPageEnabled: portal.login_page.enabled,
        }
      : null,
    webhook: destination
      ? {
          livemode: destination.livemode,
          status: destination.status,
          type: destination.type,
          eventPayload: destination.event_payload,
          eventsFrom: destination.events_from ?? [],
          enabledEvents: destination.enabled_events,
          snapshotApiVersion: destination.snapshot_api_version ?? null,
          url: destination.webhook_endpoint?.url ?? null,
        }
      : null,
  };
  const report = evaluateStripeReadiness(snapshot, {
    accountId: expectedAccountId,
    priceId: expectedPriceId,
    apiVersion: STRIPE_API_VERSION,
  });

  console.log(JSON.stringify(report, null, 2));
  if (!report.ready) process.exitCode = 1;
}

function requiredEnv(name: string) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

function expandableId(value: string | { id: string } | null | undefined) {
  if (!value) return null;
  return typeof value === "string" ? value : value.id;
}

function legalLinkSource(value: string | null, publicBusinessInformationConfirmed: boolean) {
  if (value) return "portal" as const;
  if (publicBusinessInformationConfirmed) return "public_business_information" as const;
  return "missing" as const;
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : "Unknown readiness audit failure.";
  console.error(`stripe_readiness_error=${message}`);
  process.exitCode = 1;
});
