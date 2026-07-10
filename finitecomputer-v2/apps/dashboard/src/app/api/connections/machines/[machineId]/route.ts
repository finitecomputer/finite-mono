import { NextResponse } from "next/server";

import {
  dispatchAgentConnectionAction,
  HostedAgentControlError,
  loadAgentConnections,
} from "@/lib/hosted-agent-controls";

type RouteContext = { params: Promise<{ machineId: string }> };

export async function GET(_request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return connectionResponse(() => loadAgentConnections(machineId));
}

export async function POST(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return connectionResponse(async () => {
    const payload = await request.json().catch(() => null);
    return dispatchAgentConnectionAction(machineId, payload);
  });
}

async function connectionResponse(action: () => Promise<unknown>) {
  try {
    return NextResponse.json(await action(), {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    const status = error instanceof HostedAgentControlError ? error.status : 500;
    const message =
      error instanceof HostedAgentControlError
        ? error.message
        : "Connections are unavailable right now. Try again.";
    if (!(error instanceof HostedAgentControlError)) {
      console.warn("Connections request failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return NextResponse.json(
      { error: message },
      { status, headers: { "cache-control": "no-store" } }
    );
  }
}
