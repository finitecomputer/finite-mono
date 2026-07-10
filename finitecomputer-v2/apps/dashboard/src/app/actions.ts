"use server";
import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";

import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import type { OneTimeKeyActionState } from "@/lib/admin-ops";
import {
  adminIssueCoreFinitePrivateFriendKey,
  adminRecoverCoreRuntime,
  adminResetCoreFinitePrivateWindow,
  adminRestartCoreRuntime,
  adminRevokeCoreFinitePrivateApiKey,
  adminRotateCoreFinitePrivateApiKey,
  approveCoreFinitePrivateGrant,
  cancelFailedCoreAgentCreationRequest,
  claimCoreImportCandidates,
  coreProjectSupportsHostedRestart,
  issueCoreFinitePrivateApiKey,
  linkCoreStripeCustomer,
  loadCoreBillingOverview,
  loadCoreMe,
  resetCoreFinitePrivateGrant,
  revokeCoreFinitePrivateApiKey,
  revokeCoreFinitePrivateGrant,
  rotateCoreFinitePrivateApiKey,
  requestCoreRuntimeDestroy,
  requestCoreRuntimeRecoverKnownGoodChat,
  requestCoreRuntimeRestart,
  requestCoreRuntimeStop,
} from "@/lib/core-client";
import {
  billingSubscriptionShouldUsePortal,
  requireStripeClient,
  standardAgentPriceId,
  stripeDashboardReturnUrl,
} from "@/lib/stripe-billing";

export async function claimCoreImportCandidatesAction(formData: FormData) {
  const selectedCandidateIds = formData
    .getAll("candidateId")
    .map((value) => String(value));
  await claimCoreImportCandidates(selectedCandidateIds);
  revalidatePath("/");
  revalidatePath("/dashboard");
}

export async function restartCoreRuntimeAction(formData: FormData) {
  const machineId = String(formData.get("machineId") ?? "");
  const access = await loadDashboardMachineAccess(machineId);

  if (!access || access.mode !== "core" || !access.coreProject) {
    throw new Error("Hosted runtime restart is only available for hosted agents.");
  }
  if (!coreProjectSupportsHostedRestart(access.coreProject)) {
    throw new Error("This hosted agent does not support hosted runtime restart.");
  }

  await requestCoreRuntimeRestart(access.coreProject.project.id);

  const redirectPath = String(formData.get("redirectPath") ?? `/dashboard/machines/${machineId}`);
  revalidatePath("/dashboard");
  revalidatePath(`/dashboard/machines/${machineId}`);
  revalidatePath(redirectPath);
}

export async function recoverCoreRuntimeAction(formData: FormData) {
  const machineId = String(formData.get("machineId") ?? "");
  const access = await loadDashboardMachineAccess(machineId);

  if (!access || access.mode !== "core" || !access.coreProject) {
    throw new Error("Hosted runtime recovery is only available for hosted agents.");
  }
  if (!coreProjectSupportsHostedRestart(access.coreProject)) {
    throw new Error("This hosted agent does not support hosted runtime recovery.");
  }

  await requestCoreRuntimeRecoverKnownGoodChat(access.coreProject.project.id);

  const redirectPath = String(formData.get("redirectPath") ?? `/dashboard/machines/${machineId}`);
  revalidatePath("/dashboard");
  revalidatePath(`/dashboard/machines/${machineId}`);
  revalidatePath(redirectPath);
}

export async function stopCoreRuntimeAction(formData: FormData) {
  const machineId = String(formData.get("machineId") ?? "");
  const access = await loadDashboardMachineAccess(machineId);

  if (!access || access.mode !== "core" || !access.coreProject) {
    throw new Error("Hosted runtime stop is only available for hosted agents.");
  }
  if (!coreProjectSupportsHostedRestart(access.coreProject)) {
    throw new Error("This hosted agent does not support hosted runtime stop.");
  }

  await requestCoreRuntimeStop(access.coreProject.project.id);

  const redirectPath = String(formData.get("redirectPath") ?? `/dashboard/machines/${machineId}`);
  revalidatePath("/dashboard");
  revalidatePath(`/dashboard/machines/${machineId}`);
  revalidatePath(redirectPath);
}

export async function destroyCoreRuntimeAction(formData: FormData) {
  const machineId = String(formData.get("machineId") ?? "");
  const access = await loadDashboardMachineAccess(machineId);

  if (!access || access.mode !== "core" || !access.coreProject) {
    throw new Error("Hosted runtime destroy is only available for hosted agents.");
  }
  if (!coreProjectSupportsHostedRestart(access.coreProject)) {
    throw new Error("This hosted agent does not support hosted runtime destroy.");
  }

  await requestCoreRuntimeDestroy(access.coreProject.project.id);

  const redirectPath = String(formData.get("redirectPath") ?? `/dashboard/machines/${machineId}`);
  revalidatePath("/dashboard");
  revalidatePath(`/dashboard/machines/${machineId}`);
  revalidatePath(redirectPath);
}

export async function cancelFailedAgentCreationRequestAction(formData: FormData) {
  const requestId = String(formData.get("requestId") ?? "");
  const core = await loadCoreMe({ cacheMode: "fresh" });
  const request = core.me?.agent_creation_requests.find(
    (candidate) => candidate.id === requestId
  );

  if (!request || request.status !== "failed") {
    throw new Error("Only failed agent creation requests can be reset.");
  }

  await cancelFailedCoreAgentCreationRequest(request.id);

  revalidatePath("/");
  revalidatePath("/dashboard");
}

export async function startBillingCheckoutAction() {
  redirect(await billingCheckoutDestination());
}

export async function billingCheckoutDestination() {
  const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
  if (!billing.billing || !billing.account.email || !billing.account.workosUserId) {
    throw new Error(billing.error ?? "A verified WorkOS account is required for billing.");
  }

  const stripe = requireStripeClient();
  let stripeCustomerId = billing.billing.billing_account?.stripe_customer_id?.trim() ?? "";
  if (!stripeCustomerId) {
    const customer = await stripe.customers.create({
      email: billing.account.email,
      name: billing.billing.customer_org.name,
      metadata: {
        finite_customer_org_id: billing.billing.customer_org.id,
        finite_workos_user_id: billing.account.workosUserId,
      },
    });
    stripeCustomerId = customer.id;
    await linkCoreStripeCustomer(stripeCustomerId);
  }
  if (
    billingSubscriptionShouldUsePortal(
      billing.billing.billing_account?.subscription_status,
      billing.billing.billing_account?.stripe_subscription_id
    )
  ) {
    return billingPortalDestination(stripeCustomerId);
  }

  const checkout = await stripe.checkout.sessions.create({
    mode: "subscription",
    customer: stripeCustomerId,
    client_reference_id: billing.billing.customer_org.id,
    allow_promotion_codes: true,
    success_url: stripeDashboardReturnUrl("/dashboard?billing=success"),
    cancel_url: stripeDashboardReturnUrl("/dashboard?billing=cancelled"),
    line_items: [
      {
        price: standardAgentPriceId(),
        quantity: 1,
      },
    ],
    metadata: {
      finite_customer_org_id: billing.billing.customer_org.id,
    },
    subscription_data: {
      metadata: {
        finite_customer_org_id: billing.billing.customer_org.id,
      },
    },
  });

  if (!checkout.url) {
    throw new Error("Stripe did not return a Checkout URL.");
  }

  return checkout.url;
}

export async function openBillingPortalAction() {
  const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
  const stripeCustomerId = billing.billing?.billing_account?.stripe_customer_id?.trim();
  if (!stripeCustomerId) {
    return startBillingCheckoutAction();
  }

  redirect(await billingPortalDestination(stripeCustomerId));
}

async function billingPortalDestination(stripeCustomerId: string) {
  const portal = await requireStripeClient().billingPortal.sessions.create({
    customer: stripeCustomerId,
    return_url: stripeDashboardReturnUrl("/dashboard"),
  });

  return portal.url;
}

export async function approveFinitePrivateGrantAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can approve Finite Private grants.");
  }

  await approveCoreFinitePrivateGrant({
    verifiedEmail: String(formData.get("verifiedEmail") ?? ""),
    workosUserId: String(formData.get("workosUserId") ?? ""),
    limitProfileId: String(formData.get("limitProfileId") ?? ""),
  });

  revalidatePath("/dashboard");
}

export async function issueFinitePrivateApiKeyAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can issue Finite Private keys.");
  }

  await issueCoreFinitePrivateApiKey({
    grantId: String(formData.get("grantId") ?? ""),
    rawKey: String(formData.get("rawKey") ?? ""),
    projectId: String(formData.get("projectId") ?? ""),
    agentRuntimeId: String(formData.get("agentRuntimeId") ?? ""),
  });

  revalidatePath("/dashboard");
}

export async function resetFinitePrivateGrantAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can reset Finite Private grants.");
  }

  await resetCoreFinitePrivateGrant(String(formData.get("grantId") ?? ""));

  revalidatePath("/dashboard");
}

export async function revokeFinitePrivateGrantAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can revoke Finite Private grants.");
  }

  await revokeCoreFinitePrivateGrant(String(formData.get("grantId") ?? ""));

  revalidatePath("/dashboard");
}

export async function rotateFinitePrivateApiKeyAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can rotate Finite Private keys.");
  }

  await rotateCoreFinitePrivateApiKey({
    keyId: String(formData.get("keyId") ?? ""),
    rawKey: String(formData.get("rawKey") ?? ""),
  });

  revalidatePath("/dashboard");
}

export async function revokeFinitePrivateApiKeyAction(formData: FormData) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error("Only dashboard admins can revoke Finite Private keys.");
  }

  await revokeCoreFinitePrivateApiKey(String(formData.get("keyId") ?? ""));

  revalidatePath("/dashboard");
}

// --- Admin Ops (/dashboard/admin) ---
//
// The isAdmin checks below are a UI gate only. Core independently authorizes
// each call against FC_CORE_ADMIN_EMAILS using the admin's verified identity
// headers, so a bypassed dashboard gate still cannot mutate Core.

async function requireAdminViewer(action: string) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error(`Only dashboard admins can ${action}.`);
  }
}

export async function adminOpsRestartRuntimeAction(formData: FormData) {
  await requireAdminViewer("restart hosted runtimes");
  await adminRestartCoreRuntime(String(formData.get("projectId") ?? ""));
  revalidatePath("/dashboard/admin");
}

export async function adminOpsRecoverRuntimeAction(formData: FormData) {
  await requireAdminViewer("recover hosted runtimes");
  await adminRecoverCoreRuntime(String(formData.get("projectId") ?? ""));
  revalidatePath("/dashboard/admin");
}

export async function adminOpsRevokeFinitePrivateKeyAction(formData: FormData) {
  await requireAdminViewer("revoke Finite Private keys");
  await adminRevokeCoreFinitePrivateApiKey(String(formData.get("keyId") ?? ""));
  revalidatePath("/dashboard/admin");
}

export async function adminOpsResetFinitePrivateWindowAction(formData: FormData) {
  await requireAdminViewer("reset Finite Private burst windows");
  await adminResetCoreFinitePrivateWindow(String(formData.get("grantId") ?? ""));
  revalidatePath("/dashboard/admin");
}

// One-time key actions: the raw key is returned in the action state so the
// client can show it exactly once. It is never persisted or logged.
export async function adminOpsIssueFriendKeyAction(
  _prevState: OneTimeKeyActionState,
  formData: FormData
): Promise<OneTimeKeyActionState> {
  try {
    await requireAdminViewer("issue Finite Private friend keys");
    const issued = await adminIssueCoreFinitePrivateFriendKey({
      email: String(formData.get("email") ?? ""),
      limitProfileId: String(formData.get("limitProfileId") ?? ""),
    });
    revalidatePath("/dashboard/admin");
    return {
      status: "issued",
      keyId: issued.api_key.id,
      grantId: issued.grant?.id ?? null,
      rawKey: issued.raw_api_key,
      note: issued.raw_api_key_note,
    };
  } catch (error) {
    return {
      status: "error",
      error: error instanceof Error ? error.message : "Issuing the friend key failed.",
    };
  }
}

export async function adminOpsRotateKeyAction(
  _prevState: OneTimeKeyActionState,
  formData: FormData
): Promise<OneTimeKeyActionState> {
  try {
    await requireAdminViewer("rotate Finite Private keys");
    const rotated = await adminRotateCoreFinitePrivateApiKey(
      String(formData.get("keyId") ?? "")
    );
    revalidatePath("/dashboard/admin");
    return {
      status: "issued",
      keyId: rotated.api_key.id,
      grantId: rotated.grant?.id ?? null,
      rawKey: rotated.raw_api_key,
      note: rotated.raw_api_key_note,
    };
  } catch (error) {
    return {
      status: "error",
      error: error instanceof Error ? error.message : "Rotating the key failed.",
    };
  }
}
