import type { AccountAuthContext } from "./dashboard-auth";

// The account boundary proves email; the gate signs it; Sites owns permissions.
// No account/agent provisioning or share lookup belongs in this adapter.
export class SiteGateError extends Error {
  constructor(message: string, readonly status: number) { super(message); }
}

export function gateSiteOrigin(value: string, env = process.env): string {
  const base = env.FC_SITES_GATE_BASE_DOMAIN?.trim() || "finite.site";
  let url: URL;
  try { url = new URL(value); } catch { throw new SiteGateError("Choose a valid site.", 400); }
  const label = url.hostname.endsWith(`.${base}`) ? url.hostname.slice(0, -base.length - 1) : "";
  const local = env.NODE_ENV !== "production" && base.endsWith(".localhost") && url.protocol === "http:";
  if (!/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label)
      || ["api", "git", "auth", "www"].includes(label)
      || (!local && (url.protocol !== "https:" || url.port))
      || url.username || url.password || url.pathname !== "/" || url.search || url.hash) {
    throw new SiteGateError("Choose a valid site.", 400);
  }
  return url.origin;
}

export function gateReturnPath(value: string): string {
  if (!value.startsWith("/") || value.startsWith("//") || value.includes("\\")
      || value.length > 1024 || /[^\x21-\x7e]/u.test(value)
      || new URL(value, "https://site.invalid").pathname.startsWith("/_finite/")) {
    throw new SiteGateError("Choose a valid site path.", 400);
  }
  return value;
}

export async function mintSiteGateSession(
  account: AccountAuthContext,
  output: string,
  returnTo: string,
  env = process.env,
): Promise<string> {
  if (!account.workosUserId || !account.emailVerified || !account.email
      || (account.source !== "workos" && !(env.NODE_ENV !== "production" && account.source === "dev"))) {
    throw new SiteGateError("Sign in with a verified email to view this site.", 401);
  }
  const origin = gateSiteOrigin(output, env);
  const path = gateReturnPath(returnTo);
  const token = env.FINITE_GATE_ACCOUNT_TOKEN?.trim();
  const gate = env.FC_SITES_AUTH_GATE_URL?.trim();
  if (!gate || !token || !/^[0-9a-f]{64}$/.test(token)) {
    throw new SiteGateError("Site sign-in is temporarily unavailable.", 503);
  }
  const endpoint = new URL(gate);
  if (endpoint.username || endpoint.password || endpoint.pathname !== "/" || endpoint.search || endpoint.hash
      || (endpoint.protocol !== "https:" && !(env.NODE_ENV !== "production" && endpoint.protocol === "http:" && (endpoint.hostname === "localhost" || endpoint.hostname.endsWith(".localhost"))))) {
    throw new SiteGateError("Site sign-in is temporarily unavailable.", 503);
  }
  try {
    const response = await fetch(new URL("/vouch", endpoint), {
      method: "POST", redirect: "error", cache: "no-store",
      signal: AbortSignal.timeout(5000),
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({ output: origin, return_to: path, email: account.email }),
    });
    if (!response.ok) throw new Error("gate unavailable");
    const body = await response.text();
    if (body.length > 8192) throw new Error("gate response too large");
    const payload = JSON.parse(body) as { redeem_url?: unknown };
    if (typeof payload.redeem_url !== "string") throw new Error("invalid gate response");
    const target = new URL(payload.redeem_url);
    const keys = Array.from(target.searchParams.keys()).sort().join(",");
    if (target.origin !== origin || target.pathname !== "/_finite/auth" || target.username || target.password || target.hash
        || keys !== "gate_code,return_to" || !target.searchParams.get("gate_code")
        || target.searchParams.get("return_to") !== path) throw new Error("invalid gate target");
    return target.toString();
  } catch {
    throw new SiteGateError("Site sign-in is temporarily unavailable.", 502);
  }
}
