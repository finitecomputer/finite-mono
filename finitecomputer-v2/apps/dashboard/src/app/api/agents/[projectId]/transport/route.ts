import { requestOriginMatchesHost } from "@/lib/http-headers";
import { NextResponse } from "next/server";
import { agentTransportControl, CoreFetchError } from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

type Context = { params: Promise<{ projectId: string }> };
async function control(request: Request, context: Context) {
  const headers = { "cache-control": "no-store" };
  // This first UI slice is internal only. Core independently verifies the JWT,
  // project ownership, operator role, endpoint generation and hosted-access flag.
  if (!(await loadOptionalViewerContext()).isAdmin) {
    return NextResponse.json({ error: "Admin access required" }, { status: 403, headers });
  }
  if (request.method !== "GET" && !requestOriginMatchesHost(request)) {
    return NextResponse.json({ error: "Same-origin request required" }, { status: 403, headers });
  }
  const { projectId } = await context.params;
  try {
    const payload = request.method === "GET" ? undefined : await request.json();
    const result = await agentTransportControl(
      projectId, request.method as "GET" | "POST" | "DELETE" | "PUT", payload,
    );
    return NextResponse.json(result ?? null, { headers });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof CoreFetchError ? error.message : "Agent access is unavailable" },
      { status: error instanceof CoreFetchError ? error.status : 503, headers },
    );
  }
}
export const GET = control;
export const POST = control;
export const PUT = control;
export const DELETE = control;
