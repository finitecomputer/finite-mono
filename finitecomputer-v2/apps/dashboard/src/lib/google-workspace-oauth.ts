import { sealData, unsealData } from "iron-session";

export const GOOGLE_WORKSPACE_SCOPES = [
  "https://www.googleapis.com/auth/drive",
  "https://www.googleapis.com/auth/spreadsheets",
  "https://www.googleapis.com/auth/gmail.readonly",
  "https://www.googleapis.com/auth/gmail.send",
  "https://www.googleapis.com/auth/gmail.modify",
  "https://www.googleapis.com/auth/calendar",
  "https://www.googleapis.com/auth/contacts.readonly",
  "https://www.googleapis.com/auth/documents",
  "openid",
  "https://www.googleapis.com/auth/userinfo.email",
  "https://www.googleapis.com/auth/userinfo.profile",
] as const;

/** The callback binds the connected email through Google's userinfo endpoint. */
const REQUIRED_IDENTITY_SCOPES = ["openid", "https://www.googleapis.com/auth/userinfo.email"];
const IDENTITY_SCOPES: ReadonlySet<string> = new Set([
  ...REQUIRED_IDENTITY_SCOPES,
  "https://www.googleapis.com/auth/userinfo.profile",
]);

const STATE_TTL_SECONDS = 15 * 60;

export type GoogleWorkspaceOAuthConfig = {
  clientId: string;
  clientSecret: string;
  redirectUri: string;
};

type GoogleWorkspaceOAuthState = {
  machineId: string;
  workosUserId: string;
  issuedAtMs: number;
};

/**
 * People may untick individual permissions on Google's consent screen. Accept
 * any grant within the contract that keeps the account identity and some
 * Workspace access, in contract order; agentd applies the same rule.
 */
export function acceptedGoogleWorkspaceScopes(scopeValue: string): string[] | null {
  const granted = new Set(scopeValue.split(/\s+/u).filter(Boolean));
  const contract: ReadonlySet<string> = new Set(GOOGLE_WORKSPACE_SCOPES);
  const grantedScopes = [...granted];
  if (
    grantedScopes.some((scope) => !contract.has(scope)) ||
    REQUIRED_IDENTITY_SCOPES.some((scope) => !granted.has(scope)) ||
    grantedScopes.every((scope) => IDENTITY_SCOPES.has(scope))
  ) {
    return null;
  }
  return GOOGLE_WORKSPACE_SCOPES.filter((scope) => granted.has(scope));
}

export function googleWorkspaceOAuthConfigured(
  env: Record<string, string | undefined> = process.env
) {
  return Boolean(
    env.GOOGLE_WORKSPACE_CLIENT_ID?.trim() &&
      env.GOOGLE_WORKSPACE_CLIENT_SECRET?.trim() &&
      oauthStatePassword(env)
  );
}

export function googleWorkspaceOAuthConfig(
  requestUrl: string,
  env: Record<string, string | undefined> = process.env
): GoogleWorkspaceOAuthConfig | null {
  const clientId = env.GOOGLE_WORKSPACE_CLIENT_ID?.trim();
  const clientSecret = env.GOOGLE_WORKSPACE_CLIENT_SECRET?.trim();
  if (!clientId || !clientSecret || !oauthStatePassword(env)) {
    return null;
  }
  const baseUrl = dashboardBaseUrl(requestUrl, env);
  return {
    clientId,
    clientSecret,
    redirectUri: new URL("/google-workspace/callback", baseUrl).toString(),
  };
}

/**
 * Resolve browser-facing redirects from configured public origin rather than
 * Next's loopback request URL behind Caddy/host networking.
 */
export function googleWorkspaceDashboardUrl(
  path: string,
  requestUrl: string,
  env: Record<string, string | undefined> = process.env
) {
  return new URL(path, dashboardBaseUrl(requestUrl, env));
}

export async function sealGoogleWorkspaceState(
  state: GoogleWorkspaceOAuthState,
  env: Record<string, string | undefined> = process.env
) {
  const password = oauthStatePassword(env);
  if (!password) {
    throw new Error("Google Workspace connection is unavailable.");
  }
  return sealData(state, { password, ttl: STATE_TTL_SECONDS });
}

export async function unsealGoogleWorkspaceState(
  sealed: string,
  env: Record<string, string | undefined> = process.env
) {
  const password = oauthStatePassword(env);
  if (!password) {
    return null;
  }
  try {
    const state = await unsealData<GoogleWorkspaceOAuthState>(sealed, { password });
    if (
      !state.machineId?.trim() ||
      !state.workosUserId?.trim() ||
      !Number.isFinite(state.issuedAtMs) ||
      Date.now() - state.issuedAtMs > STATE_TTL_SECONDS * 1000 ||
      state.issuedAtMs > Date.now() + 60_000
    ) {
      return null;
    }
    return state;
  } catch {
    return null;
  }
}

function oauthStatePassword(env: Record<string, string | undefined>) {
  const password = env.WORKOS_COOKIE_PASSWORD?.trim();
  return password && password.length >= 32 ? password : null;
}

function dashboardBaseUrl(requestUrl: string, env: Record<string, string | undefined>) {
  for (const candidate of [
    env.FC_DASHBOARD_PUBLIC_URL,
    env.NEXT_PUBLIC_APP_URL,
    env.FC_DASHBOARD_BASE_URL,
    env.NEXT_PUBLIC_WORKOS_REDIRECT_URI,
    requestUrl,
  ]) {
    if (!candidate?.trim()) continue;
    try {
      const parsed = new URL(candidate);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        return parsed.origin;
      }
    } catch {
      // Try the next configured public URL.
    }
  }
  throw new Error("Dashboard URL is unavailable.");
}
