import { NextRequest } from "next/server";

import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  issueBrainClientCapability,
  officialBrainFrameParentOrigin,
} from "@/lib/brain-identity-provider";
import { proxyBrainRequest } from "@/lib/brain-proxy";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";

type RouteContext = {
  params: Promise<{ path?: string[] }>;
};

async function proxy(request: NextRequest, context: RouteContext) {
  const { path = [] } = await context.params;
  if (request.method === "GET" && path.length === 0) {
    const parentOrigin = officialBrainFrameParentOrigin(request.url, request.headers);
    if (!parentOrigin) {
      return Response.json(
        { error: "Brain opens only inside its dashboard frame." },
        { status: 403, headers: { "cache-control": "no-store" } },
      );
    }
    const account = await getAccountAuthContext();
    const config = hostedDeviceConfig();
    if (!account.workosUserId || !account.emailVerified) {
      return Response.json(
        { error: "Sign in again to open Brain." },
        { status: 401, headers: { "cache-control": "no-store" } },
      );
    }
    if (!config) {
      return Response.json(
        { error: "Brain identity is not available right now." },
        { status: 503, headers: { "cache-control": "no-store" } },
      );
    }
    return proxyBrainRequest(request, "/client", path, {
      clientCapability: issueBrainClientCapability(
        config.apiToken,
        account.workosUserId,
        parentOrigin,
      ),
      parentOrigin,
    });
  }
  return proxyBrainRequest(request, "/client", path);
}

export const GET = proxy;
export const HEAD = proxy;
