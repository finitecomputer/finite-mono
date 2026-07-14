import { NextResponse } from "next/server";

import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  coreProjectSupportsRetirement,
  requestCoreRuntimeDestroy,
} from "@/lib/core-client";
import {
  requestHasExactOrigin,
  requestOriginMatchesHost,
} from "@/lib/http-headers";
import { workosBaseUrl } from "@/lib/workos-auth";

type RouteContext = {
  params: Promise<{ machineId: string }>;
};

export async function POST(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  const configuredBaseUrl = workosBaseUrl();
  if (
    !requestOriginMatchesHost(request) &&
    !requestHasExactOrigin(request) &&
    (!configuredBaseUrl || !requestHasExactOrigin(request, configuredBaseUrl))
  ) {
    return machineRedirect(request, machineId, "unavailable");
  }
  const access = await loadDashboardMachineAccess(machineId);

  if (
    !access?.coreProject ||
    // Operator-only maintenance for now; mirrors the admin-gated UI section.
    !access.viewer.isAdmin ||
    !coreProjectSupportsRetirement(access.coreProject) ||
    !access.canRetireRuntime
  ) {
    return machineRedirect(request, access?.machineId ?? machineId, "unavailable");
  }

  try {
    await requestCoreRuntimeDestroy(access.coreProject.project.id);
  } catch {
    return machineRedirect(request, access.machineId, "failed");
  }

  const destination = new URL("/dashboard", request.url);
  destination.searchParams.set("new", "1");
  destination.searchParams.set("agentRemoval", "requested");
  return NextResponse.redirect(destination, { status: 303 });
}

export async function GET(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return machineRedirect(request, machineId, "unavailable");
}

function machineRedirect(request: Request, machineId: string, result: string) {
  const destination = new URL(
    `/dashboard/machines/${encodeURIComponent(machineId)}`,
    request.url
  );
  destination.searchParams.set("removal", result);
  return NextResponse.redirect(destination, { status: 303 });
}
