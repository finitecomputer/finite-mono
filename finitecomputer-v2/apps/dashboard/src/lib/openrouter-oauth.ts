import { sealData, unsealData } from "iron-session";

import { dashboardBaseUrl } from "@/lib/dashboard-base-url";

export const OPENROUTER_OAUTH_COOKIE = "openrouter_oauth";
export const OPENROUTER_STATE_TTL_SECONDS = 15 * 60;

const OPENROUTER_AUTH_URL = "https://openrouter.ai/auth";
const OPENROUTER_KEY_EXCHANGE_URL = "https://openrouter.ai/api/v1/auth/keys";
const CODE_VERIFIER_PATTERN = /^[A-Za-z0-9\-._~]{43,128}$/u;

type OpenRouterOAuthState = {
  machineId: string;
  workosUserId: string;
  codeVerifier: string;
  issuedAtMs: number;
};

export function openRouterOAuthConfigured(
  env: Record<string, string | undefined> = process.env
) {
  return Boolean(oauthStatePassword(env));
}

export function openRouterCallbackUrl(
  requestUrl: string,
  env: Record<string, string | undefined> = process.env
) {
  return new URL("/openrouter/callback", dashboardBaseUrl(requestUrl, env)).toString();
}

export function openRouterDashboardUrl(
  path: string,
  requestUrl: string,
  env: Record<string, string | undefined> = process.env
) {
  return new URL(path, dashboardBaseUrl(requestUrl, env));
}

export function openRouterAuthorizationUrl(callbackUrl: string, codeChallenge: string) {
  const authorization = new URL(OPENROUTER_AUTH_URL);
  authorization.searchParams.set("callback_url", callbackUrl);
  authorization.searchParams.set("code_challenge", codeChallenge);
  authorization.searchParams.set("code_challenge_method", "S256");
  return authorization;
}

export async function generateOpenRouterCodeVerifier() {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return base64Url(bytes);
}

export async function openRouterCodeChallenge(verifier: string) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return base64Url(new Uint8Array(digest));
}

export async function sealOpenRouterState(
  state: OpenRouterOAuthState,
  env: Record<string, string | undefined> = process.env
) {
  const password = oauthStatePassword(env);
  if (!password) {
    throw new Error("OpenRouter sign-in is unavailable.");
  }
  return sealData(state, { password, ttl: OPENROUTER_STATE_TTL_SECONDS });
}

export async function unsealOpenRouterState(
  sealed: string,
  env: Record<string, string | undefined> = process.env
): Promise<OpenRouterOAuthState | null> {
  const password = oauthStatePassword(env);
  if (!password) {
    return null;
  }
  try {
    const state = await unsealData<OpenRouterOAuthState>(sealed, { password });
    if (
      !state.machineId?.trim() ||
      !state.workosUserId?.trim() ||
      !CODE_VERIFIER_PATTERN.test(state.codeVerifier ?? "") ||
      !Number.isFinite(state.issuedAtMs) ||
      Date.now() - state.issuedAtMs > OPENROUTER_STATE_TTL_SECONDS * 1000 ||
      state.issuedAtMs > Date.now() + 60_000
    ) {
      return null;
    }
    return state;
  } catch {
    return null;
  }
}

export async function exchangeOpenRouterCode(
  code: string,
  codeVerifier: string,
  fetchImpl: typeof fetch = fetch
): Promise<string | null> {
  const response = await fetchImpl(OPENROUTER_KEY_EXCHANGE_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      code,
      code_verifier: codeVerifier,
      code_challenge_method: "S256",
    }),
    cache: "no-store",
    signal: AbortSignal.timeout(15_000),
  });
  const payload = (await response.json().catch(() => null)) as { key?: unknown } | null;
  const key = typeof payload?.key === "string" ? payload.key.trim() : "";
  if (!response.ok || key.length < 20 || key.length > 16 * 1024) {
    return null;
  }
  return key;
}

function oauthStatePassword(env: Record<string, string | undefined>) {
  const password = env.WORKOS_COOKIE_PASSWORD?.trim();
  return password && password.length >= 32 ? password : null;
}

function base64Url(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}
