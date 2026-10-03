import {
  FINITE_STRIPE_WEBHOOK_URL,
  type StripeReadinessSnapshot,
} from "./stripe-readiness";

/** Only these fields may enter the readiness report, never signing secrets. */
export type StripeReadinessEventDestination = {
  id: string;
  livemode: boolean;
  status: string;
  type: string;
  event_payload: string;
  events_from?: string[] | null;
  enabled_events: string[];
  snapshot_api_version?: string | null;
  webhook_endpoint?: { url?: string | null } | null;
};

/** Messages constructed by this audit contain names/instructions, not API data. */
export class StripeReadinessError extends Error {}

export function stripeReadinessErrorMessage(error: unknown): string {
  return error instanceof StripeReadinessError
    ? error.message
    : "Stripe readiness audit failed. Verify read-only access and configuration; provider error details are withheld.";
}

export async function selectStripeReadinessWebhook(
  destinations:
    | AsyncIterable<StripeReadinessEventDestination>
    | Iterable<StripeReadinessEventDestination>,
  expectedDestinationId?: string,
): Promise<StripeReadinessSnapshot["webhook"]> {
  const expectedId = expectedDestinationId?.trim();
  const matches: StripeReadinessEventDestination[] = [];
  // Consume every page before accepting a URL as unique. Never infer the
  // runtime signing-secret binding from order, timestamps or event coverage.
  for await (const destination of destinations) {
    if (expectedId
      ? destination.id === expectedId
      : destination.webhook_endpoint?.url === FINITE_STRIPE_WEBHOOK_URL) {
      matches.push(destination);
    }
  }
  if (matches.length > 1) {
    throw new StripeReadinessError(
      "Webhook destination selection is ambiguous. Set STRIPE_EXPECTED_EVENT_DESTINATION_ID to the destination verified against the runtime signing-secret binding.",
    );
  }
  const destination = matches[0];
  if (!destination) {
    if (expectedId) {
      throw new StripeReadinessError("The expected webhook destination was not found. No URL fallback was selected.");
    }
    return null;
  }
  if (destination.webhook_endpoint?.url !== FINITE_STRIPE_WEBHOOK_URL) {
    throw new StripeReadinessError("The expected webhook destination does not use the Finite production webhook URL.");
  }
  return {
    id: destination.id,
    livemode: destination.livemode,
    status: destination.status,
    type: destination.type,
    eventPayload: destination.event_payload,
    eventsFrom: [...(destination.events_from ?? [])],
    enabledEvents: [...destination.enabled_events],
    snapshotApiVersion: destination.snapshot_api_version ?? null,
    url: destination.webhook_endpoint.url,
  };
}
