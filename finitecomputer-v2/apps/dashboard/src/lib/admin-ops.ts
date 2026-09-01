// Pure helpers for the Admin Ops page (/dashboard/admin).
//
// The dashboard admin gate here is UI-only. Core independently enforces every
// admin mutation against its validated WorkOS operator organization.

export type AdminOpsViewer = {
  isAdmin: boolean;
};

export const FINITE_PRIVATE_1X_PROFILE_ID = "finite-private-generous-v2";
export const FINITE_PRIVATE_5X_PROFILE_ID = "finite-private-generous-5x-v1";

export type FinitePrivateProfileOption = {
  id: string;
  burst_limit_units: number;
};

export function finitePrivateAssignableProfiles<T extends FinitePrivateProfileOption>(
  profiles: T[] | null | undefined
): T[] {
  const byId = new Map((profiles ?? []).map((profile) => [profile.id, profile]));
  return [FINITE_PRIVATE_1X_PROFILE_ID, FINITE_PRIVATE_5X_PROFILE_ID]
    .map((id) => byId.get(id))
    .filter((profile): profile is T => Boolean(profile));
}

export function finitePrivateProfileLabel(profileId: string): string {
  if (profileId === FINITE_PRIVATE_1X_PROFILE_ID) return "1× · 100M units / 5h";
  if (profileId === FINITE_PRIVATE_5X_PROFILE_ID) return "5× · 500M units / 5h";
  return profileId;
}

export function finitePrivateAccountForProject<
  T extends { projects: Array<{ id: string }> },
>(accounts: T[] | null | undefined, projectId: string): T | null {
  return (
    (accounts ?? []).find((account) =>
      account.projects.some((project) => project.id === projectId)
    ) ?? null
  );
}

export function groupAdminRuntimesByOwner<
  T extends { owner_email?: string | null; project_id: string },
>(runtimes: T[]): Array<{ key: string; email: string | null; runtimes: T[] }> {
  const groups = new Map<string, { key: string; email: string | null; runtimes: T[] }>();
  for (const runtime of runtimes) {
    const email = runtime.owner_email?.trim().toLowerCase() || null;
    const key = email ?? `unknown:${runtime.project_id}`;
    const group = groups.get(key) ?? { key, email, runtimes: [] };
    group.runtimes.push(runtime);
    groups.set(key, group);
  }
  return [...groups.values()].sort((left, right) =>
    (left.email ?? left.key).localeCompare(right.email ?? right.key)
  );
}

export type AdminRuntimeCapabilities = {
  restart?: boolean;
  recover_known_good_chat?: boolean;
  runtime_upgrade?: boolean;
};

export type AdminUserSearchRuntime = {
  project_id: string;
  project_display_name: string;
  owner_email?: string | null;
  agent_runtime_id: string;
  source_host_id: string;
  source_machine_id: string;
  runtime_artifact_id?: string | null;
  runtime_artifact_version_label?: string | null;
  runtime_status: string;
  published_app_urls?: string[] | null;
  runtime_capabilities?: AdminRuntimeCapabilities | null;
};

export type AdminUserSearchAccount = {
  userId: string;
  email: string;
  grant: {
    id: string;
    user_id: string;
    limit_profile_id: string;
    status: string;
    current_window_used_units: number;
  };
  apiKeys: Array<{
    id: string;
    grant_id: string;
    project_id?: string | null;
    agent_runtime_id?: string | null;
    status: string;
  }>;
  projects: Array<{
    id: string;
    displayName: string;
    agentRuntimeId?: string | null;
  }>;
};

export type RuntimeFinitePrivateGrantSummary = {
  grantId: string;
  grantStatus: "active" | "revoked";
  grantUserId: string;
  limitProfileId: string;
  currentWindowStartedAt: string | null;
  currentWindowUsedUnits: number;
  keyId: string;
  keyStatus: "active" | "revoked";
  keyProjectId: string | null;
  keyAgentRuntimeId: string | null;
  matchScope: "runtime" | "project";
};

export type AdminRuntimeFinitePrivateTarget = {
  project_id: string;
  agent_runtime_id: string;
};

export type AdminRuntimeFinitePrivateGrant = {
  id: string;
  user_id: string;
  limit_profile_id: string;
  status: "active" | "revoked";
  current_window_started_at?: string | null;
  current_window_used_units: number;
};

export type AdminRuntimeFinitePrivateApiKey = {
  id: string;
  grant_id: string;
  project_id?: string | null;
  agent_runtime_id?: string | null;
  status: "active" | "revoked";
  updated_at: string;
};

export type AdminRuntimeFinitePrivateState = {
  grants: AdminRuntimeFinitePrivateGrant[];
  apiKeys: AdminRuntimeFinitePrivateApiKey[];
};

export function adminRuntimeSupportsRestart(
  runtime: { runtime_capabilities?: AdminRuntimeCapabilities | null } | null | undefined
) {
  return runtime?.runtime_capabilities?.restart === true;
}

export function adminRuntimeSupportsRecovery(
  runtime: { runtime_capabilities?: AdminRuntimeCapabilities | null } | null | undefined
) {
  return runtime?.runtime_capabilities?.recover_known_good_chat === true;
}

export function adminRuntimeSupportsUpgrade(
  runtime: { runtime_capabilities?: AdminRuntimeCapabilities | null } | null | undefined
) {
  return runtime?.runtime_capabilities?.runtime_upgrade === true;
}

/**
 * Resolve the Finite Private key/grant most relevant to an admin runtime row.
 * Runtime-scoped active keys win over project-scoped keys; otherwise the newest
 * matching key is used so a tile can still explain revoked/replaced state.
 */
export function finitePrivateGrantSummaryForRuntime(
  runtime: AdminRuntimeFinitePrivateTarget,
  state: AdminRuntimeFinitePrivateState | null | undefined,
): RuntimeFinitePrivateGrantSummary | null {
  if (!state) {
    return null;
  }
  const grantsById = new Map(state.grants.map((grant) => [grant.id, grant]));
  const matchingKeys = state.apiKeys
    .filter(
      (key) =>
        key.agent_runtime_id === runtime.agent_runtime_id ||
        key.project_id === runtime.project_id,
    )
    .sort(
      (left, right) =>
        runtimeKeySortScore(runtime, right) - runtimeKeySortScore(runtime, left),
    );
  const key = matchingKeys[0];
  if (!key) {
    return null;
  }
  const grant = grantsById.get(key.grant_id);
  if (!grant) {
    return null;
  }
  return {
    grantId: grant.id,
    grantStatus: grant.status,
    grantUserId: grant.user_id,
    limitProfileId: grant.limit_profile_id,
    currentWindowStartedAt: grant.current_window_started_at ?? null,
    currentWindowUsedUnits: grant.current_window_used_units,
    keyId: key.id,
    keyStatus: key.status,
    keyProjectId: key.project_id ?? null,
    keyAgentRuntimeId: key.agent_runtime_id ?? null,
    matchScope:
      key.agent_runtime_id === runtime.agent_runtime_id ? "runtime" : "project",
  };
}

function runtimeKeySortScore(
  runtime: AdminRuntimeFinitePrivateTarget,
  key: AdminRuntimeFinitePrivateApiKey,
) {
  let score = 0;
  if (key.status === "active") {
    score += 1_000_000_000_000_000;
  }
  if (key.agent_runtime_id === runtime.agent_runtime_id) {
    score += 1_000_000_000_000;
  } else if (key.project_id === runtime.project_id) {
    score += 500_000_000_000;
  }
  const updatedAt = Date.parse(key.updated_at);
  if (Number.isFinite(updatedAt)) {
    score += updatedAt;
  }
  return score;
}

export function adminRuntimeMatchesSearch(
  runtime: AdminUserSearchRuntime,
  account: AdminUserSearchAccount | null | undefined,
  query: string,
  finitePrivate: RuntimeFinitePrivateGrantSummary | null | undefined = null,
): boolean {
  const tokens = adminSearchTokens(query);
  if (tokens.length === 0) {
    return true;
  }
  const haystack = adminSearchText([
    runtime.project_id,
    runtime.project_display_name,
    runtime.owner_email,
    runtime.agent_runtime_id,
    runtime.source_host_id,
    runtime.source_machine_id,
    runtime.runtime_artifact_id,
    runtime.runtime_artifact_version_label,
    runtime.runtime_status,
    runtime.published_app_urls,
    account?.userId,
    account?.email,
    account?.grant.id,
    account?.grant.user_id,
    account?.grant.limit_profile_id,
    account?.grant.limit_profile_id
      ? finitePrivateProfileLabel(account.grant.limit_profile_id)
      : null,
    account?.grant.status,
    account?.grant.current_window_used_units,
    account?.apiKeys.map((apiKey) => [
      apiKey.id,
      apiKey.grant_id,
      apiKey.project_id,
      apiKey.agent_runtime_id,
      apiKey.status,
    ]),
    account?.projects.map((project) => [
      project.id,
      project.displayName,
      project.agentRuntimeId,
    ]),
    finitePrivate?.grantId,
    finitePrivate?.grantUserId,
    finitePrivate?.limitProfileId,
    finitePrivate?.limitProfileId
      ? finitePrivateProfileLabel(finitePrivate.limitProfileId)
      : null,
    finitePrivate?.grantStatus,
    finitePrivate?.currentWindowStartedAt,
    finitePrivate?.currentWindowUsedUnits,
    finitePrivate?.keyId,
    finitePrivate?.keyStatus,
    finitePrivate?.keyProjectId,
    finitePrivate?.keyAgentRuntimeId,
    finitePrivate?.matchScope,
  ]);
  return tokens.every((token) => haystack.includes(token));
}

export function canAccessAdminOps(viewer: AdminOpsViewer | null | undefined): boolean {
  return Boolean(viewer?.isAdmin);
}

/** Human label for how long ago a runtime last heartbeated. */
export function heartbeatAgeLabel(
  lastHeartbeatAt: string | null | undefined,
  nowMs: number,
): string {
  if (!lastHeartbeatAt) {
    return "never";
  }
  const heartbeatMs = Date.parse(lastHeartbeatAt);
  if (!Number.isFinite(heartbeatMs)) {
    return "unknown";
  }
  const deltaMs = nowMs - heartbeatMs;
  if (deltaMs < 0) {
    return "just now";
  }
  const seconds = Math.floor(deltaMs / 1000);
  if (seconds < 60) {
    return `${seconds}s ago`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 48) {
    return `${hours}h ago`;
  }
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/**
 * State returned by the one-time key server actions. The raw key only ever
 * lives in this in-memory action state; it is never persisted, logged, or
 * shown again after the page state is replaced.
 */
export type OneTimeKeyActionState =
  | { status: "idle" }
  | { status: "error"; error: string }
  | {
      status: "issued";
      keyId: string;
      grantId: string | null;
      rawKey: string;
      note: string;
    };

export const ONE_TIME_KEY_WARNING =
  "You will not see this key again. Copy it now and hand it off securely.";

export type OneTimeKeyDisplay = {
  keyId: string;
  grantId: string | null;
  rawKey: string;
  warning: string;
};

/**
 * One-time display model: only an `issued` action state with non-empty raw
 * key material produces a display; everything else renders nothing.
 */
export function oneTimeKeyDisplay(
  state: OneTimeKeyActionState | null | undefined,
): OneTimeKeyDisplay | null {
  if (!state || state.status !== "issued") {
    return null;
  }
  const rawKey = state.rawKey.trim();
  if (!rawKey) {
    return null;
  }
  return {
    keyId: state.keyId,
    grantId: state.grantId,
    rawKey,
    warning: state.note.trim() || ONE_TIME_KEY_WARNING,
  };
}

export function oneTimeKeyError(
  state: OneTimeKeyActionState | null | undefined,
): string | null {
  if (!state || state.status !== "error") {
    return null;
  }
  return state.error.trim() || "The admin action failed.";
}

export type LaunchCodeBatchFormInput = {
  name: string;
  codeCount: number;
  expiresInHours?: number;
  hostingTier: LaunchCodeHostingTier;
};

export type LaunchCodeHostingTier = "standard" | "confidential";

/**
 * Validate the intentionally small operator form before it reaches Core. Core
 * repeats these checks; this keeps accidental blank, indefinite, or oversized
 * issuance out of the normal UI path.
 *
 * Issuance is standard-only while the confidential lane has no deployed
 * runner (Core rejects the tier server-side too); `launchCodeHostingTier`
 * below still parses `confidential` so pre-existing batch rows keep listing.
 */
export function launchCodeBatchFormInput(formData: FormData): LaunchCodeBatchFormInput {
  const name = String(formData.get("name") ?? "").trim();
  if (!name) {
    throw new Error("Batch name is required.");
  }
  if (name.length > 120 || /[\u0000-\u001f\u007f]/u.test(name)) {
    throw new Error("Batch name is invalid.");
  }

  const codeCount = boundedWholeNumber(formData.get("codeCount"), 1, 1_000, "Code count");
  const expiryValue = String(formData.get("expiresInHours") ?? "").trim();
  const expiresInHours = expiryValue
    ? boundedWholeNumber(expiryValue, 1, 720, "Expiry hours")
    : undefined;
  const hostingTier = launchCodeHostingTier(formData.get("hostingTier"));
  if (hostingTier === "confidential") {
    throw new Error("Confidential hosting is not currently available. Issue Standard codes.");
  }
  return { name, codeCount, expiresInHours, hostingTier };
}

export function launchCodeHostingTier(value: FormDataEntryValue | string | null | undefined): LaunchCodeHostingTier {
  const normalized = String(value ?? "").trim().toLowerCase();
  if (!normalized || normalized === "standard") {
    return "standard";
  }
  if (normalized === "confidential") {
    return "confidential";
  }
  throw new Error("Hosting tier must be Standard or Confidential.");
}

export function launchCodeHostingTierLabel(value: string | null | undefined) {
  return launchCodeHostingTier(value) === "confidential" ? "Confidential" : "Standard";
}

function boundedWholeNumber(
  value: FormDataEntryValue | string | null,
  minimum: number,
  maximum: number,
  label: string
) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${label} must be a whole number from ${minimum} to ${maximum}.`);
  }
  return parsed;
}

function adminSearchTokens(query: string) {
  return query.trim().toLowerCase().split(/\s+/u).filter(Boolean);
}

function adminSearchText(values: unknown[]): string {
  return values.map(adminSearchValue).filter(Boolean).join(" ").toLowerCase();
}

function adminSearchValue(value: unknown): string {
  if (value == null) {
    return "";
  }
  if (Array.isArray(value)) {
    return value.map(adminSearchValue).filter(Boolean).join(" ");
  }
  return String(value);
}

export type OneTimeLaunchCodeActionState =
  | { status: "idle" }
  | { status: "error"; error: string }
  | {
      status: "issued";
      batch: {
        id: string;
        name: string;
        codeCount: number;
        expiresAt: string;
        hostingTier: LaunchCodeHostingTier;
      };
      codes: Array<{ id: string; code: string }>;
    };

/** Plaintext values only, one per line, for the client-created one-time file. */
export function launchCodeDownloadText(codes: Array<{ code: string }>) {
  return `${codes.map((entry) => entry.code.trim()).filter(Boolean).join("\n")}\n`;
}

export function launchCodeDownloadFilename(name: string) {
  const normalized = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, "-")
    .replace(/(^-|-$)/gu, "")
    .slice(0, 80);
  return `${normalized || "launch-code-batch"}-codes.txt`;
}
