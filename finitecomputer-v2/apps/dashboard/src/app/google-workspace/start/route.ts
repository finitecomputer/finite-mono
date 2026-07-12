import { NextResponse } from "next/server";

import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  googleWorkspaceOAuthConfig,
  googleWorkspaceDashboardUrl,
  GOOGLE_WORKSPACE_SCOPES,
  sealGoogleWorkspaceState,
} from "@/lib/google-workspace-oauth";

export async function GET(request: Request) {
  const requestUrl = new URL(request.url);
  const machineId = requestUrl.searchParams.get("machineId")?.trim();
  if (!machineId) {
    return NextResponse.redirect(googleWorkspaceDashboardUrl("/dashboard", request.url));
  }
  const redirectPath = `/dashboard/machines/${encodeURIComponent(machineId)}/connections`;
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return NextResponse.redirect(googleWorkspaceDashboardUrl("/login", request.url));
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  const config = googleWorkspaceOAuthConfig(request.url);
  if (!access || !config) {
    return NextResponse.redirect(
      googleWorkspaceDashboardUrl(`${redirectPath}?google=unavailable`, request.url)
    );
  }

  const state = await sealGoogleWorkspaceState({
    machineId: access.machineId,
    workosUserId: account.workosUserId,
    issuedAtMs: Date.now(),
  });
  const authorization = new URL("https://accounts.google.com/o/oauth2/v2/auth");
  authorization.searchParams.set("client_id", config.clientId);
  authorization.searchParams.set("redirect_uri", config.redirectUri);
  authorization.searchParams.set("response_type", "code");
  authorization.searchParams.set("access_type", "offline");
  authorization.searchParams.set("prompt", "select_account consent");
  authorization.searchParams.set("include_granted_scopes", "false");
  authorization.searchParams.set("scope", GOOGLE_WORKSPACE_SCOPES.join(" "));
  authorization.searchParams.set("state", state);
  return NextResponse.redirect(authorization);
}
