import { NextResponse } from "next/server";

import { HostedAgentControlError } from "@/lib/hosted-agent-controls";
import { applyChatAdmission } from "@/lib/hosted-agent-admission";

type RouteContext = { params: Promise<{ machineId: string }> };

// No GET on purpose: the sidecar exposes no list/read surface for the
// Welcome allowlist, so the dashboard cannot render current entries.
export async function POST(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  try {
    const payload = await request.json().catch(() => null);
    const result = await applyChatAdmission(machineId, payload);
    return NextResponse.json(result, {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    const status = error instanceof HostedAgentControlError ? error.status : 500;
    const message =
      error instanceof HostedAgentControlError
        ? error.message
        : "Chat admission is unavailable right now. Try again.";
    if (!(error instanceof HostedAgentControlError)) {
      console.warn("Chat admission request failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return NextResponse.json(
      { error: message },
      { status, headers: { "cache-control": "no-store" } }
    );
  }
}
