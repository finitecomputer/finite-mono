import { NextRequest } from "next/server";

import { proxyBrainRequest } from "@/lib/brain-proxy";

export function GET(request: NextRequest) {
  return proxyBrainRequest(request, "/health");
}
