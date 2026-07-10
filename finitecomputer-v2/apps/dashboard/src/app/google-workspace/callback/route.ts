import { NextResponse } from "next/server";

import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  applyGoogleConnection,
  HostedAgentControlError,
} from "@/lib/hosted-agent-controls";
import {
  googleWorkspaceOAuthConfig,
  GOOGLE_WORKSPACE_SCOPES,
  unsealGoogleWorkspaceState,
} from "@/lib/google-workspace-oauth";

export async function GET(request: Request) {
  const requestUrl = new URL(request.url);
  const state = await unsealGoogleWorkspaceState(
    requestUrl.searchParams.get("state")?.trim() ?? ""
  );
  if (!state) {
    return NextResponse.redirect(new URL("/dashboard", requestUrl));
  }
  const redirectPath = `/dashboard/machines/${encodeURIComponent(state.machineId)}/connections`;
  const redirect = (result: string) =>
    NextResponse.redirect(new URL(`${redirectPath}?google=${result}`, requestUrl));
  if (requestUrl.searchParams.get("error")) {
    return redirect("cancelled");
  }
  const code = requestUrl.searchParams.get("code")?.trim();
  const account = await getAccountAuthContext();
  const access = await loadDashboardMachineAccess(state.machineId, { coreCacheMode: "swr" });
  const config = googleWorkspaceOAuthConfig(request.url);
  if (
    !code ||
    !account.workosUserId ||
    account.workosUserId !== state.workosUserId ||
    !account.emailVerified ||
    !access ||
    !config
  ) {
    return redirect("failed");
  }

  try {
    const tokenResponse = await fetch("https://oauth2.googleapis.com/token", {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        client_id: config.clientId,
        client_secret: config.clientSecret,
        code,
        grant_type: "authorization_code",
        redirect_uri: config.redirectUri,
      }),
      cache: "no-store",
      signal: AbortSignal.timeout(15_000),
    });
    const tokens = (await tokenResponse.json()) as {
      access_token?: unknown;
      refresh_token?: unknown;
      scope?: unknown;
    };
    const accessToken = requiredString(tokens.access_token);
    const refreshToken = requiredString(tokens.refresh_token);
    const scopeValue = requiredString(tokens.scope);
    if (!tokenResponse.ok || !accessToken || !refreshToken || !scopeValue) {
      return redirect("failed");
    }
    const scopes = scopeValue.split(/\s+/u).filter(Boolean);
    if (!sameScopes(scopes)) return redirect("failed");

    const userInfoResponse = await fetch("https://openidconnect.googleapis.com/v1/userinfo", {
      headers: { authorization: `Bearer ${accessToken}` },
      cache: "no-store",
      signal: AbortSignal.timeout(10_000),
    });
    const userInfo = (await userInfoResponse.json()) as { email?: unknown };
    const connectedEmail = requiredString(userInfo.email);
    if (!userInfoResponse.ok || !connectedEmail || !connectedEmail.includes("@")) {
      return redirect("failed");
    }

    await applyGoogleConnection(access.machineId, {
      clientId: config.clientId,
      clientSecret: config.clientSecret,
      refreshToken,
      accessToken,
      redirectUri: config.redirectUri,
      connectedEmail,
      scopes,
    });
    return redirect("connected");
  } catch (error) {
    if (!(error instanceof HostedAgentControlError)) {
      console.warn("Google Workspace connection failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return redirect("failed");
  }
}

function requiredString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function sameScopes(scopes: string[]) {
  const granted = new Set(scopes);
  return (
    granted.size === GOOGLE_WORKSPACE_SCOPES.length &&
    GOOGLE_WORKSPACE_SCOPES.every((scope) => granted.has(scope))
  );
}
