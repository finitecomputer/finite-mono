import { NextResponse } from "next/server";

import { buildPwaManifest } from "@/lib/pwa-manifest";

export function GET(request: Request) {
  const url = new URL(request.url);
  const manifest = buildPwaManifest(url.searchParams.get("machine"));

  return new NextResponse(JSON.stringify(manifest), {
    headers: {
      "cache-control": "private, no-store",
      "content-type": "application/manifest+json",
    },
  });
}
