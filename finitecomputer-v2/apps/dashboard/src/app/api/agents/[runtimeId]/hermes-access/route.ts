import {
  CoreFetchError, createCoreHostedHermesSession, loadCoreHostedHermesAccess, setCoreHostedHermesAccess,
} from "@/lib/core-client";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { requestOriginMatchesHost } from "@/lib/http-headers";

type RouteContext = { params: Promise<{ runtimeId: string }> };
export const GET = handle;
export const PUT = handle;
export const POST = handle;

async function handle(request: Request, context: RouteContext) {
  const headers = { "cache-control": "no-store" };
  const fail = (status: number) => Response.json(
    { error: "Hosted Hermes access is unavailable. Refresh access and try again." }, { status, headers }
  );
  try {
    if (request.method !== "GET" && !requestOriginMatchesHost(request)) return fail(403);
    const { runtimeId } = await context.params;
    const access = await loadDashboardMachineAccess(runtimeId);
    if (!access || access.machineId !== runtimeId) return fail(404);
    let result: unknown;
    if (request.method === "PUT") {
      const payload = await request.json().catch(() => null);
      if (
        !payload || typeof payload.enabled !== "boolean" ||
        !Number.isSafeInteger(payload.expectedGeneration) || payload.expectedGeneration < 0
      ) return fail(400);
      result = await setCoreHostedHermesAccess(runtimeId, {
        enabled: payload.enabled, expectedGeneration: payload.expectedGeneration,
      }, request.signal);
    } else if (request.method === "POST") {
      result = await createCoreHostedHermesSession(runtimeId, request.signal);
    } else {
      result = await loadCoreHostedHermesAccess(runtimeId, request.signal);
    }
    return Response.json(result, { headers });
  } catch (error) {
    return fail(error instanceof CoreFetchError && [401, 403, 404, 409].includes(error.status)
      ? error.status : 503);
  }
}
