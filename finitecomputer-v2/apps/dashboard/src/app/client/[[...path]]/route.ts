import { NextRequest } from "next/server";

import { proxyBrainRequest } from "@/lib/brain-proxy";

type RouteContext = {
  params: Promise<{ path?: string[] }>;
};

async function proxy(request: NextRequest, context: RouteContext) {
  const { path = [] } = await context.params;
  return proxyBrainRequest(request, "/client", path);
}

export const GET = proxy;
export const HEAD = proxy;
