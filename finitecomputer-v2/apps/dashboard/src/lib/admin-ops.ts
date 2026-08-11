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
