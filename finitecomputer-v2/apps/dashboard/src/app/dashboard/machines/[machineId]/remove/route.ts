import { NextResponse } from "next/server";

import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  coreProjectSupportsHostedRestart,
  requestCoreRuntimeDestroy,
} from "@/lib/core-client";

type RouteContext = {
  params: Promise<{ machineId: string }>;
};

export async function POST(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  const access = await loadDashboardMachineAccess(machineId);

  if (
    !access?.coreProject ||
    !coreProjectSupportsHostedRestart(access.coreProject) ||
    !access.canRemoveKataRuntime
  ) {
    return machineRedirect(request, machineId, "unavailable");
  }

  try {
    await requestCoreRuntimeDestroy(access.coreProject.project.id);
  } catch {
    return machineRedirect(request, machineId, "failed");
  }

  return machineRedirect(request, machineId, "requested");
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
