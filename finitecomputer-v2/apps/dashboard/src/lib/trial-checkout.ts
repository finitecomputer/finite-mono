import { randomUUID } from "node:crypto";
import type Stripe from "stripe";
import { loadCoreTrialOffer, reserveCoreTrial } from "@/lib/core-client";
import { requireStripeClient, standardAgentCheckoutParams, stripeIdempotencyKey } from "@/lib/stripe-billing";

export function trialCheckoutParams(
  input: Parameters<typeof standardAgentCheckoutParams>[0],
  offer: { campaignId: string; trialDays: number },
  attemptId: string,
): Stripe.Checkout.SessionCreateParams {
  const base = standardAgentCheckoutParams(input);
  const metadata = { ...base.metadata, finite_trial_campaign_id: offer.campaignId, finite_trial_attempt_id: attemptId };
  return {
    ...base,
    // Trial eligibility is already validated in Finite. Do not accidentally
    // stack an invoice discount onto the first $200 post-trial payment.
    allow_promotion_codes: false,
    payment_method_collection: "always",
    expires_at: Math.floor(Date.now() / 1000) + 31 * 60,
    metadata,
    subscription_data: { metadata, trial_period_days: offer.trialDays },
  };
}

export async function trialCheckoutDestination(
  code: string,
  input: Parameters<typeof standardAgentCheckoutParams>[0],
) {
  const offer = await loadCoreTrialOffer(code);
  const stripe = requireStripeClient();
  const price = await stripe.prices.retrieve(input.priceId);
  if (!price.active || price.currency !== "usd" || price.unit_amount !== 20000 ||
      price.recurring?.interval !== "month" || price.recurring.interval_count !== 1) {
    throw new Error("The trial plan is unavailable. Please contact support.");
  }
  const attemptId = randomUUID();
  const session = await stripe.checkout.sessions.create(trialCheckoutParams(input, offer, attemptId), {
    idempotencyKey: stripeIdempotencyKey("checkout", attemptId),
  });
  // No session URL reaches the user until Core atomically reserves capacity.
  // Every reservation therefore has a real Stripe session that will emit an
  // expiry event even if this process dies before returning its redirect.
  const reservation = await reserveCoreTrial({
    code, customerOrgId: input.customerOrgId, stripeCustomerId: input.stripeCustomerId,
    stripeSessionId: session.id, attemptId, checkoutExpiresAt: session.expires_at,
    trialDays: offer.trialDays,
  }).catch(async (error: unknown) => {
    // On an ambiguous Core response, expiring the unexposed session is safe:
    // its verified expiry releases any reservation that committed.
    await stripe.checkout.sessions.expire(session.id).catch(() => undefined);
    throw error;
  });
  if (reservation.stripeSessionId !== session.id) {
    await stripe.checkout.sessions.expire(session.id);
  }
  const chosen = reservation.stripeSessionId === session.id ? session :
    await stripe.checkout.sessions.retrieve(reservation.stripeSessionId);
  if (chosen.status !== "open" || !chosen.url) {
    throw new Error("Your previous trial checkout is finishing or expired. Please return to the dashboard and try again shortly.");
  }
  return chosen.url;
}
