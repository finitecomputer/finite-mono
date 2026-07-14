import { NextRequest } from "next/server";

import { brainOpaqueCorsPreflight, proxyBrainRequest } from "@/lib/brain-proxy";

type RouteContext = {
  params: Promise<{ path?: string[] }>;
};

async function proxy(request: NextRequest, context: RouteContext) {
  const { path = [] } = await context.params;
  return proxyBrainRequest(request, "/_admin", path);
}

export const GET = proxy;
export const HEAD = proxy;
export const POST = proxy;
export const PUT = proxy;
export const PATCH = proxy;
export const DELETE = proxy;
export const OPTIONS = brainOpaqueCorsPreflight;
