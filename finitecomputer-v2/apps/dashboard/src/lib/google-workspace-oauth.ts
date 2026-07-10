import { sealData, unsealData } from "iron-session";

export const GOOGLE_WORKSPACE_SCOPES = [
  "https://www.googleapis.com/auth/drive",
  "https://www.googleapis.com/auth/spreadsheets",
  "https://www.googleapis.com/auth/gmail.readonly",
  "https://www.googleapis.com/auth/gmail.send",
  "https://www.googleapis.com/auth/gmail.modify",
  "https://www.googleapis.com/auth/calendar",
  "https://www.googleapis.com/auth/contacts.readonly",
  "https://www.googleapis.com/auth/documents.readonly",
  "https://www.googleapis.com/auth/script.projects",
  "https://www.googleapis.com/auth/script.deployments",
  "openid",
  "https://www.googleapis.com/auth/userinfo.email",
  "https://www.googleapis.com/auth/userinfo.profile",
] as const;

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
