// Pure helpers for the Admin Ops page (/dashboard/admin).
//
// The dashboard admin gate here is UI-only. Core independently enforces every
// admin mutation against its FC_CORE_ADMIN_EMAILS allowlist.

export type AdminOpsViewer = {
  isAdmin: boolean;
};

export function canAccessAdminOps(viewer: AdminOpsViewer | null | undefined): boolean {
  return Boolean(viewer?.isAdmin);
}

/**
 * Parse a comma-separated admin email allowlist with the same semantics as
 * Core's FC_CORE_ADMIN_EMAILS: entries trimmed and lowercased, blanks dropped.
 */
export function parseAdminEmailAllowlist(raw: string | null | undefined): Set<string> {
  const allowlist = new Set<string>();
  for (const entry of (raw ?? "").split(",")) {
    const email = entry.trim().toLowerCase();
    if (email) {
      allowlist.add(email);
    }
  }
  return allowlist;
}

/**
 * True when the (already normalized) email is in FC_CORE_ADMIN_EMAILS. The
 * dashboard shares Core's allowlist so the UI gate and Core enforcement agree.
 */
export function isCoreAdminEmail(
  email: string | null,
  env: Record<string, string | undefined> = process.env,
): boolean {
  if (!email) {
    return false;
  }
  return parseAdminEmailAllowlist(env.FC_CORE_ADMIN_EMAILS).has(email);
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
};

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
  return { name, codeCount, expiresInHours };
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
