// Pure helpers for the Admin Ops page (/dashboard/admin).
//
// The dashboard admin gate here is UI-only. Core independently enforces every
// admin mutation against its validated WorkOS operator organization.

export type AdminOpsViewer = {
  isAdmin: boolean;
};

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

export type AdminOpsRuntime = {
  project_display_name: string;
  owner_email?: string | null;
  project_id: string;
  agent_runtime_id: string;
  source_host_id: string;
  source_machine_id: string;
  runtime_artifact_id?: string | null;
  runtime_artifact_version_label?: string | null;
  runtime_status: string;
  last_heartbeat_at?: string | null;
  hermes_available?: boolean | null;
  published_app_urls: string[];
  active_finite_private_key_count: number;
  runtime_link_active: boolean;
  runtime_capabilities?: {
    restart?: boolean;
    recover_known_good_chat?: boolean;
    runtime_upgrade?: boolean;
  } | null;
};

export type AdminOpsFinitePrivateGrant = {
  id: string;
  user_id: string;
  limit_profile_id: string;
  status: "active" | "revoked";
  current_window_started_at?: string | null;
  current_window_used_units: number;
};

export type AdminOpsFinitePrivateApiKey = {
  id: string;
  grant_id: string;
  project_id?: string | null;
  agent_runtime_id?: string | null;
  status: "active" | "revoked";
  updated_at: string;
};

export type AdminOpsFinitePrivateState = {
  grants: AdminOpsFinitePrivateGrant[];
  apiKeys: AdminOpsFinitePrivateApiKey[];
};

/**
 * Resolve the Finite Private key/grant most relevant to an admin runtime row.
 * Runtime-scoped active keys win over project-scoped keys; otherwise the newest
 * matching key is used so a tile can still explain revoked/replaced state.
 */
export function finitePrivateGrantSummaryForRuntime(
  runtime: AdminOpsRuntime,
  state: AdminOpsFinitePrivateState | null | undefined,
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
    .sort((left, right) => runtimeKeySortScore(runtime, right) - runtimeKeySortScore(runtime, left));
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
    matchScope: key.agent_runtime_id === runtime.agent_runtime_id ? "runtime" : "project",
  };
}

function runtimeKeySortScore(runtime: AdminOpsRuntime, key: AdminOpsFinitePrivateApiKey) {
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
  runtime: AdminOpsRuntime,
  summary: RuntimeFinitePrivateGrantSummary | null | undefined,
  query: string,
) {
  const tokens = query
    .trim()
    .toLowerCase()
    .split(/\s+/u)
    .filter(Boolean);
  if (tokens.length === 0) {
    return true;
  }
  const haystack = [
    runtime.project_display_name,
    runtime.owner_email,
    runtime.project_id,
    runtime.agent_runtime_id,
    runtime.source_host_id,
    runtime.source_machine_id,
    runtime.runtime_artifact_id,
    runtime.runtime_artifact_version_label,
    runtime.runtime_status,
    summary?.grantId,
    summary?.grantStatus,
    summary?.grantUserId,
    summary?.limitProfileId,
    summary?.keyId,
    summary?.keyStatus,
    summary?.keyProjectId,
    summary?.keyAgentRuntimeId,
    summary?.matchScope,
  ]
    .filter((value): value is string => Boolean(value))
    .join(" ")
    .toLowerCase();
  return tokens.every((token) => haystack.includes(token));
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
