import { createHash } from "node:crypto";

import { getAccountAuthContext, type AccountAuthContext } from "@/lib/dashboard-auth";
import {
  invalidateServerSwrCache,
  readThroughServerSwr,
} from "@/lib/server-swr-cache";

export type CoreBridgeStatus = {
  configured: boolean;
  missing: string[];
};

export type CoreRuntimeStatus = "online" | "offline" | "stale" | "unknown";

export type CoreProjectImportCandidate = {
  id: string;
  display_name: string;
  status: "pending" | "claimed" | "admin_review";
  created_at: string;
  updated_at: string;
};

export type CoreProject = {
  id: string;
  display_name: string;
  hosting_tier?: CoreHostingTier | null;
  created_at: string;
  updated_at: string;
};

export type CoreRuntimeCapabilities = {
  restart?: boolean;
  recover_known_good_chat?: boolean;
  stop?: boolean;
  runtime_retirement?: boolean;
};

export type CoreAgentRuntime = {
  id: string;
  project_id: string;
  contact_endpoint?: string | null;
  runtime_status: CoreRuntimeStatus;
  hermes_available?: boolean | null;
  /**
   * Populated only from Core's persisted, versioned runtime capability
   * advertisement. N-1 rows omit it; browser code must not infer capabilities
   * from provider or artifact identity.
   */
  runtime_capabilities?: CoreRuntimeCapabilities | null;
  created_at: string;
  updated_at: string;
};

export type CoreVisibleProject = {
  project: CoreProject;
  runtime?: CoreAgentRuntime | null;
};

export type CoreAgentCreationRequest = {
  id: string;
  customer_org_id: string;
  owner_user_id: string;
  project_id: string;
  idempotency_key: string;
  display_name: string;
  runner_class: CoreRunnerClass;
  profile_picture_url?: string | null;
  status: "requested" | "launching" | "running" | "failed" | "cancelled";
  requested_launch_code?: string | null;
  agent_runtime_id?: string | null;
  created_at: string;
  updated_at: string;
};

export type CoreAgentCreationRequestSummary = {
  id: string;
  project_id: string;
  display_name: string;
  profile_picture_url?: string | null;
  status: "requested" | "launching" | "running" | "failed" | "cancelled";
  agent_runtime_id?: string | null;
  failure_message?: string | null;
  created_at: string;
  updated_at: string;
};

export type CoreRunnerClass =
  | "local_docker"
  | "apple_container"
  | "kata"
  | "phala"
  | "enclavia";

export type CoreAgentCreationResult = {
  project: CoreProject;
  request: CoreAgentCreationRequest;
  reused: boolean;
};

export type CoreRuntimeControlRequest = {
  id: string;
  project_id: string;
  agent_runtime_id: string;
  source_host_id: string;
  source_machine_id: string;
  requested_by_user_id: string;
  kind: "restart" | "recover_known_good_chat_runtime" | "upgrade" | "stop" | "destroy";
  target_runtime_artifact_id?: string | null;
  status: "requested" | "running" | "succeeded" | "failed";
  failure_message?: string | null;
  created_at: string;
  updated_at: string;
  completed_at?: string | null;
};

export type CoreSourceHostRelayEndpoint = {
  source_host_id: string;
  url: string;
  admin_token: string;
  created_at: string;
  updated_at: string;
};

export type CoreFinitePrivateGrant = {
  id: string;
  user_id: string;
  limit_profile_id: string;
  status: "active" | "revoked";
  current_window_started_at?: string | null;
  current_window_used_units: number;
  created_at: string;
  updated_at: string;
};

export type CoreFinitePrivateApiKey = {
  id: string;
  grant_id: string;
  project_id?: string | null;
  agent_runtime_id?: string | null;
  key_hash: string;
  status: "active" | "revoked";
  created_at: string;
  updated_at: string;
};

export type CoreFinitePrivateAdminAuditEvent = {
  id: string;
  action: string;
  target_type: string;
  target_id: string;
  grant_id?: string | null;
  api_key_id?: string | null;
  actor: string;
  metadata: unknown;
  created_at: string;
};

export type CoreFinitePrivateAdminState = {
  grants: CoreFinitePrivateGrant[];
  apiKeys: CoreFinitePrivateApiKey[];
  adminAuditEvents: CoreFinitePrivateAdminAuditEvent[];
};

export type CoreFinitePrivateAdminStateResult = CoreBridgeStatus & {
  state: CoreFinitePrivateAdminState | null;
  error: string | null;
};

export type CoreAdminRuntimeOverview = {
  project_id: string;
  project_display_name: string;
  owner_email?: string | null;
  agent_runtime_id: string;
  source_host_id: string;
  source_machine_id: string;
  runtime_artifact_id?: string | null;
  runtime_artifact_version_label?: string | null;
  runtime_status: CoreRuntimeStatus;
  last_heartbeat_at?: string | null;
  status_updated_at?: string | null;
  runtime_updated_at: string;
  hermes_available?: boolean | null;
  published_app_urls: string[];
  active_finite_private_key_count: number;
  runtime_link_active: boolean;
  supports_runtime_control: boolean;
};

export type CoreAdminRuntimesResult = CoreBridgeStatus & {
  runtimes: CoreAdminRuntimeOverview[] | null;
  error: string | null;
};

export type CoreLaunchCodeBatch = {
  id: string;
  name: string;
  hosting_tier?: CoreHostingTier | null;
  code_count: number;
  expires_at: string;
  revoked_at?: string | null;
  revoked_by_workos_user_id?: string | null;
  created_by_workos_user_id: string;
  created_at: string;
};

export type CoreHostingTier = "standard" | "confidential";

/** Metadata-only code status returned by later reads and revocation. */
export type CoreLaunchCodeStatus = {
  id: string;
  redeemed_customer_org_id?: string | null;
  redeemed_at?: string | null;
};

export type CoreLaunchCodeBatchDetails = {
  batch: CoreLaunchCodeBatch;
  codes: CoreLaunchCodeStatus[];
};

/** Plaintext values exist only in the immediate issuance response. */
export type CoreIssuedLaunchCodeBatch = {
  batch: CoreLaunchCodeBatch;
  codes: Array<{ id: string; code: string }>;
};

export type CoreLaunchCodeBatchesResult = CoreBridgeStatus & {
  batches: CoreLaunchCodeBatchDetails[] | null;
  error: string | null;
};

/** Raw key is present exactly once in this response and is never persisted. */
export type CoreAdminIssuedFinitePrivateKey = {
  grant?: CoreFinitePrivateGrant | null;
  api_key: CoreFinitePrivateApiKey;
  raw_api_key: string;
  raw_api_key_note: string;
};

export type CoreBillingSubscriptionStatus =
  | "incomplete"
  | "incomplete_expired"
  | "trialing"
  | "active"
  | "past_due"
  | "canceled"
  | "unpaid"
  | "paused";

export type CoreCustomerOrganization = {
  id: string;
  owner_user_id: string;
  name: string;
  billing_class: "grandfathered" | "sponsored" | "standard";
  created_at: string;
  updated_at: string;
};

export type CoreCustomerBillingAccount = {
  customer_org_id: string;
  stripe_customer_id?: string | null;
  stripe_subscription_id?: string | null;
  stripe_price_id?: string | null;
  subscription_status?: CoreBillingSubscriptionStatus | null;
  current_period_end?: string | null;
  cancel_at_period_end: boolean;
  last_stripe_event_id?: string | null;
  created_at: string;
  updated_at: string;
};

export type CoreAgentCreationEntitlement = {
  id: string;
  customer_org_id: string;
  allowed_new_agent_runtimes: number;
  launch_code?: string | null;
  created_at: string;
  updated_at: string;
};

export type CoreBillingOverview = {
  customer_org: CoreCustomerOrganization;
  billing_account?: CoreCustomerBillingAccount | null;
  agent_creation_entitlement?: CoreAgentCreationEntitlement | null;
  can_create_agent: boolean;
  requires_billing: boolean;
};

export type CoreBillingOverviewResult = CoreBridgeStatus & {
  account: AccountAuthContext;
  billing: CoreBillingOverview | null;
  error: string | null;
};

export type CoreMe = {
  email: string;
  workos_user_id: string;
  claimable_candidates: CoreProjectImportCandidate[];
  projects: CoreVisibleProject[];
  agent_creation_requests: CoreAgentCreationRequestSummary[];
};

export type CoreMeResult = {
  configured: boolean;
  missing: string[];
  account: AccountAuthContext;
  me: CoreMe | null;
  error: string | null;
};

export type CoreRuntimeRouteResolution = {
  project_id: string;
  runtime_id: string;
};

const REQUIRED_CORE_ENV = ["FC_CORE_BASE_URL"] as const;
const REQUIRED_CORE_SERVICE_ENV = ["FC_CORE_BASE_URL", "FC_CORE_API_TOKEN"] as const;
const CORE_CACHE_PREFIX = "core:";
const CORE_ME_FRESH_MS = 5_000;
const CORE_ME_STALE_MS = 30_000;
const CORE_SERVICE_FRESH_MS = 10_000;
const CORE_SERVICE_STALE_MS = 60_000;

type EnvSource = Record<string, string | undefined>;
export type CoreReadCacheMode = "fresh" | "swr";
export type CoreReadOptions = {
  cacheMode?: CoreReadCacheMode;
};

export function coreBridgeStatus(env: EnvSource = process.env): CoreBridgeStatus {
  const missing = REQUIRED_CORE_ENV.filter((name) => !env[name]?.trim());
  return {
    configured: missing.length === 0,
    missing,
  };
}

function coreServiceBridgeStatus(env: EnvSource = process.env): CoreBridgeStatus {
  const missing = REQUIRED_CORE_SERVICE_ENV.filter((name) => !env[name]?.trim());
  return {
    configured: missing.length === 0,
    missing,
  };
}

export async function loadCoreMe(options: CoreReadOptions = {}): Promise<CoreMeResult> {
  const status = coreBridgeStatus();
  const account = await getAccountAuthContext();
  if (!status.configured) {
    return {
      ...status,
      account,
      me: null,
      error: null,
    };
  }

  if (!coreAccountReady(account)) {
    return {
      ...status,
      account,
      me: null,
      error: "Sign in again to view your projects.",
    };
  }

  try {
    const load = () => coreFetch<CoreMe>("/api/core/v1/me", account);
    return {
      ...status,
      account,
      me:
        options.cacheMode === "swr"
          ? await readThroughServerSwr(
              `${CORE_CACHE_PREFIX}me:${coreCacheFingerprint(accountCacheParts(account))}`,
              { freshMs: CORE_ME_FRESH_MS, staleMs: CORE_ME_STALE_MS },
              load
            )
          : await load(),
      error: null,
    };
  } catch (error) {
    return {
      ...status,
      account,
      me: null,
      error: error instanceof Error ? error.message : "Finite Core is unavailable.",
    };
  }
}

export async function loadCoreBillingOverview(
  options: CoreReadOptions = {}
): Promise<CoreBillingOverviewResult> {
  const status = coreBridgeStatus();
  const account = await getAccountAuthContext();
  if (!status.configured) {
    return {
      ...status,
      account,
      billing: null,
      error: null,
    };
  }

  if (!coreAccountReady(account)) {
    return {
      ...status,
      account,
      billing: null,
      error: "Sign in again to view billing.",
    };
  }

  try {
    const load = () => coreFetch<CoreBillingOverview>("/api/core/v1/me/billing", account);
    return {
      ...status,
      account,
      billing:
        options.cacheMode === "swr"
          ? await readThroughServerSwr(
              `${CORE_CACHE_PREFIX}billing:${coreCacheFingerprint(accountCacheParts(account))}`,
              { freshMs: CORE_ME_FRESH_MS, staleMs: CORE_ME_STALE_MS },
              load
            )
          : await load(),
      error: null,
    };
  } catch (error) {
    return {
      ...status,
      account,
      billing: null,
      error: error instanceof Error ? error.message : "Finite Core is unavailable.",
    };
  }
}

export type CoreAgentCreationInput = {
  displayName: string;
  launchCode: string;
  idempotencyKey: string;
  profilePictureUrl?: string | null;
};

export function coreAgentCreationRequestBody(input: CoreAgentCreationInput) {
  const profilePictureUrl = optionalString(input.profilePictureUrl);
  return {
    displayName: input.displayName,
    launchCode: input.launchCode,
    idempotencyKey: input.idempotencyKey,
    ...(profilePictureUrl ? { profilePictureUrl } : {}),
  };
}

export async function requestCoreAgentCreation(input: CoreAgentCreationInput) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to create an agent.");
  }

  const result = await coreFetch<CoreAgentCreationResult>(
    "/api/core/v1/me/agent-creation-requests",
    account,
    {
      method: "POST",
      body: JSON.stringify(coreAgentCreationRequestBody(input)),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function linkCoreStripeCustomer(stripeCustomerId: string) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to manage billing.");
  }
  const result = await coreFetch<CoreCustomerBillingAccount>(
    "/api/core/v1/me/billing/stripe-customer",
    account,
    {
      method: "POST",
      body: JSON.stringify({
        stripeCustomerId: requiredString(stripeCustomerId, "Stripe customer id is required."),
      }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function syncCoreStripeSubscription(input: {
  customerOrgId?: string | null;
  stripeCustomerId: string;
  stripeSubscriptionId: string;
  stripePriceId?: string | null;
  subscriptionStatus: CoreBillingSubscriptionStatus;
  currentPeriodEnd?: string | null;
  cancelAtPeriodEnd: boolean;
  stripeEventId?: string | null;
  stripeEventCreated?: number | null;
}) {
  const result = await coreServiceFetch<CoreCustomerBillingAccount>(
    "/api/core/v1/billing/stripe/subscription",
    {
      method: "POST",
      body: JSON.stringify({
        customerOrgId: optionalString(input.customerOrgId),
        stripeCustomerId: requiredString(input.stripeCustomerId, "Stripe customer id is required."),
        stripeSubscriptionId: requiredString(
          input.stripeSubscriptionId,
          "Stripe subscription id is required."
        ),
        stripePriceId: optionalString(input.stripePriceId),
        subscriptionStatus: input.subscriptionStatus,
        currentPeriodEnd: optionalString(input.currentPeriodEnd),
        cancelAtPeriodEnd: input.cancelAtPeriodEnd,
        stripeEventId: optionalString(input.stripeEventId),
        stripeEventCreated: input.stripeEventCreated ?? null,
      }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function requestCoreRuntimeRestart(projectId: string) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to restart your agent.");
  }

  const result = await coreFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/me/projects/${encodeURIComponent(projectId)}/runtime/restart`,
    account,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function requestCoreRuntimeRecoverKnownGoodChat(projectId: string) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to recover your agent.");
  }

  const result = await coreFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/me/projects/${encodeURIComponent(projectId)}/runtime/recover-known-good-chat`,
    account,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function requestCoreRuntimeStop(projectId: string) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to stop your agent.");
  }

  const result = await coreFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/me/projects/${encodeURIComponent(projectId)}/runtime/stop`,
    account,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function requestCoreRuntimeDestroy(projectId: string) {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to manage your agent.");
  }

  const result = await coreFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/me/projects/${encodeURIComponent(projectId)}/runtime/destroy`,
    account,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function cancelFailedCoreAgentCreationRequest(requestId: string) {
  const trimmed = requestId.trim();
  if (!trimmed) {
    throw new Error("Missing agent creation request id.");
  }

  const result = await coreServiceFetch<CoreAgentCreationRequest>(
    `/api/core/v1/agent-creation-requests/${encodeURIComponent(trimmed)}/cancel`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function resolveCoreRuntimeRoute(identifier: string) {
  const routeIdentifier = identifier.trim();
  if (!routeIdentifier) return null;
  const status = coreBridgeStatus();
  if (!status.configured) return null;
  const account = await getAccountAuthContext();
  if (!coreAccountReady(account)) return null;

  try {
    return await coreFetch<CoreRuntimeRouteResolution>(
      `/api/core/v1/me/runtime-routes/${encodeURIComponent(routeIdentifier)}`,
      account
    );
  } catch (error) {
    if (error instanceof CoreFetchError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function loadCoreSourceHostRelayEndpoint(
  sourceHostId: string,
  options: CoreReadOptions = {}
) {
  const hostId = sourceHostId.trim().toLowerCase();
  if (!hostId) {
    return null;
  }
  const status = coreServiceBridgeStatus();
  if (!status.configured) {
    return null;
  }

  try {
    const pathname = `/api/core/v1/source-host-relays/${encodeURIComponent(hostId)}`;
    const load = () => coreServiceFetch<CoreSourceHostRelayEndpoint>(pathname);
    return options.cacheMode === "swr"
      ? await readThroughServerSwr(
          `${CORE_CACHE_PREFIX}source-host-relay:${coreCacheFingerprint(coreServiceCacheParts(hostId))}`,
          { freshMs: CORE_SERVICE_FRESH_MS, staleMs: CORE_SERVICE_STALE_MS },
          load
        )
      : await load();
  } catch (error) {
    if (error instanceof CoreFetchError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function loadCoreFinitePrivateAdminState(
  options: CoreReadOptions = {}
): Promise<CoreFinitePrivateAdminStateResult> {
  const status = coreBridgeStatus();
  if (!status.configured) {
    return {
      ...status,
      state: null,
      error: null,
    };
  }

  try {
    const load = () => coreAdminFetch<CoreFinitePrivateAdminState>("/api/core/v1/finite-private/admin-state");
    return {
      ...status,
      state:
        options.cacheMode === "swr"
          ? await readThroughServerSwr(
              `${CORE_CACHE_PREFIX}finite-private-admin:${coreCacheFingerprint(accountCacheParts(await getAccountAuthContext()))}`,
              { freshMs: CORE_SERVICE_FRESH_MS, staleMs: CORE_SERVICE_STALE_MS },
              load
            )
          : await load(),
      error: null,
    };
  } catch (error) {
    return {
      ...status,
      state: null,
      error: error instanceof Error ? error.message : "Finite Core is unavailable.",
    };
  }
}

export async function approveCoreFinitePrivateGrant(input: {
  verifiedEmail: string;
  workosUserId?: string | null;
  limitProfileId?: string | null;
}) {
  const result = await coreAdminFetch<CoreFinitePrivateGrant>("/api/core/v1/finite-private/grants", {
    method: "POST",
    body: JSON.stringify({
      verifiedEmail: requiredString(input.verifiedEmail, "Verified email is required."),
      workosUserId: optionalString(input.workosUserId),
      limitProfileId: optionalString(input.limitProfileId),
    }),
  });
  invalidateCoreReadCache();
  return result;
}

export async function issueCoreFinitePrivateApiKey(input: {
  grantId: string;
  rawKey: string;
  projectId?: string | null;
  agentRuntimeId?: string | null;
}) {
  const grantId = requiredString(input.grantId, "Grant id is required.");
  const result = await coreAdminFetch<CoreFinitePrivateApiKey>(
    `/api/core/v1/finite-private/grants/${encodeURIComponent(grantId)}/api-keys`,
    {
      method: "POST",
      body: JSON.stringify({
        rawKey: requiredString(input.rawKey, "Raw Finite Private key is required."),
        projectId: optionalString(input.projectId),
        agentRuntimeId: optionalString(input.agentRuntimeId),
      }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function resetCoreFinitePrivateGrant(grantId: string) {
  const result = await coreAdminFetch<CoreFinitePrivateGrant>(
    `/api/core/v1/finite-private/grants/${encodeURIComponent(
      requiredString(grantId, "Grant id is required.")
    )}/reset`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function revokeCoreFinitePrivateGrant(grantId: string) {
  const result = await coreAdminFetch<CoreFinitePrivateGrant>(
    `/api/core/v1/finite-private/grants/${encodeURIComponent(
      requiredString(grantId, "Grant id is required.")
    )}/revoke`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function rotateCoreFinitePrivateApiKey(input: {
  keyId: string;
  rawKey: string;
}) {
  const keyId = requiredString(input.keyId, "API key id is required.");
  const result = await coreAdminFetch<CoreFinitePrivateApiKey>(
    `/api/core/v1/finite-private/api-keys/${encodeURIComponent(keyId)}/rotate`,
    {
      method: "POST",
      body: JSON.stringify({
        rawKey: requiredString(input.rawKey, "Replacement raw Finite Private key is required."),
      }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function revokeCoreFinitePrivateApiKey(keyId: string) {
  const result = await coreAdminFetch<CoreFinitePrivateApiKey>(
    `/api/core/v1/finite-private/api-keys/${encodeURIComponent(
      requiredString(keyId, "API key id is required.")
    )}/revoke`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
  invalidateCoreReadCache();
  return result;
}

// --- Admin Ops ---
//
// Core validates the signed-in administrator's WorkOS AuthKit access token and
// operator organization. The dashboard isAdmin gate is only a UI convenience.

async function coreAdminFetch<T>(pathname: string, init: RequestInit = {}): Promise<T> {
  const status = coreBridgeStatus();
  if (!status.configured) {
    throw new Error(`Finite Core is not configured: ${status.missing.join(", ")}`);
  }
  const account = await getAccountAuthContext();
  return coreFetch<T>(pathname, account, init);
}

export async function loadCoreAdminRuntimes(): Promise<CoreAdminRuntimesResult> {
  const status = coreBridgeStatus();
  if (!status.configured) {
    return { ...status, runtimes: null, error: null };
  }

  try {
    return {
      ...status,
      runtimes: await coreAdminFetch<CoreAdminRuntimeOverview[]>(
        "/api/core/v1/admin/runtimes"
      ),
      error: null,
    };
  } catch (error) {
    return {
      ...status,
      runtimes: null,
      error: error instanceof Error ? error.message : "Finite Core is unavailable.",
    };
  }
}

export async function loadCoreLaunchCodeBatches(): Promise<CoreLaunchCodeBatchesResult> {
  const status = coreBridgeStatus();
  if (!status.configured) {
    return { ...status, batches: null, error: null };
  }

  try {
    return {
      ...status,
      batches: await coreAdminFetch<CoreLaunchCodeBatchDetails[]>(
        "/api/core/v1/admin/launch-code-batches"
      ),
      error: null,
    };
  } catch (error) {
    return {
      ...status,
      batches: null,
      error: error instanceof Error ? error.message : "Finite Core is unavailable.",
    };
  }
}

export async function adminIssueCoreLaunchCodeBatch(input: {
  name: string;
  codeCount: number;
  expiresInHours?: number | null;
  hostingTier?: CoreHostingTier | null;
}) {
  const result = await coreAdminFetch<CoreIssuedLaunchCodeBatch>(
    "/api/core/v1/admin/launch-code-batches",
    {
      method: "POST",
      body: JSON.stringify(coreLaunchCodeBatchRequestBody(input)),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export function coreLaunchCodeBatchRequestBody(input: {
  name: string;
  codeCount: number;
  expiresInHours?: number | null;
  hostingTier?: CoreHostingTier | null;
}) {
  return {
    name: requiredString(input.name, "Batch name is required."),
    codeCount: input.codeCount,
    expiresInHours: input.expiresInHours ?? undefined,
    hostingTier: input.hostingTier ?? "standard",
  };
}

export async function adminRevokeCoreLaunchCodeBatch(batchId: string) {
  const result = await coreAdminFetch<CoreLaunchCodeBatchDetails>(
    `/api/core/v1/admin/launch-code-batches/${encodeURIComponent(
      requiredString(batchId, "Launch Code batch id is required.")
    )}/revoke`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminRestartCoreRuntime(projectId: string) {
  const result = await coreAdminFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/admin/projects/${encodeURIComponent(
      requiredString(projectId, "Project id is required.")
    )}/runtime/restart`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminRecoverCoreRuntime(projectId: string) {
  const result = await coreAdminFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/admin/projects/${encodeURIComponent(
      requiredString(projectId, "Project id is required.")
    )}/runtime/recover-known-good-chat`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminUpgradeCoreRuntime(input: {
  projectId: string;
  targetRuntimeArtifactId: string;
}) {
  const projectId = requiredString(input.projectId, "Project id is required.");
  const targetRuntimeArtifactId = requiredString(
    input.targetRuntimeArtifactId,
    "Target runtime artifact id is required."
  );
  const result = await coreAdminFetch<CoreRuntimeControlRequest>(
    `/api/core/v1/admin/projects/${encodeURIComponent(projectId)}/runtime/upgrade`,
    {
      method: "POST",
      body: JSON.stringify({ targetRuntimeArtifactId }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminIssueCoreFinitePrivateFriendKey(input: {
  email: string;
  limitProfileId?: string | null;
}) {
  const result = await coreAdminFetch<CoreAdminIssuedFinitePrivateKey>(
    "/api/core/v1/admin/finite-private/friend-keys",
    {
      method: "POST",
      body: JSON.stringify({
        email: requiredString(input.email, "Friend email is required."),
        limitProfileId: optionalString(input.limitProfileId),
      }),
    }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminRotateCoreFinitePrivateApiKey(keyId: string) {
  const result = await coreAdminFetch<CoreAdminIssuedFinitePrivateKey>(
    `/api/core/v1/admin/finite-private/keys/${encodeURIComponent(
      requiredString(keyId, "API key id is required.")
    )}/rotate`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminRevokeCoreFinitePrivateApiKey(keyId: string) {
  const result = await coreAdminFetch<CoreFinitePrivateApiKey>(
    `/api/core/v1/admin/finite-private/keys/${encodeURIComponent(
      requiredString(keyId, "API key id is required.")
    )}/revoke`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export async function adminResetCoreFinitePrivateWindow(grantId: string) {
  const result = await coreAdminFetch<CoreFinitePrivateGrant>(
    `/api/core/v1/admin/finite-private/grants/${encodeURIComponent(
      requiredString(grantId, "Grant id is required.")
    )}/window-reset`,
    { method: "POST", body: JSON.stringify({}) }
  );
  invalidateCoreReadCache();
  return result;
}

export function coreProjectRuntimeId(project: CoreVisibleProject) {
  return project.runtime?.id.trim() || null;
}

export function coreProductProjects(projects: CoreVisibleProject[]) {
  return projects.filter((project) => {
    const legacyProject = project.project as CoreProject & {
      import_candidate_id?: unknown;
    };
    return legacyProject.import_candidate_id == null;
  });
}

export function coreProductProjectForRouteId(
  projects: CoreVisibleProject[],
  identifier: string
) {
  return (
    coreProductProjects(projects).find(
      (project) =>
        project.runtime?.id === identifier || project.project.id === identifier
    ) ?? null
  );
}

/**
 * N-1 Dashboard compatibility only: an older Core may still return the former
 * source-machine field. It is read on the server solely to recover an old
 * bookmark and is never copied into public Dashboard props or navigation.
 */
export function coreProductProjectForLegacyMachineId(
  projects: CoreVisibleProject[],
  legacyMachineId: string
) {
  return (
    coreProductProjects(projects).find((project) => {
      const legacyRuntime = project.runtime as (CoreAgentRuntime & {
        source_machine_id?: unknown;
      }) | null | undefined;
      return legacyRuntime?.source_machine_id === legacyMachineId;
    }) ?? null
  );
}

export function coreProjectSupportsHostedRuntimeControl(project: CoreVisibleProject) {
  const capabilities = project.runtime?.runtime_capabilities;
  return Boolean(
    capabilities?.restart === true ||
    capabilities?.recover_known_good_chat === true ||
    capabilities?.stop === true
  );
}

export function coreProjectSupportsHostedRestart(project: CoreVisibleProject) {
  return project.runtime?.runtime_capabilities?.restart === true;
}

export function coreProjectSupportsHostedRecovery(project: CoreVisibleProject) {
  return project.runtime?.runtime_capabilities?.recover_known_good_chat === true;
}

export function coreProjectSupportsHostedStop(project: CoreVisibleProject) {
  return project.runtime?.runtime_capabilities?.stop === true;
}

export function coreProjectSupportsRetirement(project: CoreVisibleProject) {
  return project.runtime?.runtime_capabilities?.runtime_retirement === true;
}

export function coreProjectLabel(project: CoreVisibleProject) {
  return project.project.display_name.trim() || "Agent";
}

export function coreProjectPrimaryUrl(project: CoreVisibleProject) {
  const endpoint = project.runtime?.contact_endpoint?.trim();
  return endpoint && safeHttpUrl(endpoint) ? endpoint : null;
}

export function coreAgentCreationRequestForProject(
  project: CoreVisibleProject,
  requests: CoreAgentCreationRequestSummary[]
) {
  return (
    requests.find((request) => request.project_id === project.project.id) ?? null
  );
}

export function coreProjectLaunchStatusLabel(
  project: CoreVisibleProject,
  request: CoreAgentCreationRequestSummary | null
) {
  const runtimeStatus = project.runtime?.runtime_status;
  if (runtimeStatus === "online") {
    return "Online";
  }
  if (runtimeStatus === "offline") {
    return "Offline";
  }
  if (runtimeStatus === "stale") {
    return "Needs attention";
  }
  if (request?.status === "requested") {
    return "Queued";
  }
  if (request?.status === "launching") {
    return "Starting";
  }
  if (request?.status === "failed") {
    return "Launch failed";
  }
  return null;
}

export function coreProjectLocationLabel(
  project: CoreVisibleProject,
  request: CoreAgentCreationRequestSummary | null
) {
  if (project.runtime) {
    return "Ready to use";
  }
  if (request?.status === "requested") {
    return "Waiting for launch";
  }
  if (request?.status === "launching") {
    return "Starting your agent";
  }
  if (request?.status === "failed") {
    return "Launch failed";
  }
  return "Waiting for launch";
}

export function coreIdentityHeaders(account: AccountAuthContext) {
  if (!coreAccountReady(account)) {
    throw new Error("Sign in again to continue.");
  }

  return {
    authorization: `Bearer ${account.accessToken}`,
    "content-type": "application/json",
  };
}

function coreAccountReady(
  account: AccountAuthContext
): account is AccountAuthContext & {
  email: string;
  workosUserId: string;
  emailVerified: true;
  accessToken: string;
} {
  return Boolean(
    account.email &&
      account.workosUserId &&
      account.emailVerified &&
      (account.source === "workos" || account.source === "dev") &&
      account.accessToken
  );
}

async function coreFetch<T>(
  pathname: string,
  account: AccountAuthContext,
  init: RequestInit = {}
): Promise<T> {
  const baseUrl = process.env.FC_CORE_BASE_URL?.trim();
  if (!baseUrl) {
    throw new Error("Finite Core is not configured.");
  }

  const response = await fetch(new URL(pathname, baseUrl), {
    ...init,
    cache: "no-store",
    headers: {
      ...coreIdentityHeaders(account),
      ...headersRecord(init.headers),
    },
  });
  const text = await response.text();
  const parsed = parseCoreResponseText(text, response.ok, response.status);
  if (!response.ok) {
    const message =
      parsed && typeof parsed === "object" && "error" in parsed && typeof parsed.error === "string"
        ? parsed.error
        : `Finite Core returned ${response.status}`;
    throw new Error(message);
  }
  return parsed as T;
}

class CoreFetchError extends Error {
  constructor(message: string, public readonly status: number) {
    super(message);
    this.name = "CoreFetchError";
  }
}

async function coreServiceFetch<T>(
  pathname: string,
  init: RequestInit = {}
): Promise<T> {
  const baseUrl = process.env.FC_CORE_BASE_URL?.trim();
  const token = process.env.FC_CORE_API_TOKEN?.trim();
  if (!baseUrl || !token) {
    throw new Error("Finite Core is not configured.");
  }

  const response = await fetch(new URL(pathname, baseUrl), {
    ...init,
    cache: "no-store",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
      ...headersRecord(init.headers),
    },
  });
  const text = await response.text();
  const parsed = parseCoreResponseText(text, response.ok, response.status);
  if (!response.ok) {
    const message =
      parsed && typeof parsed === "object" && "error" in parsed && typeof parsed.error === "string"
        ? parsed.error
        : `Finite Core returned ${response.status}`;
    throw new CoreFetchError(message, response.status);
  }
  return parsed as T;
}

function parseCoreResponseText(text: string, ok: boolean, status: number) {
  try {
    return text ? JSON.parse(text) : {};
  } catch {
    if (!ok) {
      throw new Error(text ? `${text.trim().slice(0, 500)} (${status})` : `Finite Core returned ${status}`);
    }
    throw new Error(`Finite Core returned non-JSON response (${status})`);
  }
}

function headersRecord(headers: HeadersInit | undefined) {
  if (!headers) {
    return {};
  }
  return Object.fromEntries(new Headers(headers).entries());
}

function requiredString(value: string | null | undefined, message: string) {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) {
    throw new Error(message);
  }
  return trimmed;
}

function optionalString(value: string | null | undefined) {
  const trimmed = value?.trim() ?? "";
  return trimmed || null;
}

export function invalidateCoreReadCache() {
  invalidateServerSwrCache(CORE_CACHE_PREFIX);
}

function accountCacheParts(account: AccountAuthContext) {
  return [
    process.env.FC_CORE_BASE_URL?.trim() ?? "",
    account.source,
    account.accessToken ?? "",
    account.workosUserId ?? "",
    account.email ?? "",
    account.emailVerified ? "verified" : "unverified",
  ];
}

function coreServiceCacheParts(...parts: string[]) {
  return [
    process.env.FC_CORE_BASE_URL?.trim() ?? "",
    process.env.FC_CORE_API_TOKEN?.trim() ?? "",
    ...parts,
  ];
}

function coreCacheFingerprint(parts: string[]) {
  return createHash("sha256").update(parts.join("\0")).digest("hex").slice(0, 32);
}

function safeHttpUrl(value: string) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}
