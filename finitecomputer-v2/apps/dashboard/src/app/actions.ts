"use server";
import { randomUUID } from "node:crypto";
import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";

import { getAccountAuthContext, loadOptionalViewerContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  launchCodeBatchFormInput,
  type OneTimeKeyActionState,
  type OneTimeLaunchCodeActionState,
} from "@/lib/admin-ops";
import {
  adminIssueCoreLaunchCodeBatch,
  adminIssueCoreFinitePrivateFriendKey,
  adminRecoverCoreRuntime,
  adminRevokeCoreLaunchCodeBatch,
  adminResetCoreFinitePrivateWindow,
  adminRestartCoreRuntime,
  adminRevokeCoreFinitePrivateApiKey,
  adminRotateCoreFinitePrivateApiKey,
  adminUpgradeCoreRuntime,
  approveCoreFinitePrivateGrant,
  cancelFailedCoreAgentCreationRequest,
  claimCoreFinitePrivateDailyReset,
  coreAdminRuntimeSupportsRecovery,
  coreAdminRuntimeSupportsRestart,
  coreAdminRuntimeSupportsUpgrade,
  coreProjectSupportsHostedRecovery,
  coreProjectSupportsHostedRestart,
  coreProjectSupportsHostedStop,
  coreProjectSupportsRetirement,
  issueCoreFinitePrivateApiKey,
  linkCoreStripeCustomer,
  loadCoreAdminRuntimes,
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
  standardAgentCheckoutParams,
  standardAgentPriceId,
  stripeCheckoutAvailable,
  stripeDashboardOnboardingReturnPath,
  stripeDashboardReturnUrl,
  stripeIdempotencyKey,
} from "@/lib/stripe-billing";
import { ensureStripeCheckoutCustomer } from "@/lib/stripe-checkout";

export async function restartCoreRuntimeAction(formData: FormData) {
  const machineId = String(formData.get("machineId") ?? "");
  const access = await loadDashboardMachineAccess(machineId);

  if (!access || access.mode !== "core" || !access.coreProject) {
    throw new Error("This agent cannot be restarted from the dashboard.");
  }
  if (!coreProjectSupportsHostedRestart(access.coreProject)) {
    throw new Error("This agent cannot be restarted from the dashboard.");
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
    throw new Error("Chat recovery is not available for this agent.");
  }
  // Operator-only maintenance for now; the section is admin-gated in the UI
  // and this keeps a hand-crafted POST from bypassing that.
  if (!access.viewer.isAdmin) {
    throw new Error("Chat recovery is not available for this agent.");
  }
  if (!coreProjectSupportsHostedRecovery(access.coreProject)) {
    throw new Error("Chat recovery is not available for this agent.");
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
    throw new Error("This agent cannot be stopped from the dashboard.");
  }
  if (!coreProjectSupportsHostedStop(access.coreProject)) {
    throw new Error("This agent cannot be stopped from the dashboard.");
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
    throw new Error("This agent cannot be removed from the dashboard.");
  }
  if (!coreProjectSupportsRetirement(access.coreProject) || !access.canRetireRuntime) {
    throw new Error("This agent cannot be removed from the dashboard.");
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
  redirect(await billingCheckoutDestination(randomUUID()));
}

export async function billingCheckoutDestination(
  attemptId: string = randomUUID(),
  returnMachineId?: string | null
) {
  if (!stripeCheckoutAvailable()) {
    throw new Error("Payment is unavailable right now.");
  }
  const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
  if (!billing.billing || !billing.account.email || !billing.account.workosUserId) {
    throw new Error(billing.error ?? "Sign in again to manage billing.");
  }

  const stripe = requireStripeClient();
  const { stripeCustomerId, customerOrgId } = await ensureStripeCheckoutCustomer({
    stripe,
    existingStripeCustomerId:
      billing.billing.billing_account?.stripe_customer_id?.trim() ?? "",
    provisionalCustomerOrgId: billing.billing.customer_org.id,
    customerOrgName: billing.billing.customer_org.name,
    email: billing.account.email,
    workosUserId: billing.account.workosUserId,
    linkCustomer: linkCoreStripeCustomer,
  });
  if (
    billingSubscriptionShouldUsePortal(
      billing.billing.billing_account?.subscription_status,
      billing.billing.billing_account?.stripe_subscription_id
    )
  ) {
    return billingPortalDestination(stripeCustomerId, returnMachineId);
  }

  const successParams = new URLSearchParams({ new: "1", billing: "success" });
  const cancelParams = new URLSearchParams({ new: "1", billing: "cancelled" });
  if (returnMachineId) {
    successParams.set("machine", returnMachineId);
    cancelParams.set("machine", returnMachineId);
  }
  const checkout = await stripe.checkout.sessions.create(
    standardAgentCheckoutParams({
      stripeCustomerId,
      customerOrgId,
      priceId: standardAgentPriceId(),
      successUrl: stripeDashboardReturnUrl(`/dashboard?${successParams.toString()}`),
      cancelUrl: stripeDashboardReturnUrl(`/dashboard?${cancelParams.toString()}`),
    }),
    { idempotencyKey: stripeIdempotencyKey("checkout", attemptId) }
  );

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

async function billingPortalDestination(
  stripeCustomerId: string,
  returnMachineId?: string | null
) {
  const portal = await requireStripeClient().billingPortal.sessions.create({
    customer: stripeCustomerId,
    return_url: stripeDashboardReturnUrl(
      stripeDashboardOnboardingReturnPath(returnMachineId)
    ),
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

export type FinitePrivateDailyResetActionState = {
  message: string;
  tone: "success" | "error" | null;
};

export async function claimFinitePrivateDailyResetAction(
  _previousState: FinitePrivateDailyResetActionState,
  _formData: FormData
): Promise<FinitePrivateDailyResetActionState> {
  void _previousState;
  void _formData;
  const account = await getAccountAuthContext();
  if (!account.accessToken || !account.email || !account.workosUserId) {
    return { message: "Sign in again to reset Finite Private usage.", tone: "error" };
  }
  try {
    const result = await claimCoreFinitePrivateDailyReset();
    revalidatePath("/dashboard");
    if (!result.performed) {
      return {
        message: `Today's free reset is already used. The next one is available at ${result.status.freeDailyResetAvailableAgainAt}.`,
        tone: "success",
      };
    }
    return {
      message: `Usage reset. This burst window now resets at ${result.status.burstResetAt}.`,
      tone: "success",
    };
  } catch (error) {
    return {
      message:
        error instanceof Error ? error.message : "Finite Private usage could not be reset.",
      tone: "error",
    };
  }
}

// --- Admin Ops (/dashboard/admin) ---
//
// The isAdmin checks below are a UI gate only. Core independently authorizes
// each call from the validated WorkOS operator organization.
// headers, so a bypassed dashboard gate still cannot mutate Core.

async function requireAdminViewer(action: string) {
  const viewer = await loadOptionalViewerContext();
  if (!viewer.isAdmin) {
    throw new Error(`Only dashboard admins can ${action}.`);
  }
}

async function loadAdminRuntimeForAction(projectId: string) {
  const result = await loadCoreAdminRuntimes();
  return (
    result.runtimes?.find((runtime) => runtime.project_id === projectId) ?? null
  );
}

export async function adminOpsRestartRuntimeAction(formData: FormData) {
  await requireAdminViewer("restart hosted runtimes");
  const projectId = String(formData.get("projectId") ?? "");
  const runtime = await loadAdminRuntimeForAction(projectId);
  if (!coreAdminRuntimeSupportsRestart(runtime)) {
    throw new Error("This hosted runtime cannot be restarted from the dashboard.");
  }
  await adminRestartCoreRuntime(projectId);
  revalidatePath("/dashboard/admin");
}

export async function adminOpsRecoverRuntimeAction(formData: FormData) {
  await requireAdminViewer("recover hosted runtimes");
  const projectId = String(formData.get("projectId") ?? "");
  const runtime = await loadAdminRuntimeForAction(projectId);
  if (!coreAdminRuntimeSupportsRecovery(runtime)) {
    throw new Error("Chat recovery is not available for this hosted runtime.");
  }
  await adminRecoverCoreRuntime(projectId);
  revalidatePath("/dashboard/admin");
}

export async function adminOpsUpgradeRuntimeAction(formData: FormData) {
  await requireAdminViewer("upgrade hosted runtimes");
  const projectId = String(formData.get("projectId") ?? "");
  const runtime = await loadAdminRuntimeForAction(projectId);
  if (!coreAdminRuntimeSupportsUpgrade(runtime)) {
    throw new Error("This hosted runtime cannot be upgraded from the dashboard.");
  }
  await adminUpgradeCoreRuntime({
    projectId,
    targetRuntimeArtifactId: String(
      formData.get("targetRuntimeArtifactId") ?? ""
    ),
  });
  revalidatePath("/dashboard/admin");
  redirect("/dashboard/admin");
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

// One-time Launch Code issuance returns raw values only in this action state.
// The dashboard never writes them to a URL, log, cache, or later Core read.
export async function adminOpsIssueLaunchCodeBatchAction(
  _prevState: OneTimeLaunchCodeActionState,
  formData: FormData
): Promise<OneTimeLaunchCodeActionState> {
  try {
    await requireAdminViewer("issue Launch Code batches");
    const issued = await adminIssueCoreLaunchCodeBatch(launchCodeBatchFormInput(formData));
    revalidatePath("/dashboard/admin");
    return {
      status: "issued",
      batch: {
        id: issued.batch.id,
        name: issued.batch.name,
        codeCount: issued.batch.code_count,
        expiresAt: issued.batch.expires_at,
        hostingTier: issued.batch.hosting_tier ?? "standard",
      },
      codes: issued.codes.map((code) => ({ id: code.id, code: code.code })),
    };
  } catch (error) {
    return {
      status: "error",
      error: error instanceof Error ? error.message : "Issuing the Launch Code batch failed.",
    };
  }
}

export async function adminOpsRevokeLaunchCodeBatchAction(formData: FormData) {
  await requireAdminViewer("revoke Launch Code batches");
  await adminRevokeCoreLaunchCodeBatch(String(formData.get("batchId") ?? ""));
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
