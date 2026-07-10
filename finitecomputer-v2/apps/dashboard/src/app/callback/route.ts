import { handleAuth } from "@workos-inc/authkit-nextjs";
import { NextRequest, NextResponse } from "next/server";

import { workosAuthStatus, workosBaseUrl } from "@/lib/workos-auth";

export async function GET(request: NextRequest) {
  const status = workosAuthStatus();

  if (!status.ready) {
    return NextResponse.json(
      {
        error: "Sign in is temporarily unavailable",
        missing: status.missing,
      },
      { status: status.enabled ? 503 : 404 },
    );
  }

  return handleAuth({
    baseURL: workosBaseUrl(),
    returnPathname: "/dashboard",
    onError({ error }) {
      console.error("[AuthKit callback error]", error);
      const response = NextResponse.redirect(
        new URL("/login", workosBaseUrl() ?? request.url)
      );
      response.headers.set("Cache-Control", "no-store");
      response.headers.set("Vary", "Cookie");
      return response;
    },
  })(request);
}
