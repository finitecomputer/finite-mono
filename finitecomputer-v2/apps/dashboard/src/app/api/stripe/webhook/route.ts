import Stripe from "stripe";
import { NextResponse } from "next/server";

import {
  type CoreBillingSubscriptionStatus,
  syncCoreStripeSubscription,
} from "@/lib/core-client";
import {
  isoFromStripeUnix,
  requireStripeClient,
  standardAgentPriceId,
  stripeWebhookSecret,
} from "@/lib/stripe-billing";

export const runtime = "nodejs";

export async function POST(request: Request) {
  const signature = request.headers.get("stripe-signature");
  if (!signature) {
    return NextResponse.json({ error: "missing Stripe signature" }, { status: 400 });
  }

  const body = await request.text();
  let event: Stripe.Event;
  try {
    event = requireStripeClient().webhooks.constructEvent(
      body,
      signature,
      stripeWebhookSecret()
    );
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "invalid Stripe webhook" },
      { status: 400 }
    );
  }

  try {
    switch (event.type) {
      case "checkout.session.completed":
        await handleCheckoutCompleted(event);
        break;
      case "customer.subscription.created":
      case "customer.subscription.updated":
      case "customer.subscription.deleted":
        await handleSubscriptionEvent(event);
        break;
    }
  } catch (error) {
    console.error("[stripe-webhook] failed to process event", {
      eventId: event.id,
      eventType: event.type,
      error: error instanceof Error ? error.message : String(error),
    });
    return NextResponse.json({ error: "failed to process Stripe webhook" }, { status: 500 });
  }

  return NextResponse.json({ received: true });
}

async function handleCheckoutCompleted(event: Stripe.Event) {
  const session = event.data.object as Stripe.Checkout.Session;
  if (session.mode !== "subscription") {
    return;
  }
  const subscriptionId = idString(session.subscription);
  if (!subscriptionId) {
    return;
  }
  const subscription = await requireStripeClient().subscriptions.retrieve(subscriptionId);
  await syncSubscription(subscription, event.id, event.created, session.metadata ?? {});
}

async function handleSubscriptionEvent(event: Stripe.Event) {
  const eventSubscription = event.data.object as Stripe.Subscription;
  const subscription = await requireStripeClient().subscriptions.retrieve(eventSubscription.id);
  await syncSubscription(subscription, event.id, event.created, eventSubscription.metadata ?? {});
}

async function syncSubscription(
  subscription: Stripe.Subscription,
  stripeEventId: string,
  // Stripe `event.created` (unix seconds): the monotonic ordering signal Core
  // uses to ignore webhooks delivered out of order.
  stripeEventCreated: number,
  fallbackMetadata: Stripe.Metadata | null = null
) {
  const stripeCustomerId = idString(subscription.customer);
  if (!stripeCustomerId) {
    throw new Error("Stripe subscription is missing customer id.");
  }
  const standardItem = standardSubscriptionItem(subscription);
  if (!standardItem) {
    console.warn("[stripe-webhook] ignoring subscription without standard hosted-agent price", {
      stripeSubscriptionId: subscription.id,
      stripeCustomerId,
      stripeEventId,
      stripePriceIds: subscription.items.data.map((item) => item.price.id),
    });
    return;
  }
  const metadata = {
    ...(fallbackMetadata ?? {}),
    ...(subscription.metadata ?? {}),
  };
  await syncCoreStripeSubscription({
    customerOrgId: metadata.finite_customer_org_id ?? null,
    stripeCustomerId,
    stripeSubscriptionId: subscription.id,
    stripePriceId: standardItem.price.id,
    subscriptionStatus: stripeStatus(subscription.status),
    currentPeriodEnd: isoFromStripeUnix(standardItem.current_period_end),
    cancelAtPeriodEnd: subscription.cancel_at_period_end,
    stripeEventId,
    stripeEventCreated,
  });
}

function standardSubscriptionItem(subscription: Stripe.Subscription) {
  const expectedPriceId = standardAgentPriceId();
  return subscription.items.data.find((item) => item.price.id === expectedPriceId) ?? null;
}

function idString(value: string | { id: string } | null) {
  if (!value) {
    return null;
  }
  return typeof value === "string" ? value : value.id;
}

function stripeStatus(status: Stripe.Subscription.Status): CoreBillingSubscriptionStatus {
  switch (status) {
    case "incomplete":
    case "incomplete_expired":
    case "trialing":
    case "active":
    case "past_due":
    case "canceled":
    case "unpaid":
    case "paused":
      return status;
    default:
      throw new Error(`Unsupported Stripe subscription status: ${status}`);
  }
}
