import { cookies } from "next/headers";
import { NextResponse } from "next/server";

import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  dispatchAgentConnectionAction,
  HostedAgentControlError,
  loadAgentConnections,
} from "@/lib/hosted-agent-controls";
import { OPENROUTER_DEFAULT_MODEL } from "@/lib/openrouter";
import {
  exchangeOpenRouterCode,
  OPENROUTER_OAUTH_COOKIE,
  openRouterDashboardUrl,
  unsealOpenRouterState,
} from "@/lib/openrouter-oauth";

export async function GET(request: Request) {
  const requestUrl = new URL(request.url);
  const sealed = (await cookies()).get(OPENROUTER_OAUTH_COOKIE)?.value ?? "";
  const state = await unsealOpenRouterState(sealed);
  if (!state) {
    return redirectTo("/dashboard", request.url);
  }
  const redirectPath = `/dashboard/machines/${encodeURIComponent(state.machineId)}/connections`;
  const finish = (result: string) => redirectTo(`${redirectPath}?openrouter=${result}`, request.url);

  const code = requestUrl.searchParams.get("code")?.trim();
  if (!code) {
    return finish("cancelled");
  }
  const account = await getAccountAuthContext();
  const access = await loadDashboardMachineAccess(state.machineId, { coreCacheMode: "swr" });
  if (
    !account.workosUserId ||
    account.workosUserId !== state.workosUserId ||
    !account.emailVerified ||
    !access
  ) {
    return finish("failed");
  }

  try {
    const apiKey = await exchangeOpenRouterCode(code, state.codeVerifier);
    if (!apiKey) {
      return finish("failed");
    }
    // Reconnecting rotates the key without disturbing a chosen model; new
    // connections start on the panel default.
    const current = await loadAgentConnections(access.machineId).catch(() => null);
    const model =
      current?.inference.profile === "openrouter" && current.inference.model
        ? current.inference.model
        : OPENROUTER_DEFAULT_MODEL;
    await dispatchAgentConnectionAction(access.machineId, {
      action: "inference",
      profile: "openrouter",
      apiKey,
      model,
    });
    return finish("connected");
  } catch (error) {
    if (!(error instanceof HostedAgentControlError)) {
      console.warn("OpenRouter connection failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return finish("failed");
  }
}

function redirectTo(path: string, requestUrl: string) {
  const response = NextResponse.redirect(openRouterDashboardUrl(path, requestUrl));
  response.cookies.set({
    name: OPENROUTER_OAUTH_COOKIE,
    value: "",
    path: "/openrouter/callback",
    maxAge: 0,
  });
  return response;
}
