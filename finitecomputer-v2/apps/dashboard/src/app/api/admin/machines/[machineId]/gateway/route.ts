import { NextResponse } from "next/server";
import { controlHostedGateway, HostedAgentControlError } from "@/lib/hosted-agent-controls";
import { requestOriginMatchesHost, requestOriginSameOrNone } from "@/lib/http-headers";

type Context = { params: Promise<{ machineId: string }> };
const headers = { "cache-control": "no-store", "referrer-policy": "no-referrer" };

export async function GET(request: Request, context: Context) {
  if (!requestOriginSameOrNone(request)) return NextResponse.json({ error: "Invalid request origin." }, { status: 403, headers });
  return respond(() => context.params.then(({ machineId }) => controlHostedGateway(machineId, "status")));
}

export async function POST(request: Request, context: Context) {
  if (!requestOriginMatchesHost(request)) return NextResponse.json({ error: "Invalid request origin." }, { status: 403, headers });
  return respond(async () => {
    const payload = await request.json().catch(() => null);
    if (!payload || !["enable", "disable"].includes(payload.action) || Object.keys(payload).length !== 1) {
      throw new HostedAgentControlError("Choose enable or disable.", 400);
    }
    const { machineId } = await context.params;
    return controlHostedGateway(machineId, payload.action);
  });
}

async function respond(action: () => Promise<unknown>) {
  try { return NextResponse.json(await action(), { headers }); }
  catch (error) {
    // Never log command responses: they can contain the gateway credential.
    return NextResponse.json({ error: error instanceof HostedAgentControlError ? error.message : "Gateway is unavailable. Try again." },
      { status: error instanceof HostedAgentControlError ? error.status : 502, headers });
  }
}
