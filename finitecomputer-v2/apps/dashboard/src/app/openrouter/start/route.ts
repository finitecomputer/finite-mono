import { NextResponse } from "next/server";

import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  generateOpenRouterCodeVerifier,
  openRouterAuthorizationUrl,
  openRouterCallbackUrl,
  openRouterCodeChallenge,
  openRouterDashboardUrl,
  OPENROUTER_OAUTH_COOKIE,
  openRouterOAuthConfigured,
  OPENROUTER_STATE_TTL_SECONDS,
  sealOpenRouterState,
} from "@/lib/openrouter-oauth";

export async function GET(request: Request) {
  const requestUrl = new URL(request.url);
  const machineId = requestUrl.searchParams.get("machineId")?.trim();
  if (!machineId) {
    return NextResponse.redirect(openRouterDashboardUrl("/dashboard", request.url));
  }
  const redirectPath = `/dashboard/machines/${encodeURIComponent(machineId)}/connections`;
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    const login = openRouterDashboardUrl("/login", request.url);
    login.searchParams.set("returnTo", `${requestUrl.pathname}${requestUrl.search}`);
    return NextResponse.redirect(login);
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access || !openRouterOAuthConfigured()) {
    return NextResponse.redirect(
      openRouterDashboardUrl(`${redirectPath}?openrouter=unavailable`, request.url)
    );
  }

  const codeVerifier = await generateOpenRouterCodeVerifier();
  const codeChallenge = await openRouterCodeChallenge(codeVerifier);
  const callbackUrl = openRouterCallbackUrl(request.url);
  const authorization = openRouterAuthorizationUrl(callbackUrl, codeChallenge);
  const sealed = await sealOpenRouterState({
    machineId: access.machineId,
    workosUserId: account.workosUserId,
    codeVerifier,
    issuedAtMs: Date.now(),
  });
  const response = NextResponse.redirect(authorization);
  response.cookies.set({
    name: OPENROUTER_OAUTH_COOKIE,
    value: sealed,
    httpOnly: true,
    secure: callbackUrl.startsWith("https://"),
    sameSite: "lax",
    path: "/openrouter/callback",
    maxAge: OPENROUTER_STATE_TTL_SECONDS,
  });
  return response;
}
