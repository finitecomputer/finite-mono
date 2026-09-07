import {
  applyResponseHeaders,
  authkit,
  handleAuthkitHeaders,
  partitionAuthkitHeaders,
} from "@workos-inc/authkit-nextjs";
import { NextRequest, NextResponse } from "next/server";

import {
  workosInteractiveAuthRequest,
  workosAuthStatus,
  workosProtectedPath,
  workosProxyBypassPath,
} from "@/lib/workos-auth";

export default async function proxy(request: NextRequest) {
  const status = workosAuthStatus();

  if (!status.enabled) {
    return NextResponse.next();
  }

  if (workosProxyBypassPath(request.nextUrl.pathname)) {
    return NextResponse.next();
  }

  if (!status.ready) {
    return NextResponse.json(
      {
        error: "WorkOS auth is enabled but not configured",
        missing: status.missing,
      },
      { status: 503 },
    );
  }

  const protectedPath = workosProtectedPath(request.nextUrl.pathname);
  if (!protectedPath) {
    return NextResponse.next();
  }

  const { session, headers, authorizationUrl } = await authkit(request, {
    onSessionRefreshError({ error, request }) {
      console.warn("WorkOS session refresh failed", {
        pathname: request.nextUrl.pathname,
        error: error instanceof Error ? error.message : String(error),
      });
    },
  });

  if (!session.user && authorizationUrl) {
    if (workosInteractiveAuthRequest(request.method, request.headers)) {
      return handleAuthkitHeaders(request, headers, { redirect: authorizationUrl });
    }

    const { responseHeaders } = partitionAuthkitHeaders(request, headers);
    return applyResponseHeaders(
      NextResponse.json(
        {
          error: "Authentication required",
          login: "/login",
        },
        { status: 401 }
      ),
      responseHeaders
    );
  }

  return handleAuthkitHeaders(request, headers);
}

export const config = {
  matcher: [
    "/((?!_next/static|_next/image|favicon.ico|favicon.svg|icons|fonts|finite-logo.svg|manifest.webmanifest).*)",
  ],
};
