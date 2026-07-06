import { signOut } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { NextRequest, NextResponse } from "next/server";

import {
  workosAuthStatus,
  workosInteractiveAuthRequest,
  workosLogoutReturnTo,
} from "@/lib/workos-auth";

export async function GET(request: NextRequest) {
  const status = workosAuthStatus();

  if (!status.enabled) {
    redirect("/");
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

  if (!workosInteractiveAuthRequest(request.method, request.headers)) {
    return new NextResponse(null, {
      status: 204,
      headers: {
        "Cache-Control": "no-store",
        Vary: "Cookie",
      },
    });
  }

  await signOut({ returnTo: workosLogoutReturnTo() });
}
