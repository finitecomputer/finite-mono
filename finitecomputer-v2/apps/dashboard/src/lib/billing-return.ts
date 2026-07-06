// Checkout-return state machine for /dashboard?billing=success|cancelled.
//
// After Stripe Checkout redirects back, Core billing state depends on webhook
// arrival. This module decides — from server-derived inputs only — which
// dashboard state to render, so the seam between "Stripe accepted payment"
// and "Core granted the entitlement" gets a designed waiting state with a
// bounded poll instead of a stale billing setup panel.

export const BILLING_SYNC_TIMEOUT_MS = 90_000;
export const BILLING_SYNC_POLL_INTERVAL_MS = 2_000;
export const BILLING_SYNC_MAX_POLL_INTERVAL_MS = 10_000;

export type BillingReturnParam = "success" | "cancelled";

export type BillingReturnState =
  // No checkout return in play; render the dashboard as usual.
  | { kind: "idle" }
  // billing=success arrived before Core knows the subscription and no sync
  // window has been stamped yet: redirect once to stamp syncStartedAt.
  | { kind: "stamp-sync-start" }
  // Checkout succeeded, Core billing not yet active, still inside the sync
  // window: show the waiting panel and keep polling until deadlineAtMs.
  | { kind: "confirming"; deadlineAtMs: number }
  // Core billing is active: auto-advance to the create-agent flow.
  | { kind: "synced" }
  // The sync window elapsed without the webhook landing: show the
  // "payment received by Stripe, still syncing" fallback. No checkout button.
  | { kind: "sync-timeout" }
  // billing=cancelled: show a gentle note above the billing setup panel.
  | { kind: "cancelled" };

export function parseBillingReturnParam(
  value: string | null | undefined
): BillingReturnParam | null {
  if (value === "success" || value === "cancelled") {
    return value;
  }
  return null;
}

export function parseBillingSyncStartedAt(
  value: string | null | undefined
): number | null {
  if (!value?.trim()) {
    return null;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    return null;
  }
  return parsed;
}

export type BillingReturnInput = {
  billingParam: BillingReturnParam | null;
  // True when the Core billing overview loaded without error.
  billingLoaded: boolean;
  // Core's requires_billing: true until the subscription webhook syncs.
  requiresBilling: boolean;
  // Epoch ms stamped into the URL when the success return first rendered.
  syncStartedAtMs: number | null;
  nowMs: number;
  timeoutMs?: number;
};

// Clock-reading wrapper for server components, where the purity lint bans
// direct Date.now() calls during render.
export function resolveBillingReturnStateNow(
  input: Omit<BillingReturnInput, "nowMs">
): BillingReturnState {
  return resolveBillingReturnState({ ...input, nowMs: Date.now() });
}

// Redirect target that stamps the start of the bounded sync window.
export function billingSyncStampRedirectPath(nowMs: number = Date.now()) {
  return `/dashboard?billing=success&billingSyncStartedAt=${nowMs}`;
}

export function resolveBillingReturnState(input: BillingReturnInput): BillingReturnState {
  const timeoutMs = input.timeoutMs ?? BILLING_SYNC_TIMEOUT_MS;

  if (!input.billingParam || !input.billingLoaded) {
    return { kind: "idle" };
  }

  if (input.billingParam === "cancelled") {
    // Only note the cancellation while billing setup is still the next step.
    return input.requiresBilling ? { kind: "cancelled" } : { kind: "idle" };
  }

  if (!input.requiresBilling) {
    return { kind: "synced" };
  }

  if (input.syncStartedAtMs === null) {
    return { kind: "stamp-sync-start" };
  }

  const deadlineAtMs = input.syncStartedAtMs + timeoutMs;
  if (input.nowMs < deadlineAtMs) {
    return { kind: "confirming", deadlineAtMs };
  }

  return { kind: "sync-timeout" };
}
