// Home-view state machine for /dashboard.
//
// The dashboard home page renders exactly one primary view per load — the
// Ready interstitial, the billing-sync waiting state, the creation form, the
// project list, the creation status panels, or the empty-account prompt —
// chosen here from server-derived inputs in a fixed priority order. The order
// reproduces the exclusions the page previously maintained by hand across
// showAgentReady / showCreateAgent / showBillingSyncState / showEmptyAccount,
// so overlapping primaries are unreachable by construction. In particular a
// tracked request flipping to "running" while a checkout return is still
// confirming resolves to "agent-ready" only; the old flags rendered both the
// Ready panel and the billing-sync panel in that state.

import type { BillingReturnState } from "./billing-return";

export type DashboardHomeBillingSyncState = Extract<
  BillingReturnState,
  { kind: "confirming" | "sync-timeout" }
>;

export type DashboardHomeView =
  // Tracked creation request is running and its project chat is reachable.
  | { kind: "agent-ready"; chatHref: string; name: string }
  // Checkout succeeded but the subscription webhook has not synced yet.
  | { kind: "billing-sync"; returnState: DashboardHomeBillingSyncState }
  // Creation form, including the chat-authorization retry form.
  | { kind: "create-agent" }
  // Non-immersive account view with at least one project.
  | { kind: "projects" }
  // A creation request is still in flight and no view above applies.
  | { kind: "pending" }
  // A creation request failed; the view carries the reset action.
  | { kind: "failed" }
  // Nothing above applies: no visible projects, nothing pending or failed.
  | { kind: "empty-account" };

export type DashboardHomeViewInput = {
  coreConfigured: boolean;
  hasAccountEmail: boolean;
  isNewAgentFlow: boolean;
  hasProjects: boolean;
  hasPendingAgentCreation: boolean;
  hasFailedAgentCreation: boolean;
  // A preserved draft plus a creation error keeps the retry form visible.
  creationAuthorizationRetry: boolean;
  // Set when the tracked creation request is running with a reachable project.
  readyAgent: { chatHref: string; name: string } | null;
  billingReturn: BillingReturnState;
};

export function resolveDashboardHomeView(
  input: DashboardHomeViewInput
): DashboardHomeView {
  // While a successful checkout is still syncing, the billing setup panel
  // (and its Start checkout button) must stay hidden to avoid a second
  // subscription attempt.
  const billingSyncState: DashboardHomeBillingSyncState | null =
    input.billingReturn.kind === "confirming" ||
    input.billingReturn.kind === "sync-timeout"
      ? input.billingReturn
      : null;
  const onboardingSurface =
    input.coreConfigured &&
    input.hasAccountEmail &&
    (!input.hasProjects || input.isNewAgentFlow);

  // The first matching kind wins, so later kinds can never overlap an
  // earlier one.
  if (input.readyAgent) {
    return { kind: "agent-ready", ...input.readyAgent };
  }
  if (
    billingSyncState &&
    onboardingSurface &&
    !input.hasPendingAgentCreation &&
    !input.hasFailedAgentCreation
  ) {
    return { kind: "billing-sync", returnState: billingSyncState };
  }
  if (
    onboardingSurface &&
    !billingSyncState &&
    (input.creationAuthorizationRetry ||
      (!input.hasPendingAgentCreation && !input.hasFailedAgentCreation))
  ) {
    return { kind: "create-agent" };
  }
  // The project list yields to the creation status panels only in the
  // immersive new-agent flow; outside it the status panels render alongside
  // the list, so "projects" stays the primary view.
  if (!input.isNewAgentFlow && input.hasProjects) {
    return { kind: "projects" };
  }
  if (input.hasPendingAgentCreation) {
    return { kind: "pending" };
  }
  if (input.hasFailedAgentCreation) {
    return { kind: "failed" };
  }
  return { kind: "empty-account" };
}
