export type WorkosAuthStatus = {
  enabled: boolean;
  ready: boolean;
  missing: string[];
};

const REQUIRED_WORKOS_ENV = [
  "WORKOS_API_KEY",
  "WORKOS_CLIENT_ID",
  "WORKOS_COOKIE_PASSWORD",
  "NEXT_PUBLIC_WORKOS_REDIRECT_URI",
] as const;

const TRUTHY_ENV_VALUES = new Set(["1", "true", "yes", "on"]);

const PROTECTED_WORKOS_PATH_PREFIXES = [
  "/api",
  "/client",
  "/dashboard",
  "/dev",
] as const;

const PUBLIC_WORKOS_PATHS = new Set([
  "/",
  "/callback",
  "/favicon.ico",
  "/favicon.svg",
  "/login",
  "/logout",
  "/manifest.webmanifest",
  "/api/stripe/webhook",
  "/signup",
]);

const PUBLIC_WORKOS_PATH_PREFIXES = ["/api/finite"] as const;

const WORKOS_PROXY_BYPASS_PATHS = new Set([
  "/callback",
  "/health",
  "/api/brain/identity-provider",
  "/api/stripe/webhook",
  "/login",
  "/logout",
  "/signup",
]);

// Brain's product client is a WorkOS-authenticated browser surface, while
// the opaque-frame identity provider uses its frame capability plus a fresh,
// request-bound WorkOS session proof. /_admin uses Brain-owned route-level auth
// (normally a Nostr signature, with narrowly scoped invitation proofs where
// specified). Let those APIs reach Brain without replacing their authority
// with an AuthKit session. Runtime callbacks likewise enforce their own
// protocol boundary.
const WORKOS_PROXY_BYPASS_PATH_PREFIXES = ["/_admin", "/api/finite"] as const;

type EnvSource = Record<string, string | undefined>;

export function workosAuthEnabled(env: EnvSource = process.env) {
  return TRUTHY_ENV_VALUES.has((env.FC_WORKOS_AUTH_ENABLED ?? "").trim().toLowerCase());
}

export function workosAuthStatus(env: EnvSource = process.env): WorkosAuthStatus {
  const enabled = workosAuthEnabled(env);
  const missing = enabled ? missingWorkosAuthEnv(env) : [];

  return {
    enabled,
    ready: enabled && missing.length === 0,
    missing,
  };
}

export function missingWorkosAuthEnv(env: EnvSource = process.env) {
  const missing: string[] = REQUIRED_WORKOS_ENV.filter((name) => !env[name]?.trim());
  const cookiePassword = env.WORKOS_COOKIE_PASSWORD?.trim();
  const redirectUri = env.NEXT_PUBLIC_WORKOS_REDIRECT_URI?.trim();

  if (cookiePassword && cookiePassword.length < 32) {
    missing.push("WORKOS_COOKIE_PASSWORD>=32");
  }

  if (redirectUri && !workosBaseUrl(env)) {
    missing.push("NEXT_PUBLIC_WORKOS_REDIRECT_URI(valid URL)");
  }

  return missing;
}

export function workosProtectedPath(pathname: string) {
  if (PUBLIC_WORKOS_PATHS.has(pathname)) {
    return false;
  }

  if (
    PUBLIC_WORKOS_PATH_PREFIXES.some((prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`))
  ) {
    return false;
  }

  return PROTECTED_WORKOS_PATH_PREFIXES.some((prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`));
}

export function workosProxyBypassPath(pathname: string) {
  if (WORKOS_PROXY_BYPASS_PATHS.has(pathname)) {
    return true;
  }

  return WORKOS_PROXY_BYPASS_PATH_PREFIXES.some(
    (prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`)
  );
}

export function workosSessionCookieName(env: EnvSource = process.env) {
  return env.WORKOS_COOKIE_NAME?.trim() || "wos-session";
}

export function workosInteractiveAuthRequest(
  method: string,
  headers: Pick<Headers, "get" | "has">
) {
  const upperMethod = method.toUpperCase();
  if (!["GET", "HEAD", "POST"].includes(upperMethod)) {
    return false;
  }

  const accept = headers.get("accept") ?? "";
  const secFetchMode = headers.get("sec-fetch-mode");
  const isRsc =
    headers.has("rsc") ||
    headers.has("RSC") ||
    headers.has("next-router-state-tree") ||
    headers.has("Next-Router-State-Tree");
  const isPrefetch =
    headers.get("purpose") === "prefetch" ||
    headers.get("sec-purpose") === "prefetch" ||
    headers.has("next-router-prefetch") ||
    headers.has("Next-Router-Prefetch");

  if (isRsc || isPrefetch) {
    return false;
  }
  if (secFetchMode) {
    return secFetchMode === "navigate";
  }

  return accept.includes("text/html");
}

export function safeWorkosReturnPathname(value: string | null | undefined, fallback = "/dashboard") {
  if (!value) {
    return fallback;
  }

  if (!value.startsWith("/") || value.startsWith("//")) {
    return fallback;
  }

  try {
    const parsed = new URL(value, "https://finite.computer");
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return fallback;
  }
}

export function workosBaseUrl(env: EnvSource = process.env) {
  const redirectUri = env.NEXT_PUBLIC_WORKOS_REDIRECT_URI?.trim();

  if (!redirectUri) {
    return undefined;
  }

  try {
    const parsed = new URL(redirectUri);
    return parsed.origin;
  } catch {
    return undefined;
  }
}

export function workosLogoutReturnTo(env: EnvSource = process.env) {
  const baseUrl = workosBaseUrl(env);
  if (!baseUrl) {
    return "/";
  }

  return new URL("/", baseUrl).toString();
}
