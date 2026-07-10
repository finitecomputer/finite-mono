import { getSignInUrl } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { NextRequest, NextResponse } from "next/server";

import { safeWorkosReturnPathname, workosAuthStatus } from "@/lib/workos-auth";

export async function GET(request: NextRequest) {
  const status = workosAuthStatus();

  if (!status.enabled) {
    return NextResponse.json({ error: "Sign in is unavailable" }, { status: 404 });
  }

  if (!status.ready) {
    return NextResponse.json(
      {
        error: "Sign in is temporarily unavailable",
        missing: status.missing,
      },
      { status: 503 },
    );
  }

  redirect(
    await getSignInUrl({
      returnTo: safeWorkosReturnPathname(request.nextUrl.searchParams.get("returnTo")),
    }),
  );
}
